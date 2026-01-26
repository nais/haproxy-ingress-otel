#!/bin/bash
#
# E2E test for HAProxy OTEL using Kind (Kubernetes in Docker)
#
# Usage:
#   ./e2e/kind-e2e.sh          # Run e2e tests (builds image if needed)
#   BUILD=0 ./e2e/kind-e2e.sh  # Skip build, use existing image
#   BUILD=1 ./e2e/kind-e2e.sh  # Force rebuild
#
# Environment variables:
#   BUILD      - Set to 0 to skip build, 1 to force rebuild
#   PLATFORM   - Override platform (linux/amd64 or linux/arm64)
#
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"
CLUSTER_NAME="haproxy-otel-e2e"
IMAGE_NAME="haproxy-otel:test"

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
    # Kill any leftover port-forwards
    pkill -f "kubectl port-forward" 2>/dev/null || true
    # Delete Kind cluster
    kind delete cluster --name "$CLUSTER_NAME" 2>/dev/null || true
    exit $exit_code
}

trap cleanup EXIT INT TERM

# Build the image if it doesn't exist or if BUILD=1
if [[ "${BUILD:-}" == "0" ]]; then
    echo "==> Skipping build (BUILD=0)"
elif [[ "${BUILD:-}" == "1" ]] || ! docker image inspect "$IMAGE_NAME" &>/dev/null; then
    echo "==> Building haproxy-otel image for $PLATFORM..."
    docker build --platform "$PLATFORM" -t "$IMAGE_NAME" "$REPO_ROOT"
else
    echo "==> Using existing haproxy-otel image (set BUILD=1 to rebuild)"
fi

echo "==> Creating kind cluster..."
kind delete cluster --name "$CLUSTER_NAME" 2>/dev/null || true
kind create cluster --name "$CLUSTER_NAME" --config "$SCRIPT_DIR/kind-config.yaml" --wait 120s

echo "==> Loading haproxy-otel image into kind..."
kind load docker-image "$IMAGE_NAME" --name "$CLUSTER_NAME"

echo "==> Applying Kubernetes manifests..."
kubectl apply -f "$SCRIPT_DIR/k8s/"

echo "==> Initial pod status:"
sleep 5
kubectl get pods -A

echo "==> Waiting for Jaeger to be ready..."
kubectl wait --for=condition=available --timeout=120s deployment/jaeger -n haproxy-otel-e2e

echo "==> Waiting for HAProxy Ingress to be ready (up to 5 minutes)..."
if ! kubectl wait --for=condition=available --timeout=300s deployment/haproxy-ingress -n haproxy-ingress; then
    echo ""
    echo "==> HAProxy Ingress failed to become ready. Debug info:"
    echo ""
    echo "--- Pod status ---"
    kubectl get pods -n haproxy-ingress -o wide
    echo ""
    echo "--- Pod describe ---"
    kubectl describe pods -n haproxy-ingress -l app=haproxy-ingress
    echo ""
    echo "--- Pod logs ---"
    kubectl logs -n haproxy-ingress -l app=haproxy-ingress --all-containers --tail=100 || true
    echo ""
    echo "--- Events ---"
    kubectl get events -n haproxy-ingress --sort-by='.lastTimestamp'
    exit 1
fi

echo "==> Waiting for echo-server to be ready..."
kubectl wait --for=condition=available --timeout=60s deployment/echo-server -n haproxy-otel-e2e

echo "==> Checking HAProxy logs for OTEL initialization..."
sleep 5
LOGS=$(kubectl logs -n haproxy-ingress -l app=haproxy-ingress --tail=100 2>&1)
if echo "$LOGS" | grep -q "OpenTelemetry initialized"; then
    echo "✓ OTEL module initialized successfully"
else
    echo "✗ OTEL initialization not found in logs"
    echo "$LOGS"
    exit 1
fi

echo "==> Starting port-forward to HAProxy ingress..."
kubectl port-forward -n haproxy-ingress svc/haproxy-ingress 8080:80 &
PF_PID=$!
sleep 3

# Verify port-forward is working
if ! kill -0 $PF_PID 2>/dev/null; then
    echo "✗ Port-forward failed to start"
    exit 1
fi
echo "✓ Port-forward ready (PID: $PF_PID)"

echo "==> Sending test requests..."
for i in {1..5}; do
    RESPONSE=$(curl -s -o /dev/null -w "%{http_code}" -H "Host: echo.local" http://localhost:8080/ || echo "000")
    echo "Request $i: HTTP $RESPONSE"
    if [[ "$RESPONSE" != "200" ]]; then
        echo "✗ Request failed with HTTP $RESPONSE"
        kubectl logs -n haproxy-ingress -l app=haproxy-ingress --tail=50
        kill $PF_PID 2>/dev/null || true
        exit 1
    fi
done

kill $PF_PID 2>/dev/null || true

echo "==> Verifying traces in Jaeger..."
echo "    Waiting for batch exporter to flush..."
sleep 10

kubectl port-forward -n haproxy-otel-e2e svc/jaeger 16686:16686 &
JAEGER_PF_PID=$!
sleep 3

# Query Jaeger API for traces with retries
TRACE_COUNT=0
for attempt in 1 2 3 4 5; do
    TRACES=$(curl -s "http://localhost:16686/api/traces?service=haproxy-ingress-e2e&limit=10" || echo '{"data":[]}')
    TRACE_COUNT=$(echo "$TRACES" | grep -o '"traceID"' | wc -l | tr -d ' ')
    if [[ "$TRACE_COUNT" -gt 0 ]]; then
        break
    fi
    echo "    Attempt $attempt: No traces yet, waiting..."
    sleep 5
done

kill $JAEGER_PF_PID 2>/dev/null || true

if [[ "$TRACE_COUNT" -gt 0 ]]; then
    echo "✓ Found $TRACE_COUNT trace(s) in Jaeger"
else
    echo "✗ No traces found in Jaeger after 5 attempts"
    echo "Jaeger response: $TRACES"
    echo ""
    echo "HAProxy Ingress logs:"
    kubectl logs -n haproxy-ingress -l app=haproxy-ingress --tail=50
    exit 1
fi

echo ""
echo "==========================================="
echo "✓ E2E tests passed!"
echo "==========================================="
