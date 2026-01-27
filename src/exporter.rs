use std::env;
use std::error::Error as StdError;
use std::fmt;
use std::sync::atomic::{AtomicU8, Ordering};

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

/// Global log level for OTEL SDK messages (OTEL_LOG_LEVEL)
/// Values: 0=off, 1=error, 2=warn, 3=info, 4=debug
static LOG_LEVEL: AtomicU8 = AtomicU8::new(3); // Default: info

/// Log levels per OTEL spec (OTEL_LOG_LEVEL)
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Default)]
pub enum LogLevel {
    Off = 0,
    Error = 1,
    Warn = 2,
    #[default]
    Info = 3,
    Debug = 4,
}

impl fmt::Display for LogLevel {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            LogLevel::Off => write!(f, "off"),
            LogLevel::Error => write!(f, "error"),
            LogLevel::Warn => write!(f, "warn"),
            LogLevel::Info => write!(f, "info"),
            LogLevel::Debug => write!(f, "debug"),
        }
    }
}

impl LogLevel {
    /// Parse log level from string (case-insensitive per OTEL spec)
    fn from_str(s: &str) -> Option<Self> {
        match s.to_lowercase().as_str() {
            "off" | "none" => Some(LogLevel::Off),
            "error" | "fatal" => Some(LogLevel::Error),
            "warn" | "warning" => Some(LogLevel::Warn),
            "info" => Some(LogLevel::Info),
            "debug" | "trace" => Some(LogLevel::Debug),
            _ => None,
        }
    }
}

/// Read OTEL_LOG_LEVEL from environment
fn resolve_log_level() -> (LogLevel, ConfigSource) {
    if let Ok(level) = env::var("OTEL_LOG_LEVEL") {
        if let Some(l) = LogLevel::from_str(&level) {
            return (l, ConfigSource::EnvGeneral);
        }
        // Per spec: warn on unrecognized value, fall back to default
        eprintln!(
            "haproxy-otel: warning: unrecognized OTEL_LOG_LEVEL='{}', using 'info'",
            level
        );
    }
    (LogLevel::default(), ConfigSource::Default)
}

/// Log at error level
#[allow(dead_code)]
#[inline]
fn log_error(msg: &str) {
    if LOG_LEVEL.load(Ordering::Relaxed) >= LogLevel::Error as u8 {
        eprintln!("haproxy-otel error: {}", msg);
    }
}

/// Log at warn level
#[allow(dead_code)]
#[inline]
fn log_warn(msg: &str) {
    if LOG_LEVEL.load(Ordering::Relaxed) >= LogLevel::Warn as u8 {
        eprintln!("haproxy-otel warn: {}", msg);
    }
}

/// Log at info level
#[inline]
fn log_info(msg: &str) {
    if LOG_LEVEL.load(Ordering::Relaxed) >= LogLevel::Info as u8 {
        eprintln!("haproxy-otel: {}", msg);
    }
}

