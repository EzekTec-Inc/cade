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
├── cade-store/                  # SQLite + AES-GCM crypto       (→ core, ai)
├── cade-server/                 # HTTP API + consolidation       (→ core, ai, store)
├── cade-agent/                  # client + tools + MCP + subagents (→ core, desktop)
├── cade-cli/                    # CLI + REPL + headless mode     (→ core, agent, ai)
├── cade-mcp/                    # MCP server integration
├── cade-web/                    # web search + scraping
├── cade-tui/                    # standalone TUI component library
├── cade-plugin/                 # plugin loading
├── cade-sdk/                    # Rust SDK for programmatic control
├── cade-ide-mcp/                # IDE bridge (Neovim, VS Code, JetBrains)
└── cade-gui/                    # WASM dashboard (eframe/egui)

extensions/
├── cade-neovim/                 # Neovim adapter
├── cade-vscode/                 # VS Code adapter
└── cade-jetbrains/              # JetBrains adapter
```

## Process model

CADE runs as **two processes**:

```
┌──────────────┐    HTTP/JSON + SSE    ┌────────────────┐
│ cade  (CLI)  │ ◀───────────────────▶ │ cade-server    │
│ Ratatui TUI  │                       │ Axum + SQLite  │
└──────────────┘                       └────────────────┘
                                               │
                                               ├─ LLM providers (cade-ai)
                                               ├─ MCP servers (stdio / TCP)
                                               └─ Tool execution backend
                                                   (local / Docker / SSH)
```

The CLI is thin — input handling, rendering, settings. All state
(messages, memory, tool history, pricing) lives in the server's SQLite DB.
A second consumer, the WASM **dashboard** at `/dashboard`, talks to the
same REST API.

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
- `checkpoints` (git stash refs + commit hashes)
- `artifacts` (logs, diffs, fetched docs)
- `providers` (encrypted API keys via AES-GCM)
- `runs` (background mode)

The DB key lives at `~/.cade/db.key` (also re-derivable from
`CADE_DB_KEY` or `CADE_MACHINE_SECRET`). Path protection in
`cade-core::permissions::rules` denies writes here even in YOLO mode.

## REST API surface (selected)

| Method | Path | Purpose |
|---|---|---|
| GET / POST / DELETE | `/v1/agents` | List / create / delete agents |
| PATCH | `/v1/agents/:id` | Update model, compaction-model, etc. |
| GET / PUT | `/v1/agents/:id/memory` | Read / write memory blocks |
| POST | `/v1/agents/:id/run` (SSE) | Agentic loop with streaming |
| POST | `/v1/agents/:id/messages/stream` (SSE) | Single-completion stream |
| GET / POST / DELETE | `/v1/agents/:id/conversations` | Conversation management |
| GET / POST | `/v1/agents/:id/checkpoints` | Checkpoint create / list |
| GET | `/v1/agents/:id/skills` | Available skills |
| POST | `/v1/agents/:id/skills/load` `unload` `enable` `disable` | Skill mutation |
| POST | `/v1/agents/:id/tool_executions` | Log a tool call |
| GET | `/v1/runs/:id` `/v1/runs/:id/stream` | Background run status |
| GET / POST / DELETE | `/v1/providers` | LLM provider keys |
| GET | `/v1/health` `/v1/config` | Server health |

All inference routes are rate-limited via `rate_limit_middleware`. The
`/dashboard` route is unauthenticated and serves the WASM bundle.
