# =============================================================================
# Version Configuration
# CI sources these from versions.env. Defaults here for local builds.
# Run 'mise run sync-versions' when updating versions.env.
# =============================================================================
ARG RUST_VERSION=1.97
ARG HAPROXY_INGRESS_VERSION=3.2.15
ARG HAPROXY_VERSION=3.2

# =============================================================================
# Build stage: Rust OTEL module (glibc - cdylib doesn't support musl)
# =============================================================================
FROM rust:${RUST_VERSION}-bookworm AS rust-builder

WORKDIR /build

# Install build dependencies (no openssl needed - using rustls)
RUN apt-get update && apt-get upgrade -y && apt-get install -y --no-install-recommends \
    pkg-config \
    && rm -rf /var/lib/apt/lists/*

# Copy workspace files
COPY Cargo.toml ./
COPY .cargo ./.cargo
COPY src ./src
COPY module ./module

# Remove tests from workspace members (tests requires edition2024/Rust 1.87+)
RUN sed -i 's/, "tests"//' Cargo.toml && sed -i 's/"tests", //' Cargo.toml

# Build the module in release mode
RUN cargo fetch && cargo build --release -p haproxy-otel-module

# =============================================================================
# Source stage: Extract binaries from official HAProxy Ingress Controller
# =============================================================================
ARG HAPROXY_INGRESS_VERSION
FROM docker.io/haproxytech/kubernetes-ingress:${HAPROXY_INGRESS_VERSION} AS ingress-source

# =============================================================================
# Final stage: Debian-based HAProxy with OTEL module and gopherd supervisor
# =============================================================================
ARG HAPROXY_VERSION
ARG HAPROXY_INGRESS_VERSION
FROM haproxytech/haproxy-debian:${HAPROXY_VERSION}

ARG HAPROXY_VERSION
ARG HAPROXY_INGRESS_VERSION

USER root

# Install runtime dependencies
RUN apt-get update && apt-get upgrade -y && apt-get install -y --no-install-recommends \
    socat \
    openssl \
    htop \
    tzdata \
    curl \
    libcap2-bin \
    ca-certificates \
    musl \
    && case "$(dpkg --print-architecture)" in \
        amd64) MUSL_ARCH=x86_64 ;; \
        arm64) MUSL_ARCH=aarch64 ;; \
        armhf) MUSL_ARCH=armhf ;; \
        *) echo "Unsupported musl architecture: $(dpkg --print-architecture)" >&2; exit 1 ;; \
    esac \
    && ln -sf "/lib/ld-musl-${MUSL_ARCH}.so.1" "/lib/libc.musl-${MUSL_ARCH}.so.1" \
    && rm -rf /var/lib/apt/lists/*

# Copy the 3.2.15 ingress runtime and its gopherd supervisor configuration.
COPY --from=ingress-source /haproxy-ingress-controller /haproxy-ingress-controller
COPY --from=ingress-source /usr/local/sbin/haproxy_wrapper /usr/local/sbin/haproxy_wrapper
COPY --from=ingress-source /usr/local/sbin/gopherd /usr/local/sbin/gopherd
COPY --from=ingress-source /usr/local/lib/libblock_secrets.so /usr/local/lib/libblock_secrets.so
COPY --from=ingress-source /etc/gopherd/gopherd.yml /etc/gopherd/gopherd.yml
COPY --from=ingress-source /etc/haproxy/haproxy.cfg /etc/haproxy/haproxy.cfg
COPY --from=ingress-source /etc/haproxy/errors /etc/haproxy/errors

# Create Lua module directory and copy OTEL module
RUN mkdir -p /usr/local/lib/lua/5.4 /etc/haproxy/lua /var/lib/haproxy
COPY --from=rust-builder /build/target/release/libhaproxy_otel_module.so /usr/local/lib/lua/5.4/haproxy_otel_module.so

# Copy OTEL Lua configuration script
COPY lua/otel.lua /etc/haproxy/lua/otel.lua

# Set permissions
RUN chown -R haproxy:haproxy /usr/local/etc/haproxy /run /var /var/lib/haproxy /etc/haproxy/lua && \
    chmod -R ug+rwx /usr/local/etc/haproxy /run /var /var/lib/haproxy && \
    chmod 644 /etc/gopherd/gopherd.yml && \
    chmod u+rx /usr/local/sbin/haproxy_wrapper && \
    setcap 'cap_net_bind_service=+ep' /usr/local/sbin/haproxy_wrapper && \
    chown haproxy:haproxy /usr/local/lib/lua/5.4/haproxy_otel_module.so && \
    chown haproxy:haproxy /etc/haproxy/lua/otel.lua

# Run as root like the original kubernetes-ingress image to allow chroot
USER root

# Set Lua path to include our module
ENV LUA_CPATH="/usr/local/lib/lua/5.4/?.so;;"

# Labels for image metadata
LABEL org.opencontainers.image.title="HAProxy Kubernetes Ingress with OpenTelemetry" \
      org.opencontainers.image.description="HAProxy Tech Kubernetes Ingress Controller with pre-compiled OpenTelemetry tracing module" \
      org.opencontainers.image.source="https://github.com/nais/haproxy-ingress-otel" \
      org.opencontainers.image.vendor="NAIS" \
      org.opencontainers.image.base.name="haproxytech/haproxy-debian:${HAPROXY_VERSION}" \
      org.opencontainers.image.version="${HAPROXY_INGRESS_VERSION}" \
      io.nais.haproxy-ingress.version="${HAPROXY_INGRESS_VERSION}" \
      io.nais.haproxy.version="${HAPROXY_VERSION}"

STOPSIGNAL SIGTERM

ENTRYPOINT ["/usr/local/sbin/gopherd"]
