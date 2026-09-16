# =============================================================================
# Version Configuration
# CI sources these from versions.env. Defaults here for local builds.
# Run 'mise run sync-versions' when updating versions.env.
# =============================================================================
ARG RUST_VERSION=1.97
ARG RUST_IMAGE_DIGEST=sha256:0e2bcaef56d041a486784e54104a81aebe0da44bd03019bd70bc0401e42e4a97
ARG HAPROXY_INGRESS_VERSION=3.2.15
ARG HAPROXY_INGRESS_COMMIT=acb08c239b356714dddf6a0e332b1751f0fd27df
ARG HAPROXY_INGRESS_IMAGE_DIGEST=sha256:6185ab228aa6a8f56fd8909e55fa4bbf82f6c765a28fe7e6cea69a7668f487e8
ARG BLOCK_SECRETS_SHA256=311c4e14d992293559eb064bf7c1c105ec321c4ff82f887c7e3b7bf5f200ef77
ARG HAPROXY_WRAPPER_SHA256=3c2a118d5b89792ac2cf9ea3a74a38abd4c6344850e7e7ca9fe962276ccfb689
ARG HAPROXY_VERSION=3.2
ARG HAPROXY_IMAGE_DIGEST=sha256:4aceb05780b31bc74a6bedd8622b6de4619f18cb1b7e7af860e27a04807b5aee

# =============================================================================
# Build stage: Rust OTEL module (glibc - cdylib doesn't support musl)
# =============================================================================
FROM rust:${RUST_VERSION}-bookworm@${RUST_IMAGE_DIGEST} AS rust-builder

WORKDIR /build

# Build the OTEL module and ingress protection binaries against glibc.
RUN apt-get update && apt-get upgrade -y && apt-get install -y --no-install-recommends \
    curl \
    gcc \
    libc6-dev \
    pkg-config \
    && rm -rf /var/lib/apt/lists/*

# Copy workspace files
COPY Cargo.toml Cargo.lock ./
COPY .cargo ./.cargo
COPY src ./src
COPY module ./module

# Remove tests from workspace members (tests requires edition2024/Rust 1.87+)
RUN sed -i 's/, "tests"//' Cargo.toml && sed -i 's/"tests", //' Cargo.toml

# Build the module in release mode
RUN cargo fetch --locked && cargo build --locked --release -p haproxy-otel-module

# HAProxy Ingress builds these in Alpine, but the final image uses glibc.
# Rebuild them here to avoid loading a musl interposer into HAProxy workers.
ARG HAPROXY_INGRESS_VERSION
ARG HAPROXY_INGRESS_COMMIT
ARG BLOCK_SECRETS_SHA256
ARG HAPROXY_WRAPPER_SHA256
RUN curl -fsSL \
        "https://raw.githubusercontent.com/haproxytech/kubernetes-ingress/${HAPROXY_INGRESS_COMMIT}/pkg/protection/block_secrets.c" \
        -o /build/block_secrets.c && \
    curl -fsSL \
        "https://raw.githubusercontent.com/haproxytech/kubernetes-ingress/${HAPROXY_INGRESS_COMMIT}/pkg/protection/haproxy_wrapper.c" \
        -o /build/haproxy_wrapper.c && \
    echo "${BLOCK_SECRETS_SHA256}  /build/block_secrets.c" | sha256sum -c - && \
    echo "${HAPROXY_WRAPPER_SHA256}  /build/haproxy_wrapper.c" | sha256sum -c - && \
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
ARG HAPROXY_INGRESS_IMAGE_DIGEST
FROM docker.io/haproxytech/kubernetes-ingress:${HAPROXY_INGRESS_VERSION}@${HAPROXY_INGRESS_IMAGE_DIGEST} AS ingress-source

# =============================================================================
# Final stage: Debian-based HAProxy with OTEL module and gopherd supervisor
# =============================================================================
ARG HAPROXY_VERSION
ARG HAPROXY_IMAGE_DIGEST
ARG HAPROXY_INGRESS_VERSION
FROM haproxytech/haproxy-debian:${HAPROXY_VERSION}@${HAPROXY_IMAGE_DIGEST}

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
