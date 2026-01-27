use std::env;
use std::error::Error as StdError;
use std::fmt;

use opentelemetry_jaeger_propagator as opentelemetry_jaeger;
use opentelemetry_otlp::WithExportConfig;
use opentelemetry_sdk::propagation::TraceContextPropagator;
use opentelemetry_sdk::trace::span_processor_with_async_runtime::BatchSpanProcessor;
use opentelemetry_sdk::trace::{RandomIdGenerator, Sampler, SdkTracerProvider};
use opentelemetry_sdk::Resource;

/// Default endpoints per OTLP spec
const DEFAULT_HTTP_ENDPOINT: &str = "http://localhost:4318";
const DEFAULT_GRPC_ENDPOINT: &str = "http://localhost:4317";
const TRACES_PATH: &str = "v1/traces";

/// Supported protocols per OTLP spec
#[derive(Debug, Clone, PartialEq, Default)]
pub enum Protocol {
    Grpc,
    #[default]
    HttpProtobuf,
    HttpJson,
}

impl fmt::Display for Protocol {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Protocol::Grpc => write!(f, "grpc"),
            Protocol::HttpProtobuf => write!(f, "http/protobuf"),
            Protocol::HttpJson => write!(f, "http/json"),
        }
    }
}

impl Protocol {
    /// Parse protocol from string (OTEL spec values)
    fn from_str(s: &str) -> Option<Self> {
        match s.to_lowercase().as_str() {
            "grpc" => Some(Protocol::Grpc),
            "http/protobuf" => Some(Protocol::HttpProtobuf),
            "http/json" => Some(Protocol::HttpJson),
            // Legacy values for backwards compatibility
            "binary" => Some(Protocol::HttpProtobuf),
            "json" => Some(Protocol::HttpJson),
            _ => None,
        }
    }

    fn default_endpoint(&self) -> &'static str {
        match self {
            Protocol::Grpc => DEFAULT_GRPC_ENDPOINT,
            Protocol::HttpProtobuf | Protocol::HttpJson => DEFAULT_HTTP_ENDPOINT,
        }
    }
}

/// Construct the traces endpoint URL per OTLP spec.
/// For HTTP: appends /v1/traces to the base endpoint
/// For gRPC: uses endpoint as-is
fn build_traces_endpoint(base: &str, protocol: &Protocol) -> String {
    match protocol {
        Protocol::Grpc => base.to_string(),
        Protocol::HttpProtobuf | Protocol::HttpJson => {
            let base = base.trim_end_matches('/');
            format!("{base}/{TRACES_PATH}")
        }
    }
}

/// Source of configuration value for debugging
#[derive(Debug, Clone, PartialEq)]
pub enum ConfigSource {
    LuaConfig,
    EnvTracesSpecific,
    EnvGeneral,
    Default,
}

impl fmt::Display for ConfigSource {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ConfigSource::LuaConfig => write!(f, "lua config"),
            ConfigSource::EnvTracesSpecific => write!(f, "env (traces-specific)"),
            ConfigSource::EnvGeneral => write!(f, "env"),
            ConfigSource::Default => write!(f, "default"),
        }
    }
}

#[derive(Clone)]
pub(crate) struct Options {
    pub(crate) service_name: String,
    // Can be: "AlwaysOn", "SilentOn", "AlwaysOff", "ParentBased"
    pub(crate) sampler: Option<String>,
    // Can be: "w3c", "jaeger", "zipkin"
    pub(crate) propagator: Option<String>,
    pub(crate) endpoint: Option<String>,
    // Can be: "grpc", "http/protobuf", "http/json" (OTEL spec)
    // Legacy: "binary" or "json"
    pub(crate) protocol: Option<String>,
}

