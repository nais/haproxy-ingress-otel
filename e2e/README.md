# E2E Tests

Two test modes are available:

## Docker Compose (HAProxy Standalone)

Tests the OTEL module with HAProxy directly (no Kubernetes).

### Interactive Mode (for development/debugging)

Run the services and keep them running to explore traces:

```bash
# Build the image first (if needed)
cd .. && docker build -t haproxy-otel:test .

# Start services
cd e2e
docker compose up -d

# Generate some traffic
curl http://localhost:8080/
curl http://localhost:8080/health

# View traces in Jaeger UI
open http://localhost:16686

# Follow logs
docker compose logs -f haproxy

# Stop when done
docker compose down
```

### Automated Test Mode

Runs the full test suite and exits:

```bash
# Build and run (first time or after changes)
BUILD=1 ./run.sh

# Run with existing image
./run.sh
```

**Endpoints:**
- HAProxy: http://localhost:8080
- HAProxy stats: http://localhost:8404/stats
- Jaeger UI: http://localhost:16686

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
