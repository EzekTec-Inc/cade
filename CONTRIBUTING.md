# Contributing to CADE

Thank you for your interest in contributing to CADE!

## Development Setup

### Prerequisites

- **Rust** (stable, 1.85+): `rustup default stable`
- **Supported platforms**: Linux (primary), macOS, Windows
- **Linux desktop deps** (optional — only needed for `--features desktop`):
  ```bash
  # Screen capture on Wayland
  sudo apt install libpipewire-0.3-dev libclang-dev libgbm-dev
  # Window control
  sudo apt install xdotool     # X11
  sudo apt install ydotool     # Wayland
  ```
- **Windows**: See [WINDOWS_SETUP.md](WINDOWS_SETUP.md) for MSVC build tools and `patch` utility setup
- **macOS**: Xcode Command Line Tools required (`xcode-select --install`)

### Optional: Faster Builds (Linux)

Install `sccache` and `mold` for significantly faster rebuilds:
```bash
cargo install sccache
sudo apt install mold
export RUSTC_WRAPPER=sccache   # add to your shell profile
```

### Build

```bash
# Debug build
cargo build

# Release build
cargo build --release

# Build with semantic search (local embeddings via fastembed + sqlite-vec)
cargo build --release --features semantic-search

# Run tests
cargo test --workspace
```

### Running Locally

```bash
# Terminal 1: Start the server
ANTHROPIC_API_KEY=sk-ant-... ./target/debug/cade-server

# Terminal 2: Start the CLI
./target/debug/cade
```

## Project Structure

CADE is a Cargo workspace with sixteen crates plus the root package. See [ARCHITECTURE.md](ARCHITECTURE.md)
for full details.

```
src             Root package: `cade` and `cade-server` binary entry points
cade-core       Shared types (permissions, settings, skills, hooks, toolsets)
cade-ai         LLM providers and model catalogue
cade-api-types  Shared API schemas and response/request types
cade-desktop    Desktop extensions (screen capture, window control) — cross-platform
cade-store      SQLite persistence + AES-GCM crypto + optional embedding search
cade-server     HTTP API server + consolidation pipeline
cade-agent      REST client, tool implementations, MCP, subagents
cade-cli        Terminal UI (Ratatui) + REPL + headless mode
cade-mcp        MCP server integration
cade-web        Web search and scraping
cade-tui        Standalone TUI component library
cade-plugin     Plugin loading and manifests
cade-sdk        Rust SDK for programmatic agent control
cade-ide-mcp    IDE MCP bridge (editor integrations)
cade-askpass    SSH/GPG password prompt IPC server
cade-gui        WASM dashboard (egui/eframe)
```

### Dependency Graph (acyclic)

```
cade-core, cade-ai, cade-api-types, cade-ide-mcp, cade-askpass ← leaf crates (zero workspace deps)
cade-desktop → cade-core
cade-store   → cade-core, cade-ai
cade-mcp     → cade-core
cade-web     → cade-core
cade-tui     → cade-core
cade-gui     → cade-core, cade-api-types
cade-agent   → cade-core, cade-desktop(?), cade-mcp(?), cade-web(?)
cade-server  → cade-core, cade-ai, cade-store, cade-agent, cade-mcp(?)
cade-cli     → cade-core, cade-agent, cade-ai, cade-tui, cade-askpass
cade-plugin  → cade-core, cade-agent
cade-sdk     → cade-core, cade-agent
```

`(?)` = optional feature-gated dependency.

### Which Files to Edit

All live code is in `crates/`. The root `src/` directory contains only:
- `main.rs` — CLI entry point
- `lib.rs` — Re-exports from workspace crates
- `bin/cade-server.rs` — Server entry point

## Guidelines

### Code Style

- Follow standard Rust idioms (`clippy`, `rustfmt`)
- Error handling: use each crate's local `error.rs` pattern (`Error` enum + `Result<T>` alias, usually with `derive_more::{Display, From}`); do not introduce `anyhow` or `thiserror` unless the crate already uses it or the change is explicitly approved
- Async: Tokio runtime, `async/await` throughout
- Naming: snake_case for functions/variables, PascalCase for types

### Commit Messages

Use conventional commit format:

```
feat(ui): add Tab path completion
fix(server): clear stale cancel_turn on Event::Open
refactor: extract LLM providers into cade-ai crate
docs: update ARCHITECTURE.md for workspace split
```

### Testing

- Run `cargo test --workspace` before submitting changes
- Add unit tests in `#[cfg(test)] mod tests` blocks
- Integration tests go in `tests/`

### Pull Requests

1. Fork and create a feature branch
2. Make your changes
3. Ensure `cargo build` and `cargo test --workspace` pass with zero warnings
4. Write a clear PR description explaining the change and its motivation
5. Reference any related issues

## Reporting Issues

- **Bugs**: Include reproduction steps, expected vs actual behavior, and logs
- **Security**: See [SECURITY.md](SECURITY.md) for responsible disclosure
- **Features**: Describe the use case and proposed API/behavior

## Architecture

See [ARCHITECTURE.md](ARCHITECTURE.md) for the full dependency graph, module
descriptions, data flow diagrams, memory/consolidation pipeline, and
cross-platform support matrix.

## License

CADE is dual-licensed under MIT and Apache-2.0. Contributions are accepted
under the same terms.
