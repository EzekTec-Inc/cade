# Architecture of CADE

CADE (Coding AI assistant with Desktop Extensions) is a stateful, self-improving
AI coding agent that runs in the user's terminal. It gives an AI agent full access
to the local development environment — shell, filesystem, desktop — and ships its
own server, requiring no external platform.

---

## Workspace Layout

CADE is a Cargo workspace with twelve independent crates plus a root package that
owns the two binaries (`cade` and `cade-server`).

```
CADE/
├── Cargo.toml              # Workspace root + binary package
├── src/
│   ├── main.rs             # `cade` CLI entry point
│   ├── lib.rs              # Re-exports workspace crates as `cade::*`
│   └── bin/
│       └── cade-server.rs  # `cade-server` entry point
├── crates/
│   ├── cade-agent/         # Client, tools, subagents
│   ├── cade-ai/            # LLM providers & model catalogue
│   ├── cade-cli/           # CLI orchestrator, headless mode
│   ├── cade-codeintel/     # Code intelligence tools
│   ├── cade-core/          # Shared types, toolsets, hooks
│   ├── cade-desktop/       # Desktop extensions (xcap, xdotool)
│   ├── cade-mcp/           # MCP client integration
│   ├── cade-plugin/        # WASM/dylib plugin system
│   ├── cade-sdk/           # Developer SDK
│   ├── cade-server/        # HTTP API + SQLite storage
│   ├── cade-tui/           # Ratatui rendering engine
│   └── cade-web/           # Web scraping and HTTP tools
└── tests/                  # Integration tests
```

## Dependency Graph

```
cade-core       (standalone — permissions, settings, skills, hooks, toolsets)
cade-ai         (standalone — LLM providers, catalogue, retry)
cade-desktop    (standalone — screen capture, window control, notifications, tray)

cade-server     → cade-core, cade-ai
cade-agent      → cade-core, cade-desktop
cade-cli        → cade-core, cade-agent, cade-ai

cade (root)     → all crates (re-exports for binaries)
```

The graph is **strictly acyclic**. Three leaf crates (`cade-core`, `cade-ai`,
`cade-desktop`) have zero dependencies on other workspace crates, making them
independently compilable and testable.

---

## Crate Responsibilities

### `cade-core`

Shared types used across the workspace. No crate dependencies.

| Module | Purpose |
|--------|---------|
| `permissions/` | `PermissionManager`, `PermissionMode` enum, allow/deny rules |
| `settings/` | `SettingsManager`, global/local config, MCP server config |
| `skills/` | Skill discovery, SKILL.md parsing, skill lifecycle |
| `hooks/` | `HookEngine` — lifecycle hooks (PreToolUse, PostToolUse, Stop, etc.) |
| `toolsets/` | `Toolset` enum — Default / Codex / Gemini tool families |

### `cade-ai`

LLM provider abstraction and model routing. No crate dependencies.

| Module | Purpose |
|--------|---------|
| `lib.rs` | `LlmProvider` trait, `LlmRouter`, `CompletionRequest/Response`, `StreamChunk`, `TokenUsage`, `LlmMessage`, `AiConfig`, retry logic, preset providers |
| `anthropic.rs` | Anthropic/Claude provider (extended thinking, streaming) |
| `openai.rs` | OpenAI provider (Chat Completions + Responses API) |
| `gemini.rs` | Google Gemini provider (thought signatures, vision) |
| `ollama.rs` | Local Ollama provider (delegates to OpenAI-compatible API) |
| `catalogue.rs` | Static model catalogue, context windows, pricing |

### `cade-desktop`

Desktop integration extensions. No crate dependencies.

| Module | Purpose |
|--------|---------|
| `capture.rs` | Screen capture via `xcap` → base64 PNG |
| `control.rs` | Window focus, text typing, key presses, mouse control (`xdotool`/`ydotool`) |
| `notify.rs` | OS desktop notifications via `notify-rust` |
| `tray.rs` | System tray icon via `ksni` |

### `cade-server`

HTTP API server and persistence layer. Depends on `cade-core` and `cade-ai`.

| Module | Purpose |
|--------|---------|
| `api/mod.rs` | Axum router with all REST endpoints |
| `api/agents.rs` | Agent CRUD, tools attachment, memory blocks, conversations |
| `api/messages.rs` | Message send/stream, context building, auto-compaction |
| `api/providers.rs` | LLM provider management (add/remove/list) |
| `api/models.rs` | Live model listing from all providers |
| `api/runs.rs` | Background run status/streaming |
| `api/tools.rs` | Tool registration |
| `api/auth.rs` | Bearer token authentication middleware |
| `api/health.rs` | Health check and server config endpoints |
| `config.rs` | `ServerConfig` from env vars, provider auto-detection |
| `state.rs` | `AppState` — shared Axum state (DB, LLM, config) |
| `storage/sqlite.rs` | SQLite persistence (agents, messages, tools, providers, memory) |
| `crypto.rs` | AES-256-GCM encryption for sensitive data at rest |
| `rate_limit.rs` | Per-agent token-bucket rate limiter |

