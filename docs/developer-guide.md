# CADE Developer Guide

Welcome to the CADE developer documentation! This guide is intended for contributors who want to understand CADE's architecture and contribute to the codebase.

## 1. Project Architecture

CADE is a Rust workspace structured via Cargo workspaces. The root package owns the CLI/server binaries, and the `crates/` directory contains the reusable components:
- **`cade-core`**: Core types, capabilities, settings, and shared definitions.
- **`cade-ai`**: LLM providers and model catalogue.
- **`cade-api-types`**: Shared API request/response schemas.
- **`cade-store`**: Central SQLite database layer. Manages state, optional RAG embeddings, typed memory blocks, and provenance.
- **`cade-server`**: The Axum HTTP server that powers the LLM agentic loop, tool dispatching, and background consolidation.
- **`cade-agent`**: Tool execution environment, MCP integration, execution backends, and client-side agent loop helpers.
- **`cade-cli` / `cade-tui`**: The interactive Terminal User Interface (Ratatui + Crossterm).
- **`cade-gui`**: Egui/eframe WASM dashboard.
- **`cade-mcp` / `cade-ide-mcp`**: MCP client/server integration and IDE-state bridge.
- **`cade-web`**: Web search and fetching capabilities.
- **`cade-plugin` / `cade-sdk`**: Plugin manifests/loading and programmatic Rust SDK.
- **`cade-desktop` / `cade-askpass`**: Desktop extensions and IPC password prompt helper.

## 2. Development Standards (rust10x)

CADE strictly adheres to high-performance and safe Rust patterns:
- **Zero Panic Policy:** `unwrap()` and `expect()` are **strictly forbidden** in production code unless an invariant is provably safe and documented. Propagate errors with `?`.
- **Newtype Pattern:** Use strongly typed wrappers for IDs (e.g., `AgentId`, `ConversationId`) to prevent domain logic bugs.
- **Error Handling:** Follow each crate's local `error.rs` convention: a crate-specific `Error` enum, `Result<T>` type alias, and `derive_more::{Display, From}` for conversions. Attach context by mapping to the local error type instead of introducing a new error crate.

## 3. The Memory System

CADE uses a tiered memory system backed by SQLite and optional vector embeddings (`fastembed` + `sqlite-vec` when built with `--features semantic-search`):
1. **Short-Term Context:** Recent messages in the active window.
2. **Archival / Semantic Memory:** Older turns are background-summarized (consolidated). Durable facts are automatically extracted, typed (`decision`, `convention`), and assigned confidence scores.
3. **Provenance:** Every fact in the database tracks exactly which turn and which tool created it.
4. **Decay:** Unused semantic memories slowly decay in confidence over time.

When developing features that touch state, always interact via `cade-store` methods, never raw SQL queries in the application layer.

## 4. Subagent Execution

Subagents are isolated execution loops:
- They run via `SubagentExecutor` in `cade-server`.
- They operate inside an `EphemeralEnvironment` (a temporary database row) to prevent polluting the parent agent's state.
- Upon completion, their findings are intelligently merged into the parent's memory via an LLM merge pass (`smart_memory_merge`), preserving typed metadata.

## 5. Testing and Validation

CADE mandates a strict Test-Driven Development (TDD) loop.
- Write a failing test first.
- Make the minimal change required.
- Refactor.

**Before submitting a PR, you MUST pass all workspace checks:**
```bash
cargo build --workspace
cargo clippy --workspace -- -D warnings
cargo test --workspace
cargo fmt --all -- --check
```
CADE enforces a zero-compiler-warning policy.

## 6. Extending with MCP Servers

CADE integrates with external Model Context Protocol (MCP) servers. If you are adding new tool capabilities, consider whether they should be natively integrated into CADE's codebase or built as an external MCP server (e.g., `cade-desktop`, `serena`). 

To test MCP servers, configure them in your local `.cade/settings.local.json`.
