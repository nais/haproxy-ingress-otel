local opentelemetry = require("haproxy_otel_module")

opentelemetry.register({
	name = "haproxy",
	otlp = {
		endpoint = "http://127.0.0.1:4317",
		protocol = "http/json",
	},
	sampler = "AlwaysOn",
	propagator = "zipkin",
})
