# E2E Tests

Two test modes are available:

## Docker Compose (HAProxy Standalone)

Tests the OTEL module with HAProxy directly (no Kubernetes):

```bash
# Build and run (first time or after changes)
BUILD=1 ./run.sh

# Run with existing image
./run.sh

# Cleanup
docker compose down
```

**Endpoints:**
- HAProxy: http://localhost:8080
- Jaeger UI: http://localhost:16686
- HAProxy stats: http://localhost:8404/stats

## Kind (Kubernetes Ingress Controller)

Tests the full HAProxy Kubernetes Ingress Controller with OTEL:

```bash
# Build and run (first time or after changes)
BUILD=1 ./kind-e2e.sh

# Run with existing image
./kind-e2e.sh
```

The kind cluster is automatically deleted after the test.

## Platform Support

Both scripts auto-detect the platform:
- **macOS ARM64** (Apple Silicon): Builds for `linux/arm64`, auto-detects Colima
- **Linux x86_64** (CI): Builds for `linux/amd64`

Set `DOCKER_HOST` manually if using a non-default Docker socket.
