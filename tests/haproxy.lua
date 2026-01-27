local opentelemetry = require("haproxy_otel_module")

opentelemetry.register({
	name = "haproxy",
	otlp = {
		endpoint = "http://localhost:4317",
		protocol = "json",
	},
	sampler = "AlwaysOn",
	propagator = "zipkin",
})
