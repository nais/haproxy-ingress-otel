# Contributing

## Prerequisites

Install [mise](https://mise.jdx.dev/) for tool management:

```bash
curl https://mise.run | sh
mise trust
```

This installs Rust, kubectl, helm, and kind at the correct versions.

## Quick Start

```bash
# Build and run tests (skips integration tests if HAProxy unavailable)
mise run test

# Lint check
mise run lint

# Build release
mise run build
```

## Running Integration Tests

Integration tests require HAProxy compiled with Lua support:

```bash
# Build HAProxy + Lua locally (macOS/Linux)
mise run setup-haproxy

# Run all tests including integration
mise run test-integration
```

## E2E Tests

```bash
# Docker Compose based
mise run e2e

# Kubernetes (kind) based
mise run e2e-kind
```

## Available Tasks

| Task                        | Description                    |
| --------------------------- | ------------------------------ |
| `mise run test`             | Run unit tests                 |
| `mise run lint`             | Run clippy and format check    |
| `mise run build`            | Build release binary           |
| `mise run e2e`              | Docker-based e2e tests         |
| `mise run e2e-kind`         | Kubernetes e2e tests           |
| `mise run setup-haproxy`    | Compile HAProxy with Lua       |
| `mise run test-integration` | Full test suite with HAProxy   |
| `mise run sync-versions`    | Sync versions.env to all files |
| `mise run check-versions`   | Verify version consistency     |

## Version Management

All versions are defined in [`versions.env`](versions.env):

```bash
# 1. Edit the source of truth
vim versions.env

# 2. Propagate to Dockerfile, manifests, etc.
mise run sync-versions

# 3. Verify
mise run check-versions

# 4. Commit
git add -A && git commit -m "deps: bump X to Y"
```

CI validates version consistency automatically.

## Code Style

- Run `cargo fmt` before committing
- No compiler warnings (`cargo clippy -- -D warnings`)
- Keep Lua module and Rust code named `haproxy-otel` (internal)
- External artifacts use `haproxy-ingress-otel`

## Pull Requests

1. Fork and create a feature branch
2. Make changes with tests
3. Run `mise run lint && mise run test`
4. Submit PR against `main`

CI runs automatically on all PRs.
