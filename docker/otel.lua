-- HAProxy OpenTelemetry Configuration
-- This file is loaded per-thread and configures the OTEL module
--
-- Environment variables can be used to configure the module:
--   OTEL_SERVICE_NAME: Service name for traces (default: "haproxy-ingress")
--   OTEL_EXPORTER_OTLP_ENDPOINT: OTLP collector endpoint (default: "http://localhost:4318/v1/trace")
--   OTEL_TRACES_SAMPLER: Sampler strategy (default: "ParentBased")
--   OTEL_PROPAGATORS: Propagation format (default: "w3c")
--   OTEL_EXPORTER_OTLP_PROTOCOL: Protocol format (default: "json")

local opentelemetry = require("haproxy_otel_module")

-- Get configuration from environment variables with defaults
local service_name = os.getenv("OTEL_SERVICE_NAME") or "haproxy-ingress"
local endpoint = os.getenv("OTEL_EXPORTER_OTLP_ENDPOINT") or "http://localhost:4318/v1/trace"
local sampler = os.getenv("OTEL_TRACES_SAMPLER") or "ParentBased"
local propagator = os.getenv("OTEL_PROPAGATORS") or "w3c"
local protocol = os.getenv("OTEL_EXPORTER_OTLP_PROTOCOL") or "json"

-- Normalize sampler names
local sampler_map = {
    ["always_on"] = "AlwaysOn",
    ["always_off"] = "AlwaysOff",
    ["parentbased_always_on"] = "ParentBased",
    ["parentbased"] = "ParentBased",
}
sampler = sampler_map[string.lower(sampler)] or sampler

-- Normalize propagator names
local propagator_map = {
    ["tracecontext"] = "w3c",
    ["b3"] = "zipkin",
    ["b3multi"] = "zipkin",
    ["jaeger"] = "jaeger",
}
propagator = propagator_map[string.lower(propagator)] or propagator

opentelemetry.register({
    name = service_name,
    sampler = sampler,
    propagator = propagator,
    otlp = {
        endpoint = endpoint,
        protocol = protocol,
    },
})

core.Info("OpenTelemetry initialized: service=" .. service_name ..
          " endpoint=" .. endpoint ..
          " sampler=" .. sampler ..
          " propagator=" .. propagator)
