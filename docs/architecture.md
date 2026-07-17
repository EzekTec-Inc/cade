# Architecture

CADE is a Cargo workspace. Each crate has a single, well-defined role and
the dependency graph is acyclic.

## Workspace layout

```
src/
├── main.rs                      # `cade` CLI entry point
├── lib.rs                       # re-exports workspace crates as cade::*
└── bin/cade-server.rs           # `cade-server` entry point

crates/
├── cade-core/                   # shared types — leaf, no workspace deps
├── cade-ai/                     # LLM providers + model catalogue — leaf
├── cade-desktop/                # OS extensions (capture, control, notify) — leaf
├── cade-store/                  # SQLite + AES-GCM crypto + embeddings (→ core, ai)
├── cade-server/                 # HTTP API + consolidation       (→ core, ai, store)
├── cade-agent/                  # client + tools + MCP + subagents (→ core, desktop)
├── cade-cli/                    # CLI + REPL + headless mode     (→ core, agent, ai)
├── cade-mcp/                    # MCP server integration
├── cade-web/                    # web search + scraping
├── cade-tui/                    # standalone TUI component library
├── cade-plugin/                 # plugin loading
├── cade-sdk/                    # Rust SDK for programmatic control
├── cade-ide-mcp/                # IDE bridge (Neovim, VS Code, JetBrains)
├── cade-askpass/                # SSH/GPG password prompt (IPC, token auth)
└── cade-gui/                    # WASM dashboard (Dioxus v0.5)

plugins/
└── cade.nvim/                   # Neovim plugin and IDE bridge
```

## Process model

CADE runs as **two primary processes** for interactive use:

```
┌──────────────┐    HTTP/JSON + SSE    ┌────────────────┐
│ cade  (CLI)  │ ◀───────────────────▶ │ cade-server    │
│ Ratatui TUI  │                       │ Axum + SQLite  │
└──────────────┘                       └────────────────┘
                                               │
                                               ├─ LLM providers (cade-ai)
                                               ├─ MCP servers (stdio / HTTP)
                                               └─ Tool execution backend
                                                   (local / Docker / SSH)
```

Other frontends, including the WASM dashboard at `/dashboard` and IDE
integrations, talk to the same server/API surface.

## Data flow — agentic turn

1. User types a message; CLI calls `POST /v1/agents/:id/run` (stream).
2. Server enters the **agentic loop** in `cade-server::server::api::run`:
   - Build context: pinned + short-term memory + recent turns + skills.
   - Call the LLM (`cade-ai::providers::*`) with the full toolset.
   - Stream `text` / `tool_call` / `tool_result` / `finish` events to the CLI.
   - Dispatch every tool call via `cade-agent::tools::manager::dispatch`.
   - Loop until the LLM emits `finish` or `MAX_TURNS=20` is hit.
3. Each tool result is persisted to `tool_executions` with `output_chars`
   and an Unicode-correct character count for cost telemetry.
4. After the turn, the server may run **Sleeptime consolidation** if the
   context window is ≥ 98% full — see [memory-system.md](memory-system.md).

## Subsystems

| Subsystem | Crate | Doc |
|---|---|---|
| Memory blocks + consolidation | `cade-server::server::consolidation` | [memory-system.md](memory-system.md) |
| Permissions + path protection | `cade-core::permissions` | [permissions.md](permissions.md) |
| Hook engine | `cade-core::hooks` | [hooks.md](hooks.md) |
| Skill discovery + loading | `cade-core::skills`, `cade-server` | [skills.md](skills.md) |
| Subagent runner | `cade-agent::subagents` | [subagents.md](subagents.md) |
| MCP integration | `cade-mcp`, `cade-agent::mcp` | [mcp-servers.md](mcp-servers.md) |
| Intelligent Tool Selection | `cade-agent::tools::its` | [intelligent-tool-selection.md](intelligent-tool-selection.md) |
| Cost / pricing registry | `cade-ai::ModelRegistry` | [cost-and-pricing.md](cost-and-pricing.md) |
| Execution backends | `cade-agent::backends` | [execution-backends.md](execution-backends.md) |

## Persistence

`cade-store` owns the SQLite schema. Migrations are tracked via
`PRAGMA user_version`. The current schema covers:

- `agents`, `conversations`, `messages`
- `memory_blocks` (pinned/short/long-term tier)
- `tool_executions` (with `output_chars` for cost telemetry)
- `checkpoints` (git commit hashes)
- `artifacts` (logs, diffs, fetched docs)
- `providers` (encrypted API keys via AES-GCM)
- `runs` (background mode)
- `observations` (tool call capture with importance scoring)
- `vec_memory_blocks`, `vec_archival_memory`, `vec_messages` — `sqlite-vec` virtual tables for embedding-based semantic search (Migration 8; populated only when the `semantic-search` feature is enabled)
- `knowledge_edges` — centralized knowledge graph triples (`entity`, `relation`, `target`) with binary vector embeddings (Migration 16)

## Diagnostics, Concurrency & Safety

