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
CLUSTER_NAME="haproxy-otel"
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
    pkill -f "kubectl port-forward" 2>/dev/null || true
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

echo "==> Adding Helm repo..."
helm repo add haproxytech https://haproxytech.github.io/helm-charts 2>/dev/null || true
helm repo update

echo "==> Installing HAProxy Ingress via Helm..."
helm install haproxy-ingress haproxytech/kubernetes-ingress \
    -n haproxy-ingress --create-namespace \
    -f "$SCRIPT_DIR/helm-values.yaml" \
    --wait --timeout 120s

echo "==> Deploying test application..."
kubectl apply -f "$SCRIPT_DIR/test-app.yaml"

echo "==> Initial pod status:"
kubectl get pods -A

echo "==> Waiting for Jaeger to be ready..."
kubectl wait --for=condition=available --timeout=120s deployment/jaeger -n haproxy-otel-e2e

echo "==> Waiting for echo-server to be ready..."
kubectl wait --for=condition=available --timeout=60s deployment/echo-server -n haproxy-otel-e2e

echo "==> HAProxy Ingress status:"
kubectl get pods -n haproxy-ingress

echo "==> Sending test requests..."
sleep 3
for i in {1..5}; do
    RESPONSE=$(curl -s -o /dev/null -w "%{http_code}" -H "Host: echo.local" http://localhost:9080/ || echo "000")
    echo "Request $i: HTTP $RESPONSE"
    if [[ "$RESPONSE" != "200" ]]; then
        echo "✗ Request failed with HTTP $RESPONSE"
        kubectl logs -n haproxy-ingress -l app.kubernetes.io/name=kubernetes-ingress --tail=50
        exit 1
    fi
done

echo "==> Verifying traces in Jaeger..."
echo "    Waiting for batch exporter to flush..."
sleep 5

kubectl port-forward -n haproxy-otel-e2e svc/jaeger 16686:16686 &
JAEGER_PF_PID=$!
sleep 3

# Query Jaeger API for traces with retries
TRACE_COUNT=0
for attempt in 1 2 3 4 5; do
    SERVICES=$(curl -s "http://localhost:16686/api/services" || echo '{"data":[]}')
    if echo "$SERVICES" | grep -q "haproxy-ingress"; then
        TRACES=$(curl -s "http://localhost:16686/api/traces?service=haproxy-ingress&limit=10" || echo '{"data":[]}')
        TRACE_COUNT=$(echo "$TRACES" | grep -o '"traceID"' | wc -l | tr -d ' ')
        if [[ "$TRACE_COUNT" -gt 0 ]]; then
            break
        fi
    fi
    echo "    Attempt $attempt: No traces yet, waiting..."
    sleep 5
done

kill $JAEGER_PF_PID 2>/dev/null || true

if [[ "$TRACE_COUNT" -gt 0 ]]; then
    echo "✓ Found $TRACE_COUNT trace(s) in Jaeger for haproxy-ingress"
else
    echo "✗ No traces found in Jaeger after 5 attempts"
    echo "Services response: $SERVICES"
    echo ""
    echo "HAProxy Ingress logs:"
    kubectl logs -n haproxy-ingress -l app.kubernetes.io/name=kubernetes-ingress --tail=50
    exit 1
fi

echo ""
echo "==========================================="
echo "✓ E2E tests passed!"
echo "==========================================="
