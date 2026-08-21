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
├── cade-core/                   # shared types, CapabilityMesh seam, permissions, settings, skills, hooks
├── cade-ai/                     # LLM providers + model catalogue + ITS + prompt caching
├── cade-desktop/                # DesktopCommander seam (screen capture, window control, notify)
├── cade-store/                  # SQLite + AES-GCM crypto + embeddings (→ core, ai)
├── cade-server/                 # HTTP API + Sleeptime memory consolidation (→ core, ai, store)
├── cade-server-bin/             # Standalone server daemon binary entrypoint
├── cade-agent/                  # SubagentSession harness + tools manager + backends
├── cade-cli/                    # CLI + REPL + headless mode (→ core, agent, ai)
├── cade-mcp/                    # Model Context Protocol integration & stream health
├── cade-web/                    # web search + scraping
├── cade-tui/                    # standalone TUI component library (Ratatui v0.30)
├── cade-plugin/                 # PluginEngine seam, package discovery, tarball installer
├── cade-sdk/                    # Rust SDK for in-process and remote programmatic control
├── cade-ide-mcp/                # IDE bridge (Neovim, VS Code, JetBrains)
├── cade-askpass/                # SSH/GPG password prompt (IPC, token auth)
└── cade-gui/                    # WASM dashboard (Dioxus v0.5) with rich MarkdownView

plugins/
└── cade.nvim/                   # Neovim plugin and IDE bridge
```

## Process model & SDK runtime

CADE supports two execution topologies:

1. **Interactive Client/Daemon Model**:
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

2. **In-Process Zero-Daemon Embedded Model (`cade-sdk`)**:
```
┌────────────────────────────────────────────────────────┐
│ Your Rust Application / CLI / Microservice / Lambda    │
│                                                        │
│  EmbeddedSession / TeamSession (`cade-sdk`)            │
│  ├── In-Memory / Local SQLite (`cade-store`)           │
│  ├── Direct LLM Routing (`cade-ai`)                    │
│  ├── Native Tool Runtime (`cade-agent`)                │
│  └── Strongly-Typed Reactive Stream (`CadeStreamEvent`)│
└────────────────────────────────────────────────────────┘
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