/// Log at debug level
#[allow(dead_code)]
#[inline]
fn log_debug(msg: &str) {
    if LOG_LEVEL.load(Ordering::Relaxed) >= LogLevel::Debug as u8 {
        eprintln!("haproxy-otel debug: {}", msg);
    }
}

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
    // Resolve log level first (affects all subsequent logging)
    let (log_level, log_level_source) = resolve_log_level();
    LOG_LEVEL.store(log_level as u8, Ordering::Relaxed);

    // Resolve protocol and endpoint first (needed for logging)
    let (protocol, protocol_source) = resolve_protocol(&options);
    let (base_endpoint, endpoint_source) = resolve_endpoint(&options, &protocol);
    let traces_endpoint = build_traces_endpoint(&base_endpoint, &protocol);
    let propagator = options.propagator.as_deref().unwrap_or("w3c");
    let sampler = options.sampler.as_deref().unwrap_or("ParentBased");

    // Log the resolved configuration
    log_info(&format!(
        "service={} protocol={} ({}) endpoint={} ({}) propagator={} sampler={} log_level={} ({})",
        options.service_name,
        protocol,
        protocol_source,
        traces_endpoint,
        endpoint_source,
        propagator,
        sampler,
        log_level,
        log_level_source
    ));

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
    // gRPC requires Tokio runtime context during builder execution
    let processor = match protocol {
        Protocol::Grpc => {
            // Enter the HAProxy Tokio runtime for gRPC client initialization
            // Tonic/hyper requires an active runtime during connection setup
            let _guard = haproxy_api::runtime().enter();
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

    #[test]
    fn test_log_level_default() {
        assert_eq!(LogLevel::default(), LogLevel::Info);
    }

    #[test]
    fn test_log_level_display() {
        assert_eq!(format!("{}", LogLevel::Off), "off");
        assert_eq!(format!("{}", LogLevel::Error), "error");
        assert_eq!(format!("{}", LogLevel::Warn), "warn");
        assert_eq!(format!("{}", LogLevel::Info), "info");
        assert_eq!(format!("{}", LogLevel::Debug), "debug");
    }

    #[test]
    fn test_log_level_from_str() {
        // Standard values
        assert_eq!(LogLevel::from_str("off"), Some(LogLevel::Off));
        assert_eq!(LogLevel::from_str("error"), Some(LogLevel::Error));
        assert_eq!(LogLevel::from_str("warn"), Some(LogLevel::Warn));
        assert_eq!(LogLevel::from_str("info"), Some(LogLevel::Info));
        assert_eq!(LogLevel::from_str("debug"), Some(LogLevel::Debug));

        // Case insensitive
        assert_eq!(LogLevel::from_str("DEBUG"), Some(LogLevel::Debug));
        assert_eq!(LogLevel::from_str("INFO"), Some(LogLevel::Info));
        assert_eq!(LogLevel::from_str("Error"), Some(LogLevel::Error));

        // Aliases
        assert_eq!(LogLevel::from_str("none"), Some(LogLevel::Off));
        assert_eq!(LogLevel::from_str("fatal"), Some(LogLevel::Error));
        assert_eq!(LogLevel::from_str("warning"), Some(LogLevel::Warn));
        assert_eq!(LogLevel::from_str("trace"), Some(LogLevel::Debug));

        // Invalid
        assert_eq!(LogLevel::from_str("invalid"), None);
        assert_eq!(LogLevel::from_str(""), None);
    }

    #[test]
    fn test_log_level_ordering() {
        // Verify log levels are ordered correctly for comparison
        assert!(LogLevel::Off < LogLevel::Error);
        assert!(LogLevel::Error < LogLevel::Warn);
        assert!(LogLevel::Warn < LogLevel::Info);
        assert!(LogLevel::Info < LogLevel::Debug);
    }

    #[test]
    fn test_resolve_log_level_default() {
        let _lock = ENV_LOCK.lock().unwrap();
        env::remove_var("OTEL_LOG_LEVEL");

        let (level, source) = resolve_log_level();
        assert_eq!(level, LogLevel::Info);
        assert_eq!(source, ConfigSource::Default);
    }

    #[test]
    fn test_resolve_log_level_from_env() {
        let _lock = ENV_LOCK.lock().unwrap();

        env::set_var("OTEL_LOG_LEVEL", "debug");
        let (level, source) = resolve_log_level();
        assert_eq!(level, LogLevel::Debug);
        assert_eq!(source, ConfigSource::EnvGeneral);

        env::set_var("OTEL_LOG_LEVEL", "error");
        let (level, source) = resolve_log_level();
        assert_eq!(level, LogLevel::Error);
        assert_eq!(source, ConfigSource::EnvGeneral);

        env::remove_var("OTEL_LOG_LEVEL");
    }

    #[test]
    fn test_resolve_log_level_invalid_falls_back() {
        let _lock = ENV_LOCK.lock().unwrap();

        env::set_var("OTEL_LOG_LEVEL", "invalid_value");
        let (level, source) = resolve_log_level();
        // Falls back to default on invalid value (per spec, logs warning)
        assert_eq!(level, LogLevel::Info);
        assert_eq!(source, ConfigSource::Default);

        env::remove_var("OTEL_LOG_LEVEL");
    }
}
