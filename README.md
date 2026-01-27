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
      value: "http://otel-collector.observability:4318/v1/traces"

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

| Variable                      | Description             | Default                          |
| ----------------------------- | ----------------------- | -------------------------------- |
| `OTEL_SERVICE_NAME`           | Service name for traces | `haproxy-ingress`                |
| `OTEL_EXPORTER_OTLP_ENDPOINT` | OTLP collector endpoint | `http://localhost:4318/v1/trace` |
| `OTEL_TRACES_SAMPLER`         | Sampling strategy       | `parentbased_always_on`          |
| `OTEL_PROPAGATORS`            | Propagation format      | `w3c`                            |
| `OTEL_EXPORTER_OTLP_PROTOCOL` | Protocol format         | `json`                           |

### Sampler Values

| Value                   | Description                 |
| ----------------------- | --------------------------- |
| `always_on`             | Sample all traces           |
| `always_off`            | Sample no traces            |
| `parentbased_always_on` | Follow parent span decision |

### Propagator Values

| Value                 | Format            |
| --------------------- | ----------------- |
| `w3c`, `tracecontext` | W3C Trace Context |
| `zipkin`, `b3`        | Zipkin B3         |
| `jaeger`              | Jaeger            |

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
      value: "http://otel-collector.observability:4318/v1/traces"
    - name: OTEL_TRACES_SAMPLER
      value: "parentbased_always_on"
    - name: OTEL_PROPAGATORS
      value: "w3c"
    - name: OTEL_EXPORTER_OTLP_PROTOCOL
      value: "json"

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
- OTLP HTTP exporter (JSON/protobuf)
- W3C, Zipkin, and Jaeger propagation formats

## What's Included

The Docker image contains:

- HAProxy Kubernetes Ingress Controller (from `haproxytech/kubernetes-ingress`)
- Pre-compiled OTEL Lua module (`/usr/local/lib/lua/5.4/haproxy_otel_module.so`)
- Default OTEL config (`/etc/haproxy/lua/otel.lua`)

## Development

### Building

```bash
docker build -t haproxy-ingress-otel:test .
```

### Testing

```bash
# Docker Compose e2e test
./e2e/e2e.sh

# Kubernetes e2e test (requires kind, helm)
./e2e/kind-e2e.sh
```

See [e2e/README.md](e2e/README.md) for detailed testing instructions.

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
