#!/bin/bash
#
# E2E test for HAProxy OTEL using Docker Compose
#
# Usage:
#   ./e2e/e2e.sh          # Run e2e tests (always builds, Docker caching makes it fast)
#   BUILD=0 ./e2e/e2e.sh  # Skip build, use existing image
#
# Environment variables:
#   BUILD      - Set to 0 to skip build (default: 1, always build)
#   PLATFORM   - Override platform (linux/amd64 or linux/arm64)
#
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"
IMAGE_NAME="haproxy-ingress-otel:test"

# Detect architecture (can be overridden with PLATFORM env var)
if [[ -n "${PLATFORM:-}" ]]; then
    echo "==> Using PLATFORM from environment: $PLATFORM"
else
    ARCH=$(uname -m)
    case "$ARCH" in
        x86_64|amd64)  PLATFORM="linux/amd64" ;;
        arm64|aarch64) PLATFORM="linux/arm64" ;;
        *)             PLATFORM="linux/amd64" ;;
    esac
    echo "==> Detected architecture: $ARCH (platform: $PLATFORM)"
fi
export PLATFORM

# Check for Colima on macOS (local development)
if [[ "$(uname -s)" == "Darwin" ]] && command -v colima &>/dev/null; then
    if colima status &>/dev/null; then
        export DOCKER_HOST="${DOCKER_HOST:-unix://$HOME/.colima/default/docker.sock}"
        echo "==> Using Colima: $DOCKER_HOST"
    fi
fi

cleanup() {
    local exit_code=$?
    echo "==> Cleaning up..."
    docker compose -f "$SCRIPT_DIR/docker-compose.yaml" down -v 2>/dev/null || true
    exit $exit_code
}

trap cleanup EXIT INT TERM

# Build the image (Docker layer caching makes this fast when unchanged)
if [[ "${BUILD:-1}" == "0" ]]; then
    echo "==> Skipping build (BUILD=0)"
else
    echo "==> Building haproxy-ingress-otel image for $PLATFORM..."
    docker build --platform "$PLATFORM" -t "$IMAGE_NAME" "$REPO_ROOT"
fi

echo "==> Starting services..."
docker compose -f "$SCRIPT_DIR/docker-compose.yaml" up -d

echo "==> Waiting for services to be ready..."
sleep 5

# Wait for HAProxy to be ready
echo "==> Checking HAProxy health..."
for i in {1..30}; do
    if curl -sf http://localhost:8404/stats > /dev/null 2>&1; then
        echo "✓ HAProxy is ready"
        break
    fi
    if [[ $i -eq 30 ]]; then
        echo "✗ HAProxy failed to become ready"
        docker compose -f "$SCRIPT_DIR/docker-compose.yaml" logs haproxy
        exit 1
    fi
    sleep 1
done

echo "==> Sending test requests..."
for i in {1..5}; do
    RESPONSE=$(curl -s -D /tmp/headers -o /dev/null -w "%{http_code}" http://localhost:8080/ || echo "000")
    echo "Request $i: HTTP $RESPONSE"
    if [[ "$RESPONSE" != "200" ]]; then
        echo "✗ Request failed with HTTP $RESPONSE"
        docker compose -f "$SCRIPT_DIR/docker-compose.yaml" logs haproxy
        exit 1
    fi
done

echo "==> Verifying trace context headers..."
TRACE_ID=$(grep -i "x-trace-id:" /tmp/headers | tail -1 | awk '{print $2}' | tr -d '\r')
SPAN_ID=$(grep -i "x-span-id:" /tmp/headers | tail -1 | awk '{print $2}' | tr -d '\r')

if [[ -z "$TRACE_ID" || "$TRACE_ID" == "-" ]]; then
    echo "✗ X-Trace-Id header missing or empty"
    echo "Headers received:"
    cat /tmp/headers
    docker compose -f "$SCRIPT_DIR/docker-compose.yaml" logs haproxy
    exit 1
fi

if [[ -z "$SPAN_ID" || "$SPAN_ID" == "-" ]]; then
    echo "✗ X-Span-Id header missing or empty"
    echo "Headers received:"
    cat /tmp/headers
    docker compose -f "$SCRIPT_DIR/docker-compose.yaml" logs haproxy
    exit 1
fi

# Validate format (hex strings)
if [[ ! "$TRACE_ID" =~ ^[0-9a-f]{32}$ ]]; then
    echo "✗ X-Trace-Id invalid format: '$TRACE_ID' (expected 32 hex chars)"
    exit 1
fi

if [[ ! "$SPAN_ID" =~ ^[0-9a-f]{16}$ ]]; then
    echo "✗ X-Span-Id invalid format: '$SPAN_ID' (expected 16 hex chars)"
    exit 1
fi

echo "✓ Trace ID: $TRACE_ID"
echo "✓ Span ID:  $SPAN_ID"

echo "==> Verifying traces in Jaeger..."
echo "    Waiting for batch exporter to flush..."
sleep 5

# Query Jaeger API for traces with retries
TRACE_COUNT=0
for attempt in 1 2 3 4 5; do
    SERVICES=$(curl -s "http://localhost:16686/api/services" || echo '{"data":[]}')
    if echo "$SERVICES" | grep -q "haproxy-e2e-test"; then
        TRACES=$(curl -s "http://localhost:16686/api/traces?service=haproxy-e2e-test&limit=10" || echo '{"data":[]}')
        TRACE_COUNT=$(echo "$TRACES" | grep -o '"traceID"' | wc -l | tr -d ' ')
        if [[ "$TRACE_COUNT" -gt 0 ]]; then
            break
        fi
    fi
    echo "    Attempt $attempt: No traces yet, waiting..."
    sleep 3
done

if [[ "$TRACE_COUNT" -gt 0 ]]; then
    echo "✓ Found $TRACE_COUNT trace(s) in Jaeger for haproxy-e2e-test"
else
    echo "✗ No traces found in Jaeger after 5 attempts"
    echo "Services: $SERVICES"
    echo ""
    echo "HAProxy logs:"
    docker compose -f "$SCRIPT_DIR/docker-compose.yaml" logs haproxy
    exit 1
fi

echo ""
echo "==========================================="
echo "✓ E2E tests passed!"
echo "==========================================="