CADE utilizes robust, production-grade systems to ensure zero-panic stability, smooth rendering, and concurrent execution safety:
1. **Global Panic Hooks**: Register custom hooks on both client TUI and backend server to write detailed backtraces and context to `~/.cade/crash.log` before aborting, preventing silent exits.
2. **Concurrent Database Safety**: Connection pools configure `PRAGMA busy_timeout = 5000;` so SQLite can safely queue concurrent read/write queries for up to 5 seconds during parallel executions.
3. **Centralized HTTP Connection Pooling**: Standardizes and pools outgoing connections across all first-party providers (`OpenAiProvider`, `AnthropicProvider`, `GeminiProvider`), utilizing a unified HTTP client built with standard keepalive (60s), connection timeout (15s), and stream timeout (120s) configurations to optimize connection reuse.
4. **File-Watcher Debouncing**: Applies a `150ms` debouncer on live reloads (settings, skills, plugins) to prevent thrashing and infinite loops during fast development/compile cycles.
5. **Cassette-Based (VCR) Mock Testing**: Integrates the `VcrCassette` recorder/player middleware to record actual LLM HTTP requests and replay them offline deterministically, keeping CADE's integration test suite isolated, offline, and cost-free.
6. **Decoupled Embedding & Vector Indexes**: Exposes abstract `Embedder` and `VectorIndex` traits to decouple embedding generation and vector search from tight local SQLite coupling, providing an easy path for future enterprise-grade, distributed vector backends (e.g. Qdrant, PGVector).
7. **Hybrid Compile-Time Tools**: Leverages strongly-typed `BuiltInTool` and `CoreToolAdapter` traits to compile-time wrap CADE's own high-performance local tools, running them with zero-copy serialization alongside CADE's dynamic Model Context Protocol (MCP) server dispatch loop.
8. **Stateful TUI Autocomplete Controller**: Extends the `OverlayComponent` trait with the type-safe `as_any_mut` upcasting pattern, enabling the TUI's active `AutocompleteOverlay` to intercept editor keystrokes and dynamically re-filter suggestion lists on-the-fly as the user types.
9. **Schema-Validated Structured Completions**: Introduces the `complete_structured` trait method to standardise structured LLM outcomes matching a strict JSON schema, using `clean_json_markers` to cleanly strip markdown block backticks and ensure 100% deterministic parsing.
10. **Lightweight Virtual Sandboxing**: Adds a secure `VirtualSandboxBackend` that isolates process environments and performs watertight path canonicalization to enforce directory boundary checks, blocking sandbox escape vectors.

The DB key lives at `~/.cade/db.key` (also re-derivable from
`CADE_DB_KEY` or `CADE_MACHINE_SECRET`). Path protection in
`cade-core::permissions::rules` denies writes here even in YOLO mode.

## AST-Based Code Modification & Serena Rules

To prevent syntax errors, corrupt diffs, and preserve strict project conventions, CADE supports and enforces **AST-based (Abstract Syntax Tree) code modifications** over raw text-based string replacements (such as generic regex edits or basic `edit_file` tools).

### The Serena AST Engine
CADE integrates with the **Serena Agent AST Engine** to parse, inspect, and mutate codebase symbols. Serena provides structural mutations including:
- `serena__replace_content`: Replaces structural code fragments using AST boundaries.
- `serena__insert_after_symbol`: Appends new code immediately after structural symbols (e.g. after a struct, function, or enum definition) safely without breaking curly braces or parentheses.
- `serena__replace_symbol_body`: Surgically replaces the inner block of a structural symbol while preserving its signature and doc comments.

### Enforcement
When working on registered coding projects, local PreToolUse hooks enforce that any modifications to source files (e.g. `*.rs`, `*.ts`, `*.js`, `*.py`, `*.lua`) must bypass generic text editors. The agent is forced to use Serena AST-based tools to perform clean, parseable syntax trees mutations.

---

## REST API surface (selected)

| Method | Path | Purpose |
|---|---|---|
| GET / POST / DELETE | `/v1/agents` | List / create / delete agents |
| PATCH | `/v1/agents/:id` | Update model, compaction-model, etc. |
| GET / PUT | `/v1/agents/:id/memory` | Read / write memory blocks |
| POST | `/v1/agents/:id/run` (SSE) | Agentic loop with streaming |
| POST | `/v1/agents/:id/messages/stream` (SSE) | Single-completion stream |
| POST | `/v1/agents/:id/edit` (SSE) | Interactive IDE hover-edits |
| POST | `/v1/agents/:id/complete` (SSE) | IDE inline ghost-text completions |
| GET / POST / DELETE | `/v1/agents/:id/conversations` | Conversation management |
| GET / POST | `/v1/agents/:id/checkpoints` | Checkpoint create / list |
| GET | `/v1/agents/:id/skills` | Available skills |
| POST | `/v1/agents/:id/skills/load` `unload` `enable` `disable` | Skill mutation |
| POST | `/v1/agents/:id/tool_executions` | Log a tool call |
| POST | `/v1/agents/:id/links` | Sync and re-attach tools to session |
| GET / POST / DELETE | `/v1/mcp/servers` | Manage MCP servers |
| GET / POST | `/v1/backends` | Manage execution backends |
| POST | `/v1/workflows/:workflow_name` | Webhook workflow dispatch loop with payload injection |
| GET | `/v1/runs/:id` `/v1/runs/:id/stream` | Background run status |
| GET / POST / DELETE | `/v1/providers` | LLM provider keys |
| GET | `/v1/health` `/v1/config` | Server health |

All inference routes are rate-limited via `rate_limit_middleware`. The
`/dashboard` route is unauthenticated and serves the WASM bundle.