/// Read endpoint from options or OTEL environment variables
/// Returns the endpoint and the source it came from
fn resolve_endpoint(options: &Options, protocol: &Protocol) -> (String, ConfigSource) {
    // 1. Check options (Lua config)
    if let Some(ref ep) = options.endpoint {
        if !ep.is_empty() {
            return (ep.clone(), ConfigSource::LuaConfig);
        }
    }

    // 2. Check OTEL_EXPORTER_OTLP_TRACES_ENDPOINT (signal-specific, used as-is)
    if let Ok(ep) = env::var("OTEL_EXPORTER_OTLP_TRACES_ENDPOINT") {
        if !ep.is_empty() {
            // Per spec: signal-specific endpoint is used as-is (no /v1/traces appended)
            return (ep, ConfigSource::EnvTracesSpecific);
        }
    }

    // 3. Check OTEL_EXPORTER_OTLP_ENDPOINT (base URL)
    if let Ok(ep) = env::var("OTEL_EXPORTER_OTLP_ENDPOINT") {
        if !ep.is_empty() {
            return (ep, ConfigSource::EnvGeneral);
        }
    }

    // 4. Default based on protocol
    (
        protocol.default_endpoint().to_string(),
        ConfigSource::Default,
    )
}

/// Read protocol from options or OTEL environment variables
/// Returns the protocol and the source it came from
fn resolve_protocol(options: &Options) -> (Protocol, ConfigSource) {
    // 1. Check options (Lua config)
    if let Some(ref proto) = options.protocol {
        if let Some(p) = Protocol::from_str(proto) {
            return (p, ConfigSource::LuaConfig);
        }
    }

    // 2. Check OTEL_EXPORTER_OTLP_TRACES_PROTOCOL (signal-specific)
    if let Ok(proto) = env::var("OTEL_EXPORTER_OTLP_TRACES_PROTOCOL") {
        if let Some(p) = Protocol::from_str(&proto) {
            return (p, ConfigSource::EnvTracesSpecific);
        }
    }

    // 3. Check OTEL_EXPORTER_OTLP_PROTOCOL (general)
    if let Ok(proto) = env::var("OTEL_EXPORTER_OTLP_PROTOCOL") {
        if let Some(p) = Protocol::from_str(&proto) {
            return (p, ConfigSource::EnvGeneral);
        }
    }

    // 4. Default per OTLP spec
    (Protocol::default(), ConfigSource::Default)
}

