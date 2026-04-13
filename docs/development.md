# Development Guide

## Quick Start

```bash
# Clone and build
git clone https://github.com/EzekTec-Inc/CADE.git
cd CADE
cargo build

# Run tests
cargo test --workspace
```

## Workspace Structure

CADE is a Cargo workspace with twelve crates. Changes to one crate only recompile
that crate and its dependents — not the entire project.

| Crate | Rebuild triggers recompile of… |
|-------|-------------------------------|
| `cade-core` | cade-server, cade-agent, cade-cli, cade-plugin, cade-mcp, cade-web, cade-codeintel, cade-tui, cade-sdk |
| `cade-ai` | cade-server, cade-cli, cade-sdk |
| `cade-desktop` | cade-agent |
| `cade-mcp` | cade-agent |
| `cade-web` | cade-agent |
| `cade-codeintel` | cade-server |
| `cade-tui` | cade-cli |
| `cade-plugin` | cade-agent, cade-cli |
| `cade-sdk` | root binary (cade) |
| `cade-server` | root binary (cade-server) |
| `cade-agent` | cade-cli, cade-sdk |
| `cade-cli` | root binary (cade) |

## Environment Variables

### Server

| Variable | Default | Description |
|----------|---------|-------------|
| `ANTHROPIC_API_KEY` | — | Anthropic/Claude API key |
| `OPENAI_API_KEY` | — | OpenAI API key |
| `GOOGLE_API_KEY` | — | Google Gemini API key |
| `OLLAMA_BASE_URL` | `http://localhost:11434` | Ollama endpoint |
| `CADE_SERVER_PORT` | `8284` | Server listen port |
| `CADE_DB_PATH` | `~/.cade/cade.db` | SQLite database path |
| `CADE_API_KEY` | — | Optional auth token |
| `CADE_LLM_PROVIDER` | auto-detect | Force a provider |
| `CADE_DEFAULT_MODEL` | provider default | Force a model |

### CLI

| Variable | Default | Description |
|----------|---------|-------------|
| `CADE_SERVER_URL` | `http://localhost:8284` | Server URL |
| `CADE_API_KEY` | — | Auth token |

### Security

| Variable | Default | Description |
|----------|---------|-------------|
| `CADE_FS_ROOT` | — | Restrict file tools to this directory |

## Debugging

### Server logs

The server logs to stderr with `tracing`. Set the log level:

```bash
RUST_LOG=debug cade-server
```

### CLI logs

The CLI redirects tracing output to `/tmp/cade.log` to avoid TUI corruption:

```bash
tail -f /tmp/cade.log
```

### Database inspection

```bash
sqlite3 ~/.cade/cade.db ".tables"
sqlite3 ~/.cade/cade.db "SELECT id, name, model FROM agents ORDER BY created_at DESC LIMIT 10"
```

## Testing

```bash
# All workspace tests
cargo test --workspace

# Specific crate
cargo test -p cade-cli

# With output
cargo test --workspace -- --nocapture
```

## Release Build

```bash
cargo build --release
# Binaries: target/release/cade, target/release/cade-server
```

## Adding a New LLM Provider

1. Create `crates/cade-ai/src/new_provider.rs`
2. Implement `LlmProvider` trait (`complete` + `stream`)
3. Register in `crates/cade-ai/src/lib.rs`:
   - Add `pub mod new_provider;`
   - Add to `LlmRouter::build()` auto-detection
   - Add to `provider_from_row()` match
4. Add model entries to `catalogue.rs`

## Adding a New Tool

1. Create `crates/cade-agent/src/tools/new_tool.rs`
2. Implement `run()` returning `(String, bool)` — (output, is_error)
3. Register schema in `crates/cade-agent/src/tools/manager.rs`
4. Add to `dispatch()` match in manager.rs
5. Add to the appropriate `Toolset` in `crates/cade-core/src/toolsets/mod.rs`

## Capability System

### Adding a new capability

1. Add variant to `Capability` enum in `crates/cade-core/src/capabilities/mod.rs`
2. Update `Capability::ALL`, `name()`, and `from_name()`
3. Add to relevant `Profile` presets in `Profile::capabilities()`
4. Classify tools in `crates/cade-agent/src/tools/catalog.rs`:
   - `meta_tool_capability()` for meta tools
   - `native_tool_capability()` for native tools
5. Optionally gate commands in `crates/cade-cli/src/cli/repl/capability_gate.rs`

### Build profiles

The workspace supports compile-time feature gating:

```bash
cargo build --features full                          # default — everything
cargo build --no-default-features --features pro     # no desktop/web/mcp/sdk
cargo build --no-default-features --features lean    # minimal core only
```

Feature flags are defined in:
- `Cargo.toml` (root) — `full`, `pro`, `lean`, `desktop`, `web`, `mcp`, `integration`
- `crates/cade-agent/Cargo.toml` — `desktop`, `web`, `mcp`

When adding a new optional dependency, place it behind a feature flag in the
owning crate and wire it through the root Cargo.toml.

### Root package topology

The root package (`src/`) owns both binaries (`cade` and `cade-server`).
It re-exports workspace crates via `src/lib.rs` so binaries can use
`cade::agent`, `cade::server`, etc.

This means **both binaries share the same dependency set**. Server-only deps
(axum, tower-http, cade-ai) are compiled even for the `cade` client binary.
A future improvement would split binaries into separate crates, but the
current layout works and the release build uses LTO to eliminate dead code.
