#!/bin/bash
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"
CLUSTER_NAME="haproxy-otel-e2e"
IMAGE_NAME="haproxy-otel:test"

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

cleanup() {
    echo "==> Cleaning up..."
    kind delete cluster --name "$CLUSTER_NAME" 2>/dev/null || true
}

trap cleanup EXIT

# Build the image if it doesn't exist or if BUILD=1
if [[ "${BUILD:-0}" == "1" ]] || ! docker image inspect "$IMAGE_NAME" &>/dev/null; then
    echo "==> Building haproxy-otel image for $PLATFORM..."
    docker build --platform "$PLATFORM" -t "$IMAGE_NAME" "$REPO_ROOT"
else
    echo "==> Using existing haproxy-otel image (set BUILD=1 to rebuild)"
fi

echo "==> Creating kind cluster..."
kind create cluster --name "$CLUSTER_NAME" --config "$SCRIPT_DIR/kind-config.yaml" --wait 60s

echo "==> Loading haproxy-otel image into kind..."
kind load docker-image "$IMAGE_NAME" --name "$CLUSTER_NAME"

echo "==> Applying Kubernetes manifests..."
kubectl apply -f "$SCRIPT_DIR/k8s/"

echo "==> Checking initial pod status..."
sleep 5
kubectl get pods -A

echo "==> Waiting for Jaeger to be ready..."
kubectl wait --for=condition=available --timeout=120s deployment/jaeger -n haproxy-otel-e2e

echo "==> Waiting for HAProxy Ingress to be ready..."
if ! kubectl wait --for=condition=available --timeout=240s deployment/haproxy-ingress -n haproxy-ingress; then
    echo "==> HAProxy Ingress failed to become ready. Debugging info:"
    echo ""
    echo "Pod status:"
    kubectl get pods -n haproxy-ingress -o wide
    echo ""
    echo "Pod describe:"
    kubectl describe pods -n haproxy-ingress -l app=haproxy-ingress
    echo ""
    echo "Pod logs:"
    kubectl logs -n haproxy-ingress -l app=haproxy-ingress --all-containers --tail=100 || true
    echo ""
    echo "Events:"
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

echo "==> Port-forwarding HAProxy ingress..."
kubectl port-forward -n haproxy-ingress svc/haproxy-ingress 8080:80 &
PF_PID=$!
sleep 3

echo "==> Sending test requests..."
for i in {1..5}; do
    RESPONSE=$(curl -s -o /dev/null -w "%{http_code}" -H "Host: echo.local" http://localhost:8080/ || echo "000")
    echo "Request $i: HTTP $RESPONSE"
    if [ "$RESPONSE" != "200" ]; then
        echo "✗ Request failed with HTTP $RESPONSE"
        kubectl logs -n haproxy-ingress -l app=haproxy-ingress --tail=50
        kill $PF_PID 2>/dev/null || true
        exit 1
    fi
done

kill $PF_PID 2>/dev/null || true

echo "==> Verifying traces were sent to Jaeger..."
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
    if [ "$TRACE_COUNT" -gt 0 ]; then
        break
    fi
    echo "    Attempt $attempt: No traces yet, waiting..."
    sleep 5
done

kill $JAEGER_PF_PID 2>/dev/null || true

if [ "$TRACE_COUNT" -gt 0 ]; then
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