pub fn init(options: Options) -> Result<(), Box<dyn StdError + Send + Sync + 'static>> {
    // Resolve protocol and endpoint first (needed for logging)
    let (protocol, protocol_source) = resolve_protocol(&options);
    let (base_endpoint, endpoint_source) = resolve_endpoint(&options, &protocol);
    let traces_endpoint = build_traces_endpoint(&base_endpoint, &protocol);
    let propagator = options.propagator.as_deref().unwrap_or("w3c");
    let sampler = options.sampler.as_deref().unwrap_or("ParentBased");

    // Log the resolved configuration
    eprintln!(
        "haproxy-otel: service={} protocol={} ({}) endpoint={} ({}) propagator={} sampler={}",
        options.service_name,
        protocol,
        protocol_source,
        traces_endpoint,
        endpoint_source,
        propagator,
        sampler
    );

    // Configure propagator
    match propagator {
        "w3c" => {
            opentelemetry::global::set_text_map_propagator(TraceContextPropagator::new());
        }
        "zipkin" => {
            opentelemetry::global::set_text_map_propagator(opentelemetry_zipkin::Propagator::new());
        }
        "jaeger" => {
            opentelemetry::global::set_text_map_propagator(opentelemetry_jaeger::Propagator::new());
        }
        _ => {
            // Default to w3c for unknown propagators
            opentelemetry::global::set_text_map_propagator(TraceContextPropagator::new());
        }
    }

    // Build the exporter based on protocol
    let processor = match protocol {
        Protocol::Grpc => {
            let exporter = opentelemetry_otlp::SpanExporter::builder()
                .with_tonic()
                .with_endpoint(&traces_endpoint)
                .build()?;
            BatchSpanProcessor::builder(exporter, crate::runtime::HaproxyTokio::new()).build()
        }
        Protocol::HttpProtobuf => {
            let exporter = opentelemetry_otlp::SpanExporter::builder()
                .with_http()
                .with_endpoint(&traces_endpoint)
                .with_protocol(opentelemetry_otlp::Protocol::HttpBinary)
                .build()?;
            BatchSpanProcessor::builder(exporter, crate::runtime::HaproxyTokio::new()).build()
        }
        Protocol::HttpJson => {
            let exporter = opentelemetry_otlp::SpanExporter::builder()
                .with_http()
                .with_endpoint(&traces_endpoint)
                .with_protocol(opentelemetry_otlp::Protocol::HttpJson)
                .build()?;
            BatchSpanProcessor::builder(exporter, crate::runtime::HaproxyTokio::new()).build()
        }
    };

    let mut tracer_provider_builder = SdkTracerProvider::builder()
        .with_span_processor(processor)
        .with_id_generator(RandomIdGenerator::default())
        .with_resource(
            Resource::builder()
                .with_service_name(options.service_name)
                .build(),
        );

    match sampler {
        "AlwaysOn" | "SilentOn" => {
            tracer_provider_builder = tracer_provider_builder.with_sampler(Sampler::AlwaysOn);
        }
        "AlwaysOff" => {
            tracer_provider_builder = tracer_provider_builder.with_sampler(Sampler::AlwaysOff);
        }
        _ => {
            // Default sampler (ParentBased or unknown)
            tracer_provider_builder = tracer_provider_builder
                .with_sampler(Sampler::ParentBased(Box::new(Sampler::AlwaysOn)));
        }
    }

    opentelemetry::global::set_tracer_provider(tracer_provider_builder.build());

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    // Mutex to ensure env var tests don't interfere with each other
    static ENV_LOCK: Mutex<()> = Mutex::new(());

    fn clear_otel_env_vars() {
        env::remove_var("OTEL_EXPORTER_OTLP_ENDPOINT");
        env::remove_var("OTEL_EXPORTER_OTLP_TRACES_ENDPOINT");
        env::remove_var("OTEL_EXPORTER_OTLP_PROTOCOL");
        env::remove_var("OTEL_EXPORTER_OTLP_TRACES_PROTOCOL");
    }

    fn default_options() -> Options {
        Options {
            service_name: "test".to_string(),
            sampler: None,
            propagator: None,
            endpoint: None,
            protocol: None,
        }
    }

    #[test]
    fn test_protocol_from_str_otel_spec_values() {
        assert_eq!(Protocol::from_str("grpc"), Some(Protocol::Grpc));
        assert_eq!(
            Protocol::from_str("http/protobuf"),
            Some(Protocol::HttpProtobuf)
        );
        assert_eq!(Protocol::from_str("http/json"), Some(Protocol::HttpJson));
        // Case insensitive
        assert_eq!(Protocol::from_str("GRPC"), Some(Protocol::Grpc));
        assert_eq!(
            Protocol::from_str("HTTP/PROTOBUF"),
            Some(Protocol::HttpProtobuf)
        );
        assert_eq!(Protocol::from_str("HTTP/JSON"), Some(Protocol::HttpJson));
    }

    #[test]
    fn test_protocol_from_str_legacy_values() {
        assert_eq!(Protocol::from_str("binary"), Some(Protocol::HttpProtobuf));
        assert_eq!(Protocol::from_str("json"), Some(Protocol::HttpJson));
        assert_eq!(Protocol::from_str("BINARY"), Some(Protocol::HttpProtobuf));
        assert_eq!(Protocol::from_str("JSON"), Some(Protocol::HttpJson));
    }

    #[test]
    fn test_protocol_from_str_invalid() {
        assert_eq!(Protocol::from_str("invalid"), None);
        assert_eq!(Protocol::from_str(""), None);
        assert_eq!(Protocol::from_str("http"), None);
    }

    #[test]
    fn test_protocol_default() {
        assert_eq!(Protocol::default(), Protocol::HttpProtobuf);
    }

    #[test]
    fn test_protocol_display() {
        assert_eq!(format!("{}", Protocol::Grpc), "grpc");
        assert_eq!(format!("{}", Protocol::HttpProtobuf), "http/protobuf");
        assert_eq!(format!("{}", Protocol::HttpJson), "http/json");
    }

    #[test]
    fn test_protocol_default_endpoint() {
        assert_eq!(Protocol::Grpc.default_endpoint(), "http://localhost:4317");
        assert_eq!(
            Protocol::HttpProtobuf.default_endpoint(),
            "http://localhost:4318"
        );
        assert_eq!(
            Protocol::HttpJson.default_endpoint(),
            "http://localhost:4318"
        );
    }

    #[test]
    fn test_build_traces_endpoint_grpc() {
        // gRPC endpoints are used as-is
        assert_eq!(
            build_traces_endpoint("http://collector:4317", &Protocol::Grpc),
            "http://collector:4317"
        );
        assert_eq!(
            build_traces_endpoint("http://collector:4317/", &Protocol::Grpc),
            "http://collector:4317/"
        );
    }

    #[test]
    fn test_build_traces_endpoint_http() {
        // HTTP endpoints get /v1/traces appended
        assert_eq!(
            build_traces_endpoint("http://collector:4318", &Protocol::HttpProtobuf),
            "http://collector:4318/v1/traces"
        );
        assert_eq!(
            build_traces_endpoint("http://collector:4318/", &Protocol::HttpProtobuf),
            "http://collector:4318/v1/traces"
        );
        assert_eq!(
            build_traces_endpoint("http://collector:4318", &Protocol::HttpJson),
            "http://collector:4318/v1/traces"
        );
        // With custom path
        assert_eq!(
            build_traces_endpoint("http://collector:4318/custom", &Protocol::HttpProtobuf),
            "http://collector:4318/custom/v1/traces"
        );
    }

    #[test]
    fn test_resolve_protocol_lua_config_priority() {
        let _lock = ENV_LOCK.lock().unwrap();
        clear_otel_env_vars();

        // Lua config takes priority over env vars
        env::set_var("OTEL_EXPORTER_OTLP_PROTOCOL", "grpc");
        let options = Options {
            protocol: Some("http/json".to_string()),
            ..default_options()
        };
        let (protocol, source) = resolve_protocol(&options);
        assert_eq!(protocol, Protocol::HttpJson);
        assert_eq!(source, ConfigSource::LuaConfig);

        clear_otel_env_vars();
    }

    #[test]
    fn test_resolve_protocol_env_traces_specific_priority() {
        let _lock = ENV_LOCK.lock().unwrap();
        clear_otel_env_vars();

        // Traces-specific env var takes priority over general
        env::set_var("OTEL_EXPORTER_OTLP_PROTOCOL", "http/json");
        env::set_var("OTEL_EXPORTER_OTLP_TRACES_PROTOCOL", "grpc");
        let (protocol, source) = resolve_protocol(&default_options());
        assert_eq!(protocol, Protocol::Grpc);
        assert_eq!(source, ConfigSource::EnvTracesSpecific);

        clear_otel_env_vars();
    }

    #[test]
    fn test_resolve_protocol_env_general() {
        let _lock = ENV_LOCK.lock().unwrap();
        clear_otel_env_vars();

        env::set_var("OTEL_EXPORTER_OTLP_PROTOCOL", "grpc");
        let (protocol, source) = resolve_protocol(&default_options());
        assert_eq!(protocol, Protocol::Grpc);
        assert_eq!(source, ConfigSource::EnvGeneral);

        clear_otel_env_vars();
    }

    #[test]
    fn test_resolve_protocol_default() {
        let _lock = ENV_LOCK.lock().unwrap();
        clear_otel_env_vars();

        let (protocol, source) = resolve_protocol(&default_options());
        assert_eq!(protocol, Protocol::HttpProtobuf);
        assert_eq!(source, ConfigSource::Default);
    }

    #[test]
    fn test_resolve_endpoint_lua_config_priority() {
        let _lock = ENV_LOCK.lock().unwrap();
        clear_otel_env_vars();

        env::set_var("OTEL_EXPORTER_OTLP_ENDPOINT", "http://env:4318");
        let options = Options {
            endpoint: Some("http://lua:4318".to_string()),
            ..default_options()
        };
        let (endpoint, source) = resolve_endpoint(&options, &Protocol::HttpProtobuf);
        assert_eq!(endpoint, "http://lua:4318");
        assert_eq!(source, ConfigSource::LuaConfig);

        clear_otel_env_vars();
    }

    #[test]
    fn test_resolve_endpoint_env_traces_specific_priority() {
        let _lock = ENV_LOCK.lock().unwrap();
        clear_otel_env_vars();

        env::set_var("OTEL_EXPORTER_OTLP_ENDPOINT", "http://general:4318");
        env::set_var("OTEL_EXPORTER_OTLP_TRACES_ENDPOINT", "http://traces:4318");
        let (endpoint, source) = resolve_endpoint(&default_options(), &Protocol::HttpProtobuf);
        assert_eq!(endpoint, "http://traces:4318");
        assert_eq!(source, ConfigSource::EnvTracesSpecific);

        clear_otel_env_vars();
    }

    #[test]
    fn test_resolve_endpoint_env_general() {
        let _lock = ENV_LOCK.lock().unwrap();
        clear_otel_env_vars();

        env::set_var("OTEL_EXPORTER_OTLP_ENDPOINT", "http://general:4318");
        let (endpoint, source) = resolve_endpoint(&default_options(), &Protocol::HttpProtobuf);
        assert_eq!(endpoint, "http://general:4318");
        assert_eq!(source, ConfigSource::EnvGeneral);

        clear_otel_env_vars();
    }

    #[test]
    fn test_resolve_endpoint_default_http() {
        let _lock = ENV_LOCK.lock().unwrap();
        clear_otel_env_vars();

        let (endpoint, source) = resolve_endpoint(&default_options(), &Protocol::HttpProtobuf);
        assert_eq!(endpoint, "http://localhost:4318");
        assert_eq!(source, ConfigSource::Default);
    }

    #[test]
    fn test_resolve_endpoint_default_grpc() {
        let _lock = ENV_LOCK.lock().unwrap();
        clear_otel_env_vars();

        let (endpoint, source) = resolve_endpoint(&default_options(), &Protocol::Grpc);
        assert_eq!(endpoint, "http://localhost:4317");
        assert_eq!(source, ConfigSource::Default);
    }

    #[test]
    fn test_resolve_endpoint_empty_string_ignored() {
        let _lock = ENV_LOCK.lock().unwrap();
        clear_otel_env_vars();

        // Empty strings should be treated as unset
        env::set_var("OTEL_EXPORTER_OTLP_ENDPOINT", "");
        let options = Options {
            endpoint: Some("".to_string()),
            ..default_options()
        };
        let (endpoint, source) = resolve_endpoint(&options, &Protocol::HttpProtobuf);
        assert_eq!(endpoint, "http://localhost:4318");
        assert_eq!(source, ConfigSource::Default);

        clear_otel_env_vars();
    }

    #[test]
    fn test_config_source_display() {
        assert_eq!(format!("{}", ConfigSource::LuaConfig), "lua config");
        assert_eq!(
            format!("{}", ConfigSource::EnvTracesSpecific),
            "env (traces-specific)"
        );
        assert_eq!(format!("{}", ConfigSource::EnvGeneral), "env");
        assert_eq!(format!("{}", ConfigSource::Default), "default");
    }
}
