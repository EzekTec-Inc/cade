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

### OpenAI-compatible providers

CADE routes OpenAI, GPT/o-series, GPT-5-style models, and OpenRouter through
the OpenAI-compatible provider implementation in `crates/cade-ai`. The runtime
selects a provider from explicit model prefixes such as `openrouter/...`, from
configured provider defaults, or from model-name auto-detection.

OpenRouter is configured as an OpenAI-compatible provider with these upstream
endpoints:

| Endpoint | URL |
|----------|-----|
| Chat completions | `https://openrouter.ai/api/v1/chat/completions` |
| Model catalogue | `https://openrouter.ai/api/v1/models` |

When OpenRouter model IDs are routed through CADE, the provider prefix is
stripped before sending the upstream request. For example, `openrouter/foo/bar`
is resolved as OpenRouter and sent upstream as `foo/bar`.

OpenAI Responses API tool definitions must use the flat function-tool shape:
`type`, `name`, `description`, `parameters`, and `strict: false` all belong at
the top level of each tool object. CADE intentionally sets `strict` to `false`
so optional fields and runtime MCP-provided nested schemas remain compatible
with OpenAI-compatible tool calling.

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

### Rust Workspace Tests

```bash
# All workspace tests (~1,434 tests)
cargo test --workspace

# Specific crate
cargo test -p cade-cli

# Semantic search tests are included in the default suite as of 2026-04-30
# (~132 of cade-store's 133 lib tests exercise the embedding code path).
# To run the keyword-only path, disable default features:
cargo test -p cade-store --no-default-features --features bundled-sqlite

# With output
cargo test --workspace -- --nocapture
```

### Neovim Plugin Tests

The Neovim plugin has two independent test suites:

1. **MCP Runner Tests**: Simple unit tests that run via a custom minimal runner.
   ```bash
   nvim --headless --noplugin -u editors/neovim/spec/mcp/runner.lua
   ```

2. **Plenary Busted Tests**: Rich spec tests that leverage plenary's busted runner. Note that we isolate from your local `init.lua` to prevent user-configuration or plugin conflicts.
   ```bash
   XDG_CONFIG_HOME=/tmp/clean_nvim nvim --headless -u editors/neovim/spec/minimal_init.lua -c "PlenaryBustedDirectory editors/neovim/spec {minimal_init = 'editors/neovim/spec/minimal_init.lua'}" -c "qa!"
   ```

## Release Build

```bash
# Standard build (semantic memory search enabled — adds ~50MB for the
# embedding model + ONNX runtime; first run downloads the model)
cargo build --release

# Lean build without semantic memory search (keyword search only)
cargo build --release -p cade-store --no-default-features --features bundled-sqlite
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
cargo build --features full                    # desktop + web + MCP + SDK integration + remote backends
cargo build --no-default-features              # minimal root package without default MCP feature
cargo build --features semantic-search         # include local embeddings via cade-store
cargo build --no-default-features --features mcp
```

Feature flags are defined in:
- `Cargo.toml` (root) — `default = ["mcp"]`, `full`, `desktop`, `web`, `mcp`, `integration`, `backend-docker`, `backend-ssh`, `semantic-search`
- `crates/cade-agent/Cargo.toml` — `desktop`, `web`, `mcp`, `backend-docker`, `backend-ssh`
- `crates/cade-store/Cargo.toml` — `bundled-sqlite` (default) and `semantic-search` (fastembed + sqlite-vec embeddings, opt-in)

When adding a new optional dependency, place it behind a feature flag in the
owning crate and wire it through the root Cargo.toml when it must be selectable
from top-level builds.

### Root package topology

The root package (`src/`) owns both binaries (`cade` and `cade-server`).
It re-exports workspace crates via `src/lib.rs` so binaries can use
`cade::agent`, `cade::server`, etc.

This means **both binaries share the same dependency set**. Server-only deps
(axum, tower-http, cade-ai) are compiled even for the `cade` client binary.
A future improvement would split binaries into separate crates, but the
current layout works and the release build uses LTO to eliminate dead code.

## Interactive TUI Focus & Keybindings

CADE supports dynamic focus routing and keyboard controls across its plugin region slots (`Sidebar`, `Header`, `Footer`):

*   **`Ctrl+F`**: Cycles focus between the main prompt input editor and any active, occupied UI slots.
*   **`Esc`**: Immediately drops slot focus and reverts keyboard target to the main prompt input.
*   **Focus Highlighting**: Active slots render with a distinct, colored border (`colors.border_focus()`) instead of muted borders.
*   **Interactive Slot Navigation**: When a slot is focused, all input keystrokes are routed directly to that slot component's `handle_input(k)` before propagating down. The concrete `LuaUiSlot` implementation maps:
    *   `Up` / `Down` / `Tab` / `BackTab` to navigate interactive widgets.
    *   `Enter` / `Space` to click buttons or toggle states.
    *   `Left` / `Right` to cycle choices inside custom Lists.

## TUI State Signals & Extensibility Runtimes

CADE implements reactive, declarative state signals and a dual-sandbox dynamic plugin loader system:

### Declarative Signals (`signals.rs`)

To optimize rendering cycles and minimize CPU load, CADE implements `Signal<T>` backed by thread-safe `tokio::sync::watch` channels. Writing to a signal sets a global dirty flag. The TUI event tick loop selectively draws frames only when this dirty flag is set, reducing idle rendering overhead to near-zero.

### JavaScript / TypeScript Plugin Runtime (`rquickjs`)

CADE integrates a sandboxed QuickJS context (`JsRuntime`) feature-gated behind `plugin-js`. This allows developers to author local plugins in JavaScript/TypeScript using a robust sandboxed runtime that restricts memory consumption to 32MB and exposes native filesystem, globbing, and interactive prompt bindings.

### WebAssembly Sandboxed Runtime (`wasmtime`)

High-performance, cross-language dynamic plugins can be loaded as compiled `.wasm` modules inside a strict WASI container powered by Wasmtime. The WASM runtime automatically parses guest module exports starting with `cade_tool_` to register and dispatch custom tools safely at runtime.

### Active Permission Gating

CADE's `ToolContext` exposes an asynchronous `ask_permission` callback allowing running tools to programmatically request user permissions for nested or sub-actions mid-execution. When a tool invokes this, CADE's parallel execution engine halts the future and pushes a floating `PermissionOverlay` modal onto the TUI stack, resuming execution once the user approves, denies, or selects "always allow for this session."
