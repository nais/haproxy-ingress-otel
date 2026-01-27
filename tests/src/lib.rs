#![cfg(test)]

use std::net::TcpListener;

use serde_json::{json, Value as JsonValue};
use std::time::Duration;
use tokio::time::timeout;
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockGuard, MockServer, ResponseTemplate};

/// Check if HAProxy has Lua support
fn haproxy_has_lua_support() -> bool {
    let output = std::process::Command::new("haproxy")
        .arg("-vv")
        .output()
        .ok();

    match output {
        Some(out) => {
            let stdout = String::from_utf8_lossy(&out.stdout);
            // Check for +LUA in feature flags (not -LUA which means disabled)
            stdout.contains("+LUA")
        }
        None => false,
    }
}

#[tokio::test]
async fn integration_tests() {
    // Skip test if HAProxy doesn't have Lua support
    if !haproxy_has_lua_support() {
        eprintln!("SKIPPED: HAProxy not found or lacks Lua support.");
        eprintln!("Run e2e tests with Docker instead: ./e2e/e2e.sh");
        return;
    }

    // Compile haproxy-otel-module
    tokio::process::Command::new("cargo")
        .args(&["build", "--release", "-p", "haproxy-otel-module"])
        .current_dir("..")
        .status()
        .await
        .expect("Failed to compile haproxy-otel-module");

    // Start the mock server on port 4317
    let listener = TcpListener::bind("127.0.0.1:4317").unwrap();
    let mock_server = MockServer::builder().listener(listener).start().await;

    // Spawn haproxy and wait
    let mut haproxy = tokio::process::Command::new("haproxy")
        .args(&["-f", "haproxy.cfg"])
        .kill_on_drop(true)
        .spawn()
        .expect("Failed to start haproxy");
    tokio::time::sleep(Duration::from_secs(1)).await;

    // Run the tests
    run_tests(&mock_server).await.expect("Tests failed");

    haproxy.kill().await.expect("Failed to stop haproxy");
}

/// Set up the scoped mock for regular HTTP requests (for testing propagation)
async fn mount_http_mock(server: &MockServer) -> MockGuard {
    Mock::given(method("GET"))
        .and(path("/test"))
        .respond_with(ResponseTemplate::new(200).set_body_string("Hello from test server"))
        .expect(1)
        .mount_as_scoped(&server)
        .await
}

/// Set up the scoped mock for OTLP traces endpoint
async fn mount_otlp_mock(server: &MockServer) -> MockGuard {
    Mock::given(method("POST"))
        .and(path("/v1/trace"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"accepted": true})))
        .expect(1)
        .mount_as_scoped(&server)
        .await
}

async fn run_tests(server: &MockServer) -> Result<(), Box<dyn std::error::Error>> {
    let http_mock = mount_http_mock(server).await;
    let mut otlp_mock = mount_otlp_mock(server).await;

    // Make a request to HAProxy
    let client = reqwest::Client::new();
    let response = client
        .get("http://localhost:8080/test")
        .header("X-Test-Header", "test-value")
        .send()
        .await?;
    assert_eq!(response.status(), 200);

    // Verify b3 headers propagation
    let http_req = (http_mock.received_requests().await)
        .pop()
        .expect("No HTTP test requests were received");
    let trace_headers = http_req
        .headers
        .iter()
        .filter(|(name, _)| name.as_str().starts_with("x-b3"))
        .count();
    assert_eq!(trace_headers, 3, "Expected 3 tracing headers");

    // Verify the received OTLP spans
    timeout(Duration::from_secs(10), otlp_mock.wait_until_satisfied())
        .await
        .unwrap();
    let otlp_request = otlp_mock.received_requests().await.pop().unwrap();
    let spans = otlp_request.body_json::<JsonValue>().unwrap();

    // Get the spans array
    let spans_array = spans
        .pointer("/resourceSpans/0/scopeSpans/0/spans")
        .and_then(|s| s.as_array())
        .expect("Could not find spans array");

    // Verify we have exactly 2 spans
    assert_eq!(spans_array.len(), 2);

    // Find client and server spans
    let client_span = spans_array
        .iter()
        .find(|span| span["kind"].as_i64() == Some(3))
        .expect("Client span (kind=3) not found");
    let server_span = spans_array
        .iter()
        .find(|span| span["kind"].as_i64() == Some(2))
        .expect("Server span (kind=2) not found");

    let attributes = spans
        .pointer("/resourceSpans/0/resource/attributes")
        .unwrap();

    // Verify `service.name`
    let service_name = find_attribute(attributes, "service.name").unwrap();
    assert_eq!(service_name, "haproxy", "Service name should be 'haproxy'");

    // Verify `telemetry.sdk.language`
    let sdk_language = find_attribute(attributes, "telemetry.sdk.language").unwrap();
    assert_eq!(sdk_language, "rust", "SDK language should be 'rust'");

    // Verify span relationship (parent-child)
    assert_eq!(
        client_span["parentSpanId"], server_span["spanId"],
        "Client span's parent ID should match server span's ID"
    );

    // Verify both spans have the same trace ID
    assert_eq!(
        client_span["traceId"], server_span["traceId"],
        "Both spans should have the same trace ID"
    );

    // Verify custom attribute on server span
    let test_attribute = find_attribute(&server_span["attributes"], "test_attribute").unwrap();
    assert_eq!(
        test_attribute, "hello",
        "Server span should have custom attribute 'test_attribute' with value 'hello'"
    );

    // --------

    otlp_mock = mount_otlp_mock(server).await;

    // Make a _local_ request to HAProxy (non-proxied)
    let response = client.get("http://localhost:8080/status").send().await?;
    assert_eq!(response.status(), 200);

    // Verify the received OTLP spans
    timeout(Duration::from_secs(10), otlp_mock.wait_until_satisfied())
        .await
        .unwrap();
    let otlp_request = otlp_mock.received_requests().await.pop().unwrap();
    let spans = otlp_request.body_json::<JsonValue>().unwrap();

    // Get the spans array
    let spans_array = spans
        .pointer("/resourceSpans/0/scopeSpans/0/spans")
        .and_then(|s| s.as_array())
        .expect("Could not find spans array");

    // Verify we have exactly 1 spans
    assert_eq!(spans_array.len(), 1);

    // Check that frontend/backend names are set correctly
    let span = &spans_array[0];
    let frontend = find_attribute(&span["attributes"], "haproxy.frontend.name").unwrap();
    assert_eq!(frontend, "http-in", "Frontend name should be 'http-in'");
    let backend = find_attribute(&span["attributes"], "haproxy.backend.name").unwrap();
    assert_eq!(backend, "status", "Backend name should be 'status'");

    Ok(())
}

fn find_attribute<'a>(attributes: &'a JsonValue, key: &str) -> Option<&'a str> {
    (attributes.as_array()?)
        .iter()
        .find(|attr| attr["key"] == key)
        .and_then(|attr| attr.pointer("/value/stringValue").and_then(|v| v.as_str()))
}
