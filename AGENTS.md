# Agent Instructions

You are an expert Rust and systems engineer working on haproxy-ingress-otel.

## Commands

```bash
mise run lint              # Format check + clippy
mise run test              # Unit tests
mise run test-integration  # Full tests (requires HAProxy+Lua)
mise run e2e               # Docker Compose e2e tests
mise run e2e-kind          # Kind-based e2e tests
mise run build             # Release build
mise run check-versions    # Verify version consistency
mise run sync-versions     # Propagate versions.env changes
```

## Tech Stack

- **Language:** Rust 1.97 (cdylib for Lua FFI)
- **Target:** HAProxy 3.2.x Lua module
- **Dependencies:** mlua, opentelemetry, tokio (multi-threaded runtime)
- **Build:** Cargo workspace with three crates
- **CI:** GitHub Actions with mise for task execution
- **Container:** Multi-arch Docker (amd64/arm64)

## Project Structure

```text
├── src/               # Main haproxy-otel library (Lua FFI bindings)
│   ├── lib.rs         # Entry point, Lua module registration
│   ├── span.rs        # OpenTelemetry span management
│   ├── filter.rs      # HAProxy filter implementation
│   ├── exporter.rs    # OTLP HTTP exporter
│   ├── cache.rs       # Thread-local span cache
│   └── runtime.rs     # Tokio runtime management
├── module/            # Lua C module wrapper (cdylib)
├── tests/             # Integration tests (require HAProxy+Lua)
├── lua/
│   └── otel.lua       # Lua loader script
├── e2e/               # End-to-end test infrastructure
├── versions.env       # Single source of truth for versions
├── Dockerfile         # Multi-stage build
└── mise.toml          # Task runner and tool versions
```

## Code Style

Rust conventions:

```rust
// Use descriptive error handling
pub fn init_tracer() -> Result<(), Box<dyn std::error::Error>> {
    // ...
}

// Thread-local storage for HAProxy worker threads
thread_local! {
    static SPAN_CACHE: RefCell<SpanCache> = RefCell::new(SpanCache::new());
}

// Lua function registration pattern
fn register_lua_functions(lua: &Lua) -> mlua::Result<()> {
    let globals = lua.globals();
    globals.set("start_span", lua.create_function(start_span)?)?;
    Ok(())
}
```

## Version Management

All versions live in `versions.env`. Never hardcode versions elsewhere.

```bash
# Update workflow
vim versions.env              # Edit source of truth
mise run sync-versions        # Propagate to Dockerfile, manifests
mise run check-versions       # Verify (CI does this automatically)
git commit -am "deps: bump X"
```

## Boundaries

✅ **Always do:**

- Run `mise run lint` before committing
- Write tests for new functionality
- Use `versions.env` for any version references
- Follow existing patterns in the codebase

⚠️ **Ask first:**

- Adding new dependencies to Cargo.toml
- Modifying the Dockerfile build process
- Changing HAProxy configuration patterns
- Modifying CI workflow structure

🚫 **Never do:**

- Commit secrets or credentials
- Modify `versions.env` without running `sync-versions`
- Remove or skip failing tests
- Change the public Lua API without updating lua/otel.lua
- Edit files in `.local/` (build artifacts)
