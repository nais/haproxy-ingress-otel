# haproxy-ingress-otel

OpenTelemetry tracing for the [HAProxy Kubernetes Ingress Controller](https://github.com/haproxytech/kubernetes-ingress).

This is a drop-in replacement image for [haproxytech/kubernetes-ingress](https://hub.docker.com/r/haproxytech/kubernetes-ingress) with pre-compiled OpenTelemetry tracing support.

> **Fork**: Based on [khvzak/haproxy-otel](https://github.com/khvzak/haproxy-otel) with Docker packaging for Kubernetes.

## Quick Start

Use this image with the official [haproxytech/kubernetes-ingress](https://github.com/haproxytech/helm-charts/tree/main/kubernetes-ingress) Helm chart:

```yaml
# values.yaml
controller:
  image:
    repository: ghcr.io/nais/haproxy-ingress-otel
    tag: "latest"  # or pin to specific version like "3.2.4-0.2.0"

  extraEnvs:
    - name: OTEL_SERVICE_NAME
      value: "haproxy-ingress"
    - name: OTEL_EXPORTER_OTLP_ENDPOINT
      value: "http://otel-collector.observability:4318"

  config:
    global-config-snippet: |
      tune.lua.bool-sample-conversion pre-3.1-bug
      insecure-fork-wanted
      lua-load-per-thread /etc/haproxy/lua/otel.lua
    frontend-config-snippet: |
      http-request lua.start_server_span
      filter lua.opentelemetry-trace
```

```bash
helm repo add haproxytech https://haproxytech.github.io/helm-charts
helm install haproxy-ingress haproxytech/kubernetes-ingress -f values.yaml
```

## Docker Image

Multi-arch images (amd64/arm64) are published to GitHub Container Registry:

```bash
docker pull ghcr.io/nais/haproxy-ingress-otel:latest
```

### Tags

| Tag Pattern              | Example               | Description                            |
| ------------------------ | --------------------- | -------------------------------------- |
| `latest`                 | `latest`              | Latest build from main branch          |
| `<haproxy>-<otel>`       | `3.2.4-0.2.0`         | HAProxy Ingress version + OTEL version |
| `<haproxy>-<otel>-<sha>` | `3.2.4-0.2.0-abc1234` | Specific commit build                  |

## Configuration

### Environment Variables

All environment variable names follow the [OTLP specification](https://opentelemetry.io/docs/specs/otel/protocol/exporter/).

| Variable                             | Description                           | Default                       |
| ------------------------------------ | ------------------------------------- | ----------------------------- |
| `OTEL_SERVICE_NAME`                  | Service name for traces               | `haproxy-ingress`             |
| `OTEL_EXPORTER_OTLP_ENDPOINT`        | Base OTLP collector endpoint          | `:4318` (HTTP) `:4317` (gRPC) |
| `OTEL_EXPORTER_OTLP_TRACES_ENDPOINT` | Signal-specific endpoint (used as-is) | -                             |
| `OTEL_EXPORTER_OTLP_PROTOCOL`        | Transport protocol                    | `http/protobuf`               |
| `OTEL_EXPORTER_OTLP_TRACES_PROTOCOL` | Signal-specific protocol override     | -                             |
| `OTEL_TRACES_SAMPLER`                | Sampling strategy                     | `parentbased_always_on`       |
| `OTEL_PROPAGATORS`                   | Propagation format                    | `w3c`                         |
| `OTEL_LOG_LEVEL`                     | SDK logging verbosity                 | `info`                        |

**Endpoint behavior:**

- For `OTEL_EXPORTER_OTLP_ENDPOINT`: The `/v1/traces` path is automatically appended for HTTP protocols
- For `OTEL_EXPORTER_OTLP_TRACES_ENDPOINT`: Used as-is without modification

### Sampler Values

| Value                   | Description                 |
| ----------------------- | --------------------------- |
| `always_on`             | Sample all traces           |
| `always_off`            | Sample no traces            |
| `parentbased_always_on` | Follow parent span decision |

### Protocol Values

| Value           | Description                           | Default Port |
| --------------- | ------------------------------------- | ------------ |
| `grpc`          | gRPC with protobuf encoding           | 4317         |
| `http/protobuf` | HTTP with protobuf encoding (default) | 4318         |
| `http/json`     | HTTP with JSON encoding               | 4318         |

### Propagator Values

| Value                 | Format            |
| --------------------- | ----------------- |
| `w3c`, `tracecontext` | W3C Trace Context |
| `zipkin`, `b3`        | Zipkin B3         |
| `jaeger`              | Jaeger            |

### Log Levels

| Value   | Description                        |
| ------- | ---------------------------------- |
| `off`   | Disable all logging                |
| `error` | Only errors                        |
| `warn`  | Errors and warnings                |
| `info`  | Configuration at startup (default) |
| `debug` | Verbose debugging output           |

## Debugging

### Startup Verification

At startup, the module logs its resolved configuration to stderr. Look for a line like:

```text
haproxy-otel: service=haproxy-ingress protocol=http/protobuf (default) endpoint=http://collector:4318/v1/traces (env) propagator=w3c sampler=ParentBased log_level=info (default)
```

This shows:

- `service`: The service name used in traces
- `protocol`: Transport protocol and its source
- `endpoint`: The final endpoint URL and its source
- `propagator`: Context propagation format
- `sampler`: Sampling strategy
- `log_level`: Current logging verbosity

Configuration sources are shown in parentheses: `lua config`, `env (traces-specific)`, `env`, or `default`.

### Common Issues

**No traces appearing:**

1. Verify the Lua module is loaded - check HAProxy logs for the startup message
2. Verify `lua-load-per-thread` directive is in your HAProxy config
3. Check endpoint connectivity: `curl -v http://collector:4318/v1/traces`
4. Verify the frontend snippets are applied (check HAProxy config dump)

**Traces show 00000000... for trace IDs:**

The trace context isn't being propagated. Check:

- The filter is applied: `filter lua.opentelemetry-trace`
- The span is started: `http-request lua.start_server_span`
- The propagator matches your upstream (`w3c`, `jaeger`, `b3`)

**Module not loading:**

Verify HAProxy can find the shared library:

```bash
# Inside the container
ls -la /usr/local/lib/lua/5.4/haproxy_otel_module.so
haproxy -c -f /etc/haproxy/haproxy.cfg
```

### Log Level Configuration

Set `OTEL_LOG_LEVEL=debug` to see verbose output:

```yaml
extraEnvs:
  - name: OTEL_LOG_LEVEL
    value: "debug"
```

Or silence all module output:

```yaml
extraEnvs:
  - name: OTEL_LOG_LEVEL
    value: "off"
```

## HAProxy Config Snippets

The required config snippets enable the OTEL module in HAProxy:

```yaml
# In Helm values.yaml
controller:
  config:
    global-config-snippet: |
      tune.lua.bool-sample-conversion pre-3.1-bug
      insecure-fork-wanted
      lua-load-per-thread /etc/haproxy/lua/otel.lua
    frontend-config-snippet: |
      http-request lua.start_server_span
      filter lua.opentelemetry-trace
```

### Filter Options

Disable client (upstream) span creation:

```haproxy
filter lua.opentelemetry-trace start_client_span=false
```

### Custom Span Attributes

Add custom attributes to spans:

```haproxy
http-request set-var-fmt(txn.user_id) "%[req.hdr(User-ID)]"
http-request lua.set_span_attribute_var user.id txn.user_id
```

### Access Log with Trace Context

The module exposes trace and span IDs as HAProxy transaction variables for use in access logs:

| Variable            | Description               |
| ------------------- | ------------------------- |
| `txn.otel_trace_id` | 32-character hex trace ID |
| `txn.otel_span_id`  | 16-character hex span ID  |

Example log format configuration:

```yaml
controller:
  config:
    defaults-config-snippet: |
      log-format "%ci:%cp [%tr] %ft %b/%s %TR/%Tw/%Tc/%Tr/%Ta %ST %B %tsc %ac/%fc/%bc/%sc/%rc %sq/%bq %hr %hs %{+Q}r trace_id=%[var(txn.otel_trace_id)] span_id=%[var(txn.otel_span_id)]"
```

Or directly in HAProxy config:

```haproxy
defaults
    log-format "%ci:%cp [%tr] %ft %b/%s %ST %B %{+Q}r trace_id=%[var(txn.otel_trace_id)] span_id=%[var(txn.otel_span_id)]"
```

## Complete Helm Values Example

```yaml
controller:
  image:
    repository: ghcr.io/nais/haproxy-ingress-otel
    tag: "latest"  # or pin to specific version

  # OTEL configuration via environment
  extraEnvs:
    - name: OTEL_SERVICE_NAME
      value: "haproxy-ingress"
    - name: OTEL_EXPORTER_OTLP_ENDPOINT
      value: "http://otel-collector.observability:4318"
    - name: OTEL_TRACES_SAMPLER
      value: "parentbased_always_on"
    - name: OTEL_PROPAGATORS
      value: "w3c"
    - name: OTEL_EXPORTER_OTLP_PROTOCOL
      value: "http/protobuf"  # or "grpc" for gRPC transport

  # HAProxy config to enable OTEL
  config:
    global-config-snippet: |
      tune.lua.bool-sample-conversion pre-3.1-bug
      insecure-fork-wanted
      lua-load-per-thread /etc/haproxy/lua/otel.lua
    frontend-config-snippet: |
      http-request lua.start_server_span
      filter lua.opentelemetry-trace

  service:
    type: LoadBalancer
```

## Features

- Server-side span for incoming requests
- Client-side span for upstream proxying
- Custom attributes support
- OTLP exporter with gRPC and HTTP support (protobuf/JSON)
- W3C, Zipkin, and Jaeger propagation formats
- Full OTLP specification compliance for environment variables

## What's Included

The Docker image contains:

- HAProxy Kubernetes Ingress Controller (from `haproxytech/kubernetes-ingress`)
- Pre-compiled OTEL Lua module (`/usr/local/lib/lua/5.4/haproxy_otel_module.so`)
- Default OTEL config (`/etc/haproxy/lua/otel.lua`)

## Contributing

See [CONTRIBUTING.md](CONTRIBUTING.md) for development setup, testing, and version management.

## Version Policy

This image tracks the latest stable branch of HAProxy Kubernetes Ingress Controller.

All versions are defined in [`versions.env`](versions.env). A GitHub workflow checks weekly for upstream updates and creates PRs automatically.

| Component       | Source                                                                              |
| --------------- | ----------------------------------------------------------------------------------- |
| HAProxy Ingress | [haproxytech/kubernetes-ingress](https://github.com/haproxytech/kubernetes-ingress) |
| s6-overlay      | [just-containers/s6-overlay](https://github.com/just-containers/s6-overlay)         |

## Upstream

This project builds on:

- [khvzak/haproxy-otel](https://github.com/khvzak/haproxy-otel) - Rust OTEL module for HAProxy
- [haproxytech/kubernetes-ingress](https://github.com/haproxytech/kubernetes-ingress) - HAProxy Ingress Controller

## License

[MIT](LICENSE)