### `cade-agent`

Agent client, tool implementations, MCP, and subagents. Depends on `cade-core` and `cade-desktop`.

| Module | Purpose |
|--------|---------|
| `agent/client.rs` | `CadeClient` — REST client for cade-server, SSE streaming |
| `agent/session.rs` | Per-directory session persistence |
| `agent/tools.rs` | Tool registration with the server |
| `tools/manager.rs` | Tool dispatch registry |
| `tools/bash.rs` | Shell execution (streaming output) |
| `tools/fs.rs` | Read/Write/Edit/ApplyPatch with sandbox support |
| `tools/search.rs` | Grep and glob search |
| `tools/desktop.rs` | Desktop tool wrappers |
| `tools/ask.rs` | `ask_user_question` tool |
| `tools/plan.rs` | Planning tools (EnterPlanMode, Todos, etc.) |
| `mcp/` | MCP client — spawn and manage local MCP servers |
| `subagents/` | Subagent runner (spawn ephemeral agents for parallel tasks) |

### `cade-cli`

Terminal UI and REPL. Depends on `cade-core`, `cade-agent`, and `cade-ai`.

| Module | Purpose |
|--------|---------|
| `cli/repl.rs` | Interactive REPL, slash commands, tool execution loop, streaming |
| `cli/headless.rs` | Headless `-p` mode for CI/scripting |
| `cli/args.rs` | CLI argument parsing (clap) |
| `cli/export_import.rs` | Agent export/import |
| `ui/app.rs` | Main TUI application (`TuiApp`), Ratatui rendering, key handling |
| `ui/editor.rs` | `Editor` component — text buffer, undo/redo, bracketed paste |
| `ui/component.rs` | `Component` trait for unified render/input interface |
| `ui/autocomplete.rs` | Tab path completion, `@` fuzzy file picker |
| `ui/markdown.rs` | Markdown → Ratatui spans (pulldown-cmark AST parser) |
| `ui/question.rs` | Interactive question/approval prompts |
| `ui/menu.rs` | `/help` menu system |
| `ui/skills.rs` | `/skills` browser overlay |

---

## Key Data Flows

### User Message → LLM Response

```
User input → TuiApp → Repl::agent_turn()
  → CadeClient::stream_message_cancellable()        [HTTP SSE to cade-server]
    → cade-server: build_context() + LlmProvider::stream()  [LLM API call]
    → SSE chunks back to CLI
  → UI consumer task → TuiApp::push_streaming_chunk()  [throttled ~60 FPS]
  → Tool calls? → dispatch() → execute → stream_tool_return_cancellable()
  → Loop until no more tool calls
```

### Streaming Architecture (R-01..R-04)

```
SSE token → on_event → ui_tx.send()    [non-blocking channel send, ~0µs]
                           ↓
UI task   → ui_rx.recv() → app.lock() → draw_throttled()  [max ~60 FPS]
```

Network I/O is fully decoupled from TUI rendering. The tick task handles
input events and thinking animations on a separate loop.

---

## Security Model

- Bash commands require explicit approval (unless `--yolo`)
- File tools respect optional `CADE_FS_ROOT` sandbox
- `apply_patch` validates paths against traversal attacks
- Headless output is sanitized against ANSI injection
- Server auth via Bearer token with constant-time comparison
- Encryption at rest for sensitive DB fields (AES-256-GCM)
- Settings files created with 0600 permissions

See [SECURITY.md](SECURITY.md) for full details.

## Capability System

CADE organizes optional features into **capability packs** controlled by
**profiles**.

### Capabilities

| Capability | Description |
|---|---|
| `agentic` | Subagents, agent messaging, reflection, artifacts |
| `codeintel` | Tree-sitter indexing, symbol search, references |
| `desktop` | Screenshots, window control, notifications, tray |
| `web` | Web search, fetch docs, browser screenshot |
| `mcp` | MCP server management and external tools |
| `clipboard-images` | Image paste from clipboard |
| `syntax-highlighting` | Syntax highlighting in TUI |
| `advanced-memory` | Typed memory, evidence linking |
| `integration` | SDK, RPC, plugin embedding |

### Profiles

- **Core** — coding tools + memory + checkpoints
- **Pro** — Core + agentic + codeintel
- **Full** — everything (default)

### Resolution

Effective capabilities = profile baseline + `enable_capabilities` - `disable_capabilities`.

### Key files

- `crates/cade-core/src/capabilities/mod.rs` — `Capability`, `CapabilitySet`, `Profile`
- `crates/cade-agent/src/tools/catalog.rs` — capability-aware tool filtering
- `crates/cade-cli/src/cli/repl/capability_gate.rs` — command gating helpers
- `src/bootstrap/tools.rs` — `register_and_attach_with_caps()`
