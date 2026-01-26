# E2E Tests

End-to-end tests using Kind (Kubernetes in Docker) to test the HAProxy Kubernetes Ingress Controller with OpenTelemetry tracing.

## Running Tests

```bash
# Run e2e tests (builds image if needed)
./e2e/e2e.sh

# Skip build, use existing image
BUILD=0 ./e2e/e2e.sh

# Force rebuild
BUILD=1 ./e2e/e2e.sh
```

The test:

1. Creates a Kind cluster
2. Deploys Jaeger for trace collection
3. Deploys HAProxy Ingress Controller with OTEL module
4. Deploys a test echo-server with Ingress
5. Sends HTTP requests through the ingress
6. Verifies traces appear in Jaeger

The Kind cluster is automatically deleted after the test.

## Platform Support

The script runs identically locally and in CI:

- **Local (macOS ARM64)**: Auto-detects Colima, builds for `linux/arm64`
- **CI (Linux x86_64)**: Builds for `linux/amd64`

Override the platform if needed:

```bash
PLATFORM=linux/amd64 ./e2e/e2e.sh
```

## Debugging

If tests fail, the script outputs:

- Pod status
- Pod descriptions
- Container logs
- Kubernetes events

To run interactively for debugging:

```bash
# Create cluster and deploy
BUILD=0 ./e2e/e2e.sh &
# Ctrl+C after deployment succeeds

# Inspect manually
kubectl get pods -A
kubectl logs -n haproxy-ingress -l app=haproxy-ingress -f

# Access Jaeger UI
kubectl port-forward -n haproxy-otel-e2e svc/jaeger 16686:16686
open http://localhost:16686

# Cleanup
kind delete cluster --name haproxy-otel-e2e
```
