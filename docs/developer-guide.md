# CADE Developer Guide

Welcome to the CADE developer documentation! This guide is intended for contributors who want to understand CADE's architecture and contribute to the codebase.

## 1. Project Architecture

CADE is a Rust Monorepo structured via Cargo Workspaces:
- **`cade-core`**: Core types, capabilities, settings, and shared definitions.
- **`cade-store`**: Central SQLite database layer. Manages state, RAG embeddings, typed memory blocks, and provenance.
- **`cade-server`**: The Axum HTTP server that powers the LLM agentic loop, tool dispatching, and background consolidation.
- **`cade-agent`**: Tool execution environment and client SDK.
- **`cade-cli` / `cade-tui`**: The interactive Terminal User Interface (Ratatui + Crossterm).
- **`cade-gui`**: Experimental Egui-based graphical interface.
- **`cade-web`**: Web search and fetching capabilities (Brave Search, DuckDuckGo).
- **`cade-askpass`**: IPC password prompt helper for sudo/ssh.

## 2. Development Standards (rust10x)

CADE strictly adheres to high-performance and safe Rust patterns:
- **Zero Panic Policy:** `unwrap()` and `expect()` are **strictly forbidden** in production code. Always use `anyhow::Result` and propagate errors with `?`.
- **Newtype Pattern:** Use strongly typed wrappers for IDs (e.g., `AgentId`, `ConversationId`) to prevent domain logic bugs.
- **Error Handling:** Use `anyhow::Context` to attach meaningful context as errors bubble up.

## 3. The Memory System

CADE uses a sophisticated 3-tier memory system backed by SQLite and vector embeddings (`fastembed` + `sqlite-vec`):
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
