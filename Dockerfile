# Build stage for the Rust OTEL module (glibc - cdylib doesn't support musl)
FROM rust:1.87-bookworm AS rust-builder

WORKDIR /build

# Install build dependencies (no openssl needed - using rustls)
RUN apt-get update && apt-get install -y --no-install-recommends \
    pkg-config \
    && rm -rf /var/lib/apt/lists/*

# Copy workspace files
COPY Cargo.toml ./
COPY .cargo ./.cargo
COPY src ./src
COPY module ./module

# Remove tests from workspace (requires Rust 1.87+ due to edition2024)
RUN sed -i 's/"tests",//' Cargo.toml

# Build the module in release mode
RUN cargo fetch && cargo build --release -p haproxy-otel-module

# Extract the ingress controller binary and config from the official Alpine image
FROM docker.io/haproxytech/kubernetes-ingress:3.2.4 AS ingress-source

# Final image based on HAProxy Debian with glibc
FROM haproxytech/haproxy-debian:3.2

ARG TARGETPLATFORM
ARG S6_OVERLAY_VERSION=3.1.6.2

ENV S6_OVERLAY_VERSION=$S6_OVERLAY_VERSION
ENV S6_READ_ONLY_ROOT=1
ENV S6_USER=haproxy
ENV S6_GROUP=haproxy

USER root

# Install runtime dependencies
RUN apt-get update && apt-get install -y --no-install-recommends \
    socat \
    openssl \
    htop \
    tzdata \
    curl \
    libcap2-bin \
    ca-certificates \
    xz-utils \
    && rm -rf /var/lib/apt/lists/*

# Copy ingress controller binary and s6-overlay config from official image
COPY --from=ingress-source /haproxy-ingress-controller /haproxy-ingress-controller
COPY --from=ingress-source /start.sh /start.sh
COPY --from=ingress-source /init /init
COPY --from=ingress-source /etc/s6-overlay /etc/s6-overlay
COPY --from=ingress-source /command /command
COPY --from=ingress-source /package /package

# Install s6-overlay binaries (redownload for correct architecture)
RUN case "${TARGETPLATFORM}" in \
        "linux/arm64")      S6_ARCH=aarch64      ;; \
        "linux/amd64")      S6_ARCH=x86_64       ;; \
        "linux/arm/v6")     S6_ARCH=arm          ;; \
        "linux/arm/v7")     S6_ARCH=armhf        ;; \
        *)                  S6_ARCH=x86_64       ;; \
    esac && \
    curl -sS -L -o /tmp/s6-overlay-scripts.tar.xz \
        "https://github.com/just-containers/s6-overlay/releases/download/v${S6_OVERLAY_VERSION}/s6-overlay-noarch.tar.xz" && \
    tar -C / -Jxpf /tmp/s6-overlay-scripts.tar.xz && \
    curl -sS -L -o /tmp/s6-overlay-binaries.tar.xz \
        "https://github.com/just-containers/s6-overlay/releases/download/v${S6_OVERLAY_VERSION}/s6-overlay-${S6_ARCH}.tar.xz" && \
    tar -C / -Jxpf /tmp/s6-overlay-binaries.tar.xz && \
    rm -f /tmp/s6-overlay-scripts.tar.xz /tmp/s6-overlay-binaries.tar.xz

# Create Lua module directory and copy OTEL module
RUN mkdir -p /usr/local/lib/lua/5.4 /etc/haproxy/lua
COPY --from=rust-builder /build/target/release/libhaproxy_otel_module.so /usr/local/lib/lua/5.4/haproxy_otel_module.so

# Copy default OTEL configuration Lua script
COPY docker/otel.lua /etc/haproxy/lua/otel.lua

# Set permissions
RUN chown -R haproxy:haproxy /usr/local/etc/haproxy /run /var /etc/haproxy/lua && \
    chmod -R ug+rwx /usr/local/etc/haproxy /run /var && \
    chown -R haproxy:haproxy /init /etc/s6-overlay 2>/dev/null || true && \
    chmod u+x /init /start.sh /etc/s6-overlay/scripts/* 2>/dev/null || true && \
    chown haproxy:haproxy /usr/local/lib/lua/5.4/haproxy_otel_module.so && \
    chown haproxy:haproxy /etc/haproxy/lua/otel.lua

USER haproxy

# Set Lua path to include our module
ENV LUA_CPATH="/usr/local/lib/lua/5.4/?.so;;"

# Labels for image metadata
LABEL org.opencontainers.image.title="HAProxy Kubernetes Ingress with OpenTelemetry" \
      org.opencontainers.image.description="HAProxy Tech Kubernetes Ingress Controller with pre-compiled OpenTelemetry tracing module" \
      org.opencontainers.image.source="https://github.com/nais/haproxy-otel" \
      org.opencontainers.image.vendor="NAIS" \
      org.opencontainers.image.base.name="haproxytech/haproxy-debian:3.2"

ENTRYPOINT ["/start.sh"]