| Subsystem | Crate / Module | Doc |
|---|---|---|
| In-Process Embedded & Team SDK | `cade-sdk` | [crates/cade-sdk/README.md](../crates/cade-sdk/README.md) |
| Capability Mesh Seam | `cade-core::capabilities::mesh` | [ADR-0020](adr/0020-capability-mesh-unified-execution-seam.md) |
| Memory Distillation Engine | `cade-server::server::consolidation` | [memory-system.md](memory-system.md) |
| Permissions & Consent Governor | `cade-core::permissions` | [permissions.md](permissions.md) |
| SubagentSession Harness | `cade-agent::subagents::session` | [subagents.md](subagents.md) |
| Hook engine | `cade-core::hooks` | [hooks.md](hooks.md) |
| Skill discovery + loading | `cade-core::skills`, `cade-server` | [skills.md](skills.md) |
| MCP integration & Stream Health | `cade-mcp`, `cade-agent::mcp` | [mcp-servers.md](mcp-servers.md) |
| Desktop Commander Seam | `cade-desktop::desktop::commander` | [desktop-commander.md](desktop-commander.md) |
| Plugin Engine Host | `cade-plugin::engine` | [plugin-development.md](plugin-development.md) |
| Intelligent Tool Selection | `cade-ai::its` | [intelligent-tool-selection.md](intelligent-tool-selection.md) |
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
6. **Decoupled Embedding & Vector Indexes**: Exposes abstract `Embedder` and `VectorIndex` traits to decouple embedding generation and vector search from tight local SQLite coupling. Includes production-ready, feature-gated client adapters for **PostgreSQL (`pgvector` via `tokio-postgres`)** and **Qdrant (`qdrant-client` SDK)**, providing high-performance, enterprise-grade vector store support.
7. **Hybrid Compile-Time Tools**: Leverages strongly-typed `BuiltInTool` and `CoreToolAdapter` traits to compile-time wrap CADE's own high-performance local tools, running them with zero-copy serialization alongside CADE's dynamic Model Context Protocol (MCP) server dispatch loop.
8. **Stateful TUI Autocomplete Controller**: Extends the `OverlayComponent` trait with the type-safe `as_any_mut` upcasting pattern, enabling the TUI's active `AutocompleteOverlay` to intercept editor keystrokes and dynamically re-filter suggestion lists on-the-fly as the user types.
9. **Schema-Validated Structured Completions**: Introduces the `complete_structured` trait method to standardise structured LLM outcomes matching a strict JSON schema, using `clean_json_markers` to cleanly strip markdown block backticks and ensure 100% deterministic parsing.
10. **Lightweight Virtual Sandboxing**: Adds a secure `VirtualSandboxBackend` that isolates process environments and performs watertight path canonicalization to enforce directory boundary checks, blocking sandbox escape vectors.
11. **Deep Shell Execution Engine (`ShellExecutionEngine`)**: Encapsulates OS binary resolution, environment variable merging (`agent_env`, `askpass`), process lifecycle, and 25%/75% head-tail middle-truncation behind a clean 2-method seam. Includes a `WindowsPortableShellAdapter` that auto-detects Git-Bash, WSL, and MSYS2 to allow agents to run POSIX scripts on Windows without syntax failures.
12. **Multi-OS Desktop Automation Adapter**: Deepens `DesktopControl` with native, OS-level window focusing across Windows (PowerShell COM `AppActivate`), macOS (AppleScript `osascript`), and Linux (`wmctrl`/`xdotool`), backed by zero-panic `try_detect()` error handling for headless CI environments.
13. **Structured Tool Execution & Recovery**: Introduces `StructuredToolOutput` (`stdout`, `stderr`, `exit_code`, `duration_ms`, `truncated`, `error_summary`) to supply structured execution and compiler error metadata directly to LLM recovery routines.
14. **Structured Git Branch Sandboxing**: Extends `IsolatedWorkspace` with a `MergeConflictReport` engine that inspects `git status --porcelain` on merge failures, returning exact lists of unmerged files (`UU`, `AA`, `DD`) and executing automatic `merge --abort` rollbacks.
15. **Authenticated Startup Verification & Cold-Start Re-Sync**: On startup, the CLI client performs an authenticated handshake (`verify_auth()`) hitting `GET /v1/config` with Bearer auth. If a cold-start daemon generation occurs during `auto_start_server`, the client re-syncs its Bearer token directly from `~/.cade/api-token` before dispatching agent requests, completely eliminating authentication desynchronization errors.
16. **CapabilityMesh Unified Execution Seam (ADR-0020)**: Unifies built-in native tools, external MCP processes, and skills behind a single trait seam (`execute` + `active_catalog`). Encapsulates schema injection, ITS tag decoration (`["cade"]`, `["mcp"]`, `["core_mcp"]`), and error taxonomy (`NotFound`, `PermissionDenied`, `Disconnected`, `ExecutionFailed`).
17. **SubagentSession Autonomous Harness (ADR-0021)**: Decoupled autonomous runner in `cade-agent` with canonical `finish(status, summary)` tool injection, dual bounds (`max_iters` + `max_tokens_budget`), and structured `SubagentOutcome` (`Done`, `Blocked`, `Failed`, `Exhausted`).
18. **RAII Workspace Isolation (`IsolatedWorkspaceGuard`)**: Manages temporary sandbox directories and git worktrees with automatic atomic merge on success and zero-residue pruning on Drop.
19. **Real-Time Subagent Telemetry (`SubagentEventEmitter`)**: Streams typed events (`TurnStarted`, `ToolExecuting`, `Progress`, `ApprovalRequired`, `Finished`) asynchronously without blocking execution loops.
20. **Unified DesktopCommander & PluginEngine Seams**: Cross-platform automation abstraction for screen capture and window control in `cade-desktop`, alongside deep package discovery, tarball extraction, and manifest validation in `cade-plugin`.
21. **In-Process Zero-Daemon & Multi-Agent Squad Runtime (`cade-sdk`)**: Provides `EmbeddedSession` linking directly to `cade-store` SQLite and `cade-ai` `LlmRouter` in-process with zero background daemon overhead, paired with `TeamSession` programmatic squad orchestration and `CadeStreamEvent` strongly-typed reactive streams.

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
