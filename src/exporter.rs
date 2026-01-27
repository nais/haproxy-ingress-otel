use std::env;
use std::error::Error as StdError;

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
fn resolve_endpoint(options: &Options, protocol: &Protocol) -> String {
    // 1. Check options (Lua config)
    if let Some(ref ep) = options.endpoint {
        if !ep.is_empty() {
            return ep.clone();
        }
    }

    // 2. Check OTEL_EXPORTER_OTLP_TRACES_ENDPOINT (signal-specific, used as-is)
    if let Ok(ep) = env::var("OTEL_EXPORTER_OTLP_TRACES_ENDPOINT") {
        if !ep.is_empty() {
            // Per spec: signal-specific endpoint is used as-is (no /v1/traces appended)
            return ep;
        }
    }

    // 3. Check OTEL_EXPORTER_OTLP_ENDPOINT (base URL)
    if let Ok(ep) = env::var("OTEL_EXPORTER_OTLP_ENDPOINT") {
        if !ep.is_empty() {
            return ep;
        }
    }

    // 4. Default based on protocol
    protocol.default_endpoint().to_string()
}

/// Read protocol from options or OTEL environment variables
fn resolve_protocol(options: &Options) -> Protocol {
    // 1. Check options (Lua config)
    if let Some(ref proto) = options.protocol {
        if let Some(p) = Protocol::from_str(proto) {
            return p;
        }
    }

    // 2. Check OTEL_EXPORTER_OTLP_TRACES_PROTOCOL (signal-specific)
    if let Ok(proto) = env::var("OTEL_EXPORTER_OTLP_TRACES_PROTOCOL") {
        if let Some(p) = Protocol::from_str(&proto) {
            return p;
        }
    }

    // 3. Check OTEL_EXPORTER_OTLP_PROTOCOL (general)
    if let Ok(proto) = env::var("OTEL_EXPORTER_OTLP_PROTOCOL") {
        if let Some(p) = Protocol::from_str(&proto) {
            return p;
        }
    }

    // 4. Default per OTLP spec
    Protocol::default()
}

pub fn init(options: Options) -> Result<(), Box<dyn StdError + Send + Sync + 'static>> {
    // Configure propagator
    match options.propagator.as_deref() {
        None | Some("w3c") => {
            opentelemetry::global::set_text_map_propagator(TraceContextPropagator::new());
        }
        Some("zipkin") => {
            opentelemetry::global::set_text_map_propagator(opentelemetry_zipkin::Propagator::new());
        }
        Some("jaeger") => {
            opentelemetry::global::set_text_map_propagator(opentelemetry_jaeger::Propagator::new());
        }
        _ => {}
    }

    // Resolve protocol and endpoint
    let protocol = resolve_protocol(&options);
    let base_endpoint = resolve_endpoint(&options, &protocol);

    // Build the exporter based on protocol
    let processor = match protocol {
        Protocol::Grpc => {
            let endpoint = build_traces_endpoint(&base_endpoint, &protocol);
            let exporter = opentelemetry_otlp::SpanExporter::builder()
                .with_tonic()
                .with_endpoint(&endpoint)
                .build()?;
            BatchSpanProcessor::builder(exporter, crate::runtime::HaproxyTokio::new()).build()
        }
        Protocol::HttpProtobuf => {
            let endpoint = build_traces_endpoint(&base_endpoint, &protocol);
            let exporter = opentelemetry_otlp::SpanExporter::builder()
                .with_http()
                .with_endpoint(&endpoint)
                .with_protocol(opentelemetry_otlp::Protocol::HttpBinary)
                .build()?;
            BatchSpanProcessor::builder(exporter, crate::runtime::HaproxyTokio::new()).build()
        }
        Protocol::HttpJson => {
            let endpoint = build_traces_endpoint(&base_endpoint, &protocol);
            let exporter = opentelemetry_otlp::SpanExporter::builder()
                .with_http()
                .with_endpoint(&endpoint)
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

    match options.sampler.as_deref() {
        Some("AlwaysOn" | "SilentOn") => {
            tracer_provider_builder = tracer_provider_builder.with_sampler(Sampler::AlwaysOn);
        }
        Some("AlwaysOff") => {
            tracer_provider_builder = tracer_provider_builder.with_sampler(Sampler::AlwaysOff);
        }
        None | Some("ParentBased") => {
            // This is the default sampler
            tracer_provider_builder = tracer_provider_builder
                .with_sampler(Sampler::ParentBased(Box::new(Sampler::AlwaysOn)));
        }
        _ => {}
    }

    opentelemetry::global::set_tracer_provider(tracer_provider_builder.build());

    Ok(())
}
