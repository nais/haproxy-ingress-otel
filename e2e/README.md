# E2E Tests

End-to-end tests for the HAProxy OTEL module.

## Test Types

| Script        | Dependencies   | Use Case                |
| ------------- | -------------- | ----------------------- |
| `e2e.sh`      | Docker Compose | CI, fast feedback       |
| `kind-e2e.sh` | Kind, Helm     | Full Kubernetes testing |

## Docker Compose Test (CI)

Runs HAProxy directly with Docker Compose - fastest option, used in CI:

```bash
./e2e/e2e.sh          # Build and test
BUILD=0 ./e2e/e2e.sh  # Skip build, use existing image
```

## Kind/Helm Test (Kubernetes)

Tests the full HAProxy Kubernetes Ingress Controller deployment with the official Helm chart:

```bash
./e2e/kind-e2e.sh          # Build and test
BUILD=0 ./e2e/kind-e2e.sh  # Skip build, use existing image
```

The test:

1. Creates a Kind cluster with NodePort mappings (9080→30080, 9443→30443)
2. Installs HAProxy Ingress Controller via official Helm chart with custom OTEL image
3. Deploys Jaeger for trace collection
4. Deploys a test nginx backend with Ingress
5. Sends HTTP requests through the ingress
6. Verifies traces appear in Jaeger

## Files

| File                  | Description                                     |
| --------------------- | ----------------------------------------------- |
| `e2e.sh`              | Docker Compose e2e test (CI)                    |
| `kind-e2e.sh`         | Kind/Helm e2e test (Kubernetes)                 |
| `docker-compose.yaml` | Services for Docker Compose test                |
| `haproxy.cfg`         | HAProxy config for Docker Compose test          |
| `kind-config.yaml`    | Kind cluster configuration with port mappings   |
| `helm-values.yaml`    | Helm values for HAProxy with OTEL configuration |
| `test-app.yaml`       | Test application (Jaeger + nginx + Ingress)     |

## Platform Support

Both scripts detect architecture automatically:

- **macOS ARM64**: Auto-detects Colima, builds for `linux/arm64`
- **Linux x86_64**: Builds for `linux/amd64`

Override if needed:

```bash
PLATFORM=linux/amd64 ./e2e/e2e.sh
```

## Manual Kind Testing

```bash
# Create cluster
kind create cluster --name haproxy-otel --config e2e/kind-config.yaml

# Build and load image
docker build -t haproxy-otel:test .
kind load docker-image haproxy-otel:test --name haproxy-otel

# Install via Helm
helm repo add haproxytech https://haproxytech.github.io/helm-charts
helm install haproxy-ingress haproxytech/kubernetes-ingress \
    -n haproxy-ingress --create-namespace \
    -f e2e/helm-values.yaml

# Deploy test app
kubectl apply -f e2e/test-app.yaml

# Test
curl -H "Host: echo.local" http://localhost:9080/

# Access Jaeger UI
kubectl port-forward -n haproxy-otel-e2e svc/jaeger 16686:16686
open http://localhost:16686

# Cleanup
kind delete cluster --name haproxy-otel
```
