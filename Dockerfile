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

# Build the OTEL module and ingress protection binaries against glibc.
RUN apt-get update && apt-get upgrade -y && apt-get install -y --no-install-recommends \
    curl \
    gcc \
    libc6-dev \
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

# HAProxy Ingress builds these in Alpine, but the final image uses glibc.
# Rebuild them here to avoid loading a musl interposer into HAProxy workers.
ARG HAPROXY_INGRESS_VERSION
RUN curl -fsSL \
        "https://raw.githubusercontent.com/haproxytech/kubernetes-ingress/v${HAPROXY_INGRESS_VERSION}/pkg/protection/block_secrets.c" \
        -o /build/block_secrets.c && \
    curl -fsSL \
        "https://raw.githubusercontent.com/haproxytech/kubernetes-ingress/v${HAPROXY_INGRESS_VERSION}/pkg/protection/haproxy_wrapper.c" \
        -o /build/haproxy_wrapper.c && \
    gcc -O3 -std=c11 -pipe -fPIC -shared -s \
        -D_FORTIFY_SOURCE=2 -fstack-protector-strong -fstack-clash-protection \
        -fno-omit-frame-pointer -flto \
        -Wl,-z,relro -Wl,-z,now -Wl,-z,noexecstack -Wl,--as-needed -Wl,-z,defs \
        -o /build/libblock_secrets.so /build/block_secrets.c -ldl && \
    gcc -O3 -std=c11 -pipe -s \
        -D_FORTIFY_SOURCE=2 -fstack-protector-strong -fstack-clash-protection \
        -fno-omit-frame-pointer -fPIE -pie \
        -Wl,-z,relro -Wl,-z,now -Wl,-z,noexecstack -Wl,--as-needed \
        -o /build/haproxy_wrapper /build/haproxy_wrapper.c

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
    libcap2-bin \
    ca-certificates \
    && rm -rf /var/lib/apt/lists/*

# Copy the 3.2.15 ingress runtime and its gopherd supervisor configuration.
COPY --from=ingress-source /haproxy-ingress-controller /haproxy-ingress-controller
COPY --from=ingress-source /usr/local/sbin/gopherd /usr/local/sbin/gopherd
COPY --from=ingress-source /etc/gopherd/gopherd.yml /etc/gopherd/gopherd.yml
COPY --from=ingress-source /etc/haproxy/haproxy.cfg /etc/haproxy/haproxy.cfg
COPY --from=ingress-source /etc/haproxy/errors /etc/haproxy/errors
COPY --from=rust-builder /build/haproxy_wrapper /usr/local/sbin/haproxy_wrapper
COPY --from=rust-builder /build/libblock_secrets.so /usr/local/lib/libblock_secrets.so

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
