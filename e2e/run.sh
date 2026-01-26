#!/bin/bash
set -e

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"

cd "$SCRIPT_DIR"

# Detect architecture
ARCH=$(uname -m)
case "$ARCH" in
    x86_64|amd64)  PLATFORM="linux/amd64" ;;
    arm64|aarch64) PLATFORM="linux/arm64" ;;
    *)             PLATFORM="linux/amd64" ;;
esac

echo "==> Detected architecture: $ARCH (platform: $PLATFORM)"

# Check for Colima on macOS
if [[ "$(uname -s)" == "Darwin" ]] && command -v colima &>/dev/null; then
    if colima status &>/dev/null; then
        export DOCKER_HOST="${DOCKER_HOST:-unix://$HOME/.colima/default/docker.sock}"
        echo "==> Using Colima: $DOCKER_HOST"
    fi
fi

# Build the image if it doesn't exist or if BUILD=1
IMAGE_NAME="haproxy-otel:test"
if [[ "${BUILD:-0}" == "1" ]] || ! docker image inspect "$IMAGE_NAME" &>/dev/null; then
    echo "==> Building haproxy-otel image for $PLATFORM..."
    docker build --platform "$PLATFORM" -t "$IMAGE_NAME" "$REPO_ROOT"
else
    echo "==> Using existing haproxy-otel image (set BUILD=1 to rebuild)"
fi

echo ""
echo "==> Starting e2e test environment (docker-compose)..."
echo "    This runs HAProxy standalone with OTEL module."
echo "    For Kubernetes ingress controller testing, use: ./kind-e2e.sh"
echo ""

# Export PLATFORM for docker-compose
export PLATFORM

docker compose up -d

echo ""
echo "==> Waiting for services to be ready..."
sleep 3

echo ""
echo "==> Sending test requests to HAProxy..."
for i in {1..5}; do
  curl -s -o /dev/null -w "Request $i: HTTP %{http_code}\n" http://localhost:8080/
done

echo ""
curl -s -o /dev/null -w "Health check: HTTP %{http_code}\n" http://localhost:8080/health

echo ""
echo "==> Waiting for traces to be flushed..."
sleep 10

echo ""
echo "==> Checking Jaeger for traces..."
# Retry a few times as batch exporter may take time to flush
for attempt in 1 2 3; do
    TRACES=$(curl -s "http://localhost:16686/api/traces?service=haproxy-e2e-test&limit=10" || echo '{"data":[]}')
    TRACE_COUNT=$(echo "$TRACES" | grep -o '"traceID"' | wc -l | tr -d ' ')
    if [ "$TRACE_COUNT" -gt 0 ]; then
        break
    fi
    echo "Attempt $attempt: No traces yet, waiting..."
    sleep 5
done

if [ "$TRACE_COUNT" -gt 0 ]; then
    echo "✓ Found $TRACE_COUNT trace(s) in Jaeger"
    echo ""
    echo "=========================================="
    echo "✓ E2E test passed!"
    echo "=========================================="
else
    echo "✗ No traces found in Jaeger"
    echo "HAProxy logs:"
    docker compose logs haproxy 2>&1 | tail -30
    echo ""
    echo "Jaeger response: $TRACES"
    exit 1
fi

echo ""
echo "View traces at: http://localhost:16686"
echo "HAProxy stats: http://localhost:8404/stats"
echo ""
echo "Run 'docker compose down' to cleanup"
