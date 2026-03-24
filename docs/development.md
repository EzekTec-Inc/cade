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
| `cade-core` | cade-server, cade-agent, cade-cli |
| `cade-ai` | cade-server, cade-cli |
| `cade-desktop` | cade-agent |
| `cade-server` | root binary (cade-server) |
| `cade-agent` | cade-cli |
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
