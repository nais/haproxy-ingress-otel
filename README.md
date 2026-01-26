# haproxy-otel

A Rust-based [OpenTelemetry] module for [HAProxy] Community Edition using the [Lua API].

> **Note:** This is a [NAIS fork](https://github.com/nais/haproxy-otel) of [khvzak/haproxy-otel](https://github.com/khvzak/haproxy-otel) with added Docker packaging for the HAProxy Kubernetes Ingress Controller.

[OpenTelemetry]: https://opentelemetry.io
[HAProxy]: https://www.haproxy.org
[Lua API]: https://www.arpalert.org/src/haproxy-lua-api/3.0/index.html

## Pre-built Docker Image

Multi-arch images (amd64/arm64) are published to GitHub Container Registry:

```bash
docker pull ghcr.io/nais/haproxy-otel:latest
```

### Image Tags

| Tag                 | Description                               |
| ------------------- | ----------------------------------------- |
| `latest`            | Latest build from main branch             |
| `3.2.4-0.2.0`       | HAProxy Ingress 3.2.4 + OTEL module 0.2.0 |
| `3.2.4-0.2.0-<sha>` | Specific commit build                     |

## Using with HAProxy Kubernetes Ingress Controller

### Official Helm Chart

Use this image as a drop-in replacement for the official HAProxy Kubernetes Ingress Controller with the [haproxytech/kubernetes-ingress](https://github.com/haproxytech/helm-charts/tree/main/kubernetes-ingress) Helm chart:

```yaml
# values.yaml
controller:
  image:
    repository: ghcr.io/nais/haproxy-otel
    tag: "3.2.4-0.2.0"

  extraEnvs:
    - name: OTEL_SERVICE_NAME
      value: "haproxy-ingress"
    - name: OTEL_EXPORTER_OTLP_ENDPOINT
      value: "http://otel-collector.observability:4318/v1/traces"
    - name: OTEL_TRACES_SAMPLER
      value: "parentbased_always_on"
    - name: OTEL_PROPAGATORS
      value: "w3c"

  # Required HAProxy config snippets to enable OTEL
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

### Environment Variables

| Variable                      | Description             | Default                          |
| ----------------------------- | ----------------------- | -------------------------------- |
| `OTEL_SERVICE_NAME`           | Service name for traces | `haproxy-ingress`                |
| `OTEL_EXPORTER_OTLP_ENDPOINT` | OTLP collector endpoint | `http://localhost:4318/v1/trace` |
| `OTEL_TRACES_SAMPLER`         | Sampling strategy       | `parentbased_always_on`          |
| `OTEL_PROPAGATORS`            | Propagation format      | `w3c`                            |
| `OTEL_EXPORTER_OTLP_PROTOCOL` | Protocol format         | `json`                           |

### Sampler Values

- `always_on` - Sample all traces
- `always_off` - Sample no traces
- `parentbased_always_on` / `parentbased` - Follow parent span decision (default)

### Propagator Values

- `w3c` / `tracecontext` - W3C Trace Context
- `zipkin` / `b3` / `b3multi` - Zipkin B3
- `jaeger` - Jaeger

## Fork Changes from Upstream

This fork ([nais/haproxy-otel](https://github.com/nais/haproxy-otel)) adds the following to [khvzak/haproxy-otel](https://github.com/khvzak/haproxy-otel):

### Docker Image

- **Multi-stage Dockerfile** building a complete HAProxy Kubernetes Ingress Controller image
- **Base image**: `haproxytech/haproxy-debian:3.2` (glibc required for Rust cdylib)
- **Ingress controller**: Extracted from `haproxytech/kubernetes-ingress:3.2.4`
- **OTEL module**: Pre-compiled and installed at `/usr/local/lib/lua/5.4/haproxy_otel_module.so`
- **Default config**: Environment-based configuration via `/etc/haproxy/lua/otel.lua`
- **musl compatibility**: Installed for Alpine-compiled `haproxy_wrapper` binary

### CI/CD

- **GitHub Actions workflow** with lint, test, and e2e jobs
- **Multi-arch builds** (amd64/arm64) using native runners (no QEMU emulation)
- **Automated publishing** to GitHub Container Registry

### Build Configuration

- **`.cargo/config.toml`**: Added Linux linker flags for `--allow-shlib-undefined`
- **`Cargo.toml`**: Override `reqwest` to use `rustls-tls` + `hickory-dns` (avoids glibc `__res_init` issues)

### E2E Testing

- **Docker Compose test**: Standalone HAProxy with Jaeger trace verification
- **Kind test**: Full Kubernetes Ingress Controller deployment
- **Cross-platform**: Works on macOS ARM64 (Colima) and Linux x86_64 (CI)

[View full diff](https://github.com/khvzak/haproxy-otel/compare/main...nais:haproxy-otel:main)

## Features

- Server-side and client-side span creation
- Custom attributes support
- OTLP exporter
- Support for W3C/Zipkin/Jaeger headers propagation formats

## Usage and Configuration

Please check the [module](module) and [tests](tests) directories for working examples.

### HAProxy Configuration

```haproxy
global
    master-worker
    # This is required to enable non-blocking background activity (exporting traces)
    insecure-fork-wanted
    lua-load-per-thread otel.lua

...

frontend http-in
    bind *:8080
    http-request lua.start_server_span
    filter lua.opentelemetry-trace
    http-request set-var-fmt(txn.custom_attr_value) "hello"
    http-request lua.set_span_attribute_var test_attribute txn.custom_attr_value
    default_backend default

backend default
    server srv1 127.0.0.1:8080
```

Create a file named `otel.lua` with the following content:

```lua
local opentelemetry = require("haproxy_otel_module")

opentelemetry.register({
    name = "loadbalancer",  -- Service name
    sampler = "AlwaysOn",  -- Sampling strategy
    propagator = "w3c",  -- Propagation format
    otlp = {
        endpoint = "http://otel-collector:4317/v1/trace",  -- OTLP endpoint
        protocol = "json",  -- Protocol format (json or binary)
    }
})
```

### Configuration Options

| Option          | Description                      | Values                                       | Default                            |
| --------------- | -------------------------------- | -------------------------------------------- | ---------------------------------- |
| `name`          | Service name for the traces      | String                                       | `"haproxy"`                        |
| `sampler`       | Sampling strategy                | `"AlwaysOn"`, `"AlwaysOff"`, `"ParentBased"` | `"ParentBased"`                    |
| `propagator`    | Trace context propagation format | `"w3c"`, `"zipkin"`, `"jaeger"`              | `"w3c"`                            |
| `otlp.endpoint` | OTLP collector endpoint URL      | URL string                                   | `"http://localhost:4318/v1/trace"` |
| `otlp.protocol` | OTLP protocol format             | `"json"`, `"binary"`                         | `"binary"`                         |

### Tracing

To create a server-side and a client-side (upstream proxying) spans for incoming requests:

```
http-request lua.start_server_span
filter lua.opentelemetry-trace start_client_span=true
```

The `http-request` directive is required to start a new span and `filter` is responsible for (optional) client-side (upstream) span creation and finishing all the spans.
Both directives are needed to create a complete trace for the request.


| Option              | Description                    | Values          | Default |
| ------------------- | ------------------------------ | --------------- | ------- |
| `start_client_span` | Whether to create client spans | `true`, `false` | `true`  |

### Adding Custom Attributes

You can add custom attributes to spans:

```
http-request set-var-fmt(txn.user_id) "%[req.hdr(User-ID)]"
http-request lua.set_span_attribute_var user.id txn.user_id
```

You need to assign a value to a variable first, and then use the `lua.set_span_attribute_var` function to add the attribute to the span.

## Integration with OpenTelemetry Collector

This module sends traces to an OpenTelemetry [collector]. Configure your collector to receive OTLP traces and export them to your preferred backend (Jaeger, Zipkin, etc.).

[collector]: https://opentelemetry.io/docs/collector/

## License

This project is licensed under the [MIT license](LICENSE).
