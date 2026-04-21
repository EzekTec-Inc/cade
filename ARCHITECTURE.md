# CADE Architecture

This document describes the internal architecture of CADE — a stateful, self-improving Rust CLI coding agent.

---

## Workspace Overview

CADE is a Cargo workspace with **fifteen crates** and a root package that owns the two binaries (`cade` CLI and `cade-server`).

```
src/
├── main.rs                     # `cade` CLI entry point
├── lib.rs                      # Re-exports workspace crates as cade::*
└── bin/cade-server.rs          # `cade-server` entry point

crates/
├── cade-core/                  # Shared types (no workspace deps)
│   └── permissions, settings, skills, hooks, toolsets, capabilities,
│       resources (context files, palette, themes)
├── cade-ai/                    # LLM providers (no workspace deps)
│   └── anthropic, openai, gemini, ollama, model catalogue
├── cade-api-types/             # Shared API types (no workspace deps)
│   └── ChatMessage, AgentInfo, HealthInfo, tool schemas
├── cade-desktop/               # Desktop extensions (→ cade-core)
│   └── capture, control, notify — cross-platform (Linux/macOS/Windows)
├── cade-store/                 # SQLite persistence + crypto (→ cade-core, cade-ai)
│   └── sqlite/ (agents, messages, conversations, memory, tools,
│       providers, runs, evidence), crypto (AES-GCM encryption)
├── cade-server/                # HTTP API + consolidation (→ cade-core, cade-ai, cade-store, cade-agent)
│   └── api/, config, rate_limit, consolidation (sleeptime memory)
├── cade-agent/                 # REST client + tool implementations (→ cade-core, cade-desktop, cade-mcp, cade-web)
│   └── agent/ (client, session, consolidation), tools/, mcp/, subagents/
├── cade-cli/                   # TUI + REPL (→ cade-core, cade-agent, cade-ai, cade-tui)
│   └── cli/, ui/
├── cade-tui/                   # Standalone TUI component library (→ cade-core)
│   └── Ratatui-based render, timeline, layout, colors
├── cade-gui/                   # WASM dashboard (→ cade-core, cade-api-types)
│   └── egui/eframe app, SSE streaming, overlays, components
├── cade-mcp/                   # MCP (Model Context Protocol) server integration (→ cade-core)
├── cade-web/                   # Web search and scraping capabilities (→ cade-core)
├── cade-plugin/                # Plugin loading and manifests (→ cade-core, cade-agent)
├── cade-sdk/                   # Rust SDK for programmatic agent control (→ cade-core, cade-agent)
└── cade-ide-mcp/               # IDE MCP bridge (editor integrations)
```

---

## Dependency Graph (acyclic)

```
cade-core, cade-ai, cade-api-types, cade-ide-mcp   ← leaf crates (zero workspace deps)
cade-desktop → cade-core
cade-store   → cade-core, cade-ai
cade-mcp     → cade-core
cade-web     → cade-core
cade-tui     → cade-core
cade-gui     → cade-core, cade-api-types
cade-agent   → cade-core, cade-desktop(?), cade-mcp(?), cade-web(?)
cade-server  → cade-core, cade-ai, cade-store, cade-agent, cade-mcp(?)
cade-cli     → cade-core, cade-agent, cade-ai, cade-tui
cade-plugin  → cade-core, cade-agent
cade-sdk     → cade-core, cade-agent
```

`(?)` = optional feature-gated dependency. All dependencies flow downward. No circular references.

---

## Data Flow

### Agent Lifecycle

```
CLI (main.rs)
    │
    ├── bootstrap/agents.rs  → resolve_agent_and_conversation()
    │   Resolution order:
    │   1. CLI flags (--new-agent, --agent <id>, --name <query>)
    │   2. Local project agent (.cade/settings.local.json → agent_id)
    │   3. Global last agent (~/.cade/settings.json → last_agent_id)
    │   4. Create new agent (fallback)
    │
    ├── bootstrap/tools.rs   → register_and_attach_with_caps_filtered()
    ├── bootstrap/memory.rs  → seed_default_memory()
    └── bootstrap/prompt.rs  → build_system_prompt(capabilities)
            │
            ▼
        REPL / Headless loop
            │
            ▼
    cade-agent ──HTTP──► cade-server ──► LLM provider
                              │
                              ▼
                         cade-store (SQLite)
```

### Message Processing

```
User input → cade-agent → POST /v1/agents/:id/messages/stream → cade-server
                                                                      │
                                                              LLM provider call
                                                                      │
                                                              SSE stream back
                                                                      │
                                                            Tool call routing
                                                                      │
                                                            Tool execution
                                                            (local on client)
                                                                      │
                                                            Result → server
                                                                      │
                                                            Continue streaming
```

### SSE Protocol (Server-Sent Events)

The server streams typed events to clients over SSE. The GUI dashboard (`cade-gui`) and TUI (`cade-tui`) both consume these events:

| Event type | Payload | Purpose |
|------------|---------|---------|
| `text` | String chunk | Streaming assistant text |
| `reasoning` | String chunk | Streaming reasoning/thinking text |
| `tool_call` | `{name, arguments, call_id}` | Tool invocation by the LLM |
| `tool_result` | `{call_id, content}` | Result of a tool execution |
| `conversation_id` | String | Assigned conversation ID |
| `usage` | `{input, output, model}` | Token usage for the turn |
| `finish` | `{reason}` | Stream completion signal |
| `error` | `{message}` | Error during processing |
| `theme_update` | `ThemeColors` JSON | Live theme change broadcast |

### Tool Selection Pipeline

```
Available tools (from capabilities + MCP servers)
    │
    ├── ITS (Intelligent Tool Selection)
    │   Cross-encoder reranking via ONNX ms-marco-MiniLM-L-6-v2
    │   or cloud API for tool relevance scoring
    │
    ├── Toolset filtering (default / codex / gemini)
    │   Adapts edit tools per model family
    │
    └── --tools CLI filter (optional)
        Restricts what's registered in the LLM context window
```

---

## MCP Server Boot

MCP (Model Context Protocol) servers are spawned in parallel using `tokio::task::JoinSet` for O(1) boot time regardless of server count. Each server process undergoes a handshake to register its tool definitions, which are then merged into the agent's available tool set.

---

## Theme System

CADE uses a unified theme system defined in `cade-core/src/resources/themes/`:

```
ThemeColors (cade-core)
    │
    ├── cade-tui/src/colors.rs
    │   ThemeColorsExt trait → ratatui::style::Style
    │   ColorDefExt trait    → ratatui::style::Color
    │   BorderStyleExt trait → ratatui::widgets::BorderType
    │
    └── cade-gui/src/theme.rs
        EguiThemeExt trait   → egui::Color32
        EguiColorExt trait   → egui::Color32
        apply_theme()        → sets egui::Visuals + spacing
```

Both UIs consume the same `ThemeColors` struct. Themes are defined as TOML files on disk and can be switched at runtime via the `/theme <name>` command, which broadcasts a `theme_update` SSE event to connected clients.

The GUI follows a "TUI-fied" design language: zero corner rounding, no shadows, compact spacing, monospace fonts for role headers, and a full-width command palette — matching the terminal aesthetic of the TUI.

---

## Memory & Consolidation Pipeline

### Memory Blocks

| Block | Purpose | Lifecycle |
|-------|---------|-----------|
| `persona` | Agent identity and style | Persistent, always injected |
| `human` | Facts about the user | Persistent, always injected |
| `project` | Project context, stack, conventions | Persistent, always injected |
| `session_summary` | Auto-generated summary of older turns | Auto-updated by consolidation |
| `working_set` | Current task, files modified, next steps | Auto-pinned after consolidation |
| `skills` | Available skills listing | Injected on startup |

### Sleeptime Consolidation

When the context window approaches its budget, CADE runs the consolidation pipeline (`crates/cade-server/src/server/consolidation.rs`):

1. **Budget-aware turn assembly** — selects turns that fit within the token budget
2. **Pre-LLM artifact extraction** — `extract_artifacts()` scans messages before truncation for:
   - File paths (`src/foo/bar.rs`)
   - Function names (`my_func()`)
   - Error IDs (`RUSTSEC-2025-0009`, `error[E0433]`)
   - Error lines (`error: cannot find type...`)
3. **LLM summarization** — generates a narrative summary with:
   - `SEARCH ANCHORS:` section (up to 8 keywords for `conversation_search`)
   - Exact paths, function names, errors, and rejected-alternative reasoning
4. **Token limits** — `SUMMARY_MAX_TOKENS=900`, `SESSION_SUMMARY_MAX_CHARS=4500`
5. **Auto-pin** — `working_set` block is auto-pinned after consolidation to prevent aging out at the 80-turn stale threshold

### Memory Aging

- Blocks idle for 80+ turns are archived (replaced with label + excerpt in prompt)
- Archived blocks are auto-promoted back when matched by `search_memory()`
- Pinned blocks (e.g., `working_set`) are exempt from aging

---

## Settings Resolution

Settings are layered with increasing specificity:

```
Global:   ~/.cade/settings.json           (API keys, default model, hooks)
Project:  .cade/settings.json             (project-level overrides, MCP servers)
Local:    .cade/settings.local.json       (per-user, gitignored — agent ID, permissions)
CLI:      --flags and env vars            (highest priority)
```

Merge logic is in `crates/cade-core/src/settings/resolver.rs`.

---

## GUI Dashboard (cade-gui)

The GUI is an **egui/eframe WASM application** compiled to `wasm32-unknown-unknown` and served by `cade-server` at `/dashboard` via `rust-embed`. It provides a browser-based interface for:

- Agent selection and conversation management
- Real-time streaming responses via SSE
- Tool call inspection with collapsible details
- Command palette (`Ctrl+K` or `/`)
- Overlay panels: memory, MCP servers, model picker, artifacts, checkpoints, stats
- Live theme synchronization with the server

The GUI shares the `ThemeColors` system with the TUI and follows the same flat, dense, terminal-inspired design language.

---

## Cross-Platform Support

| Component | Linux | macOS | Windows |
|-----------|-------|-------|---------|
| Shell execution | `bash -c` | `bash -c` | `cmd.exe /C` |
| Notifications | D-Bus (urgency) | macOS sound | Windows Toast |
| Screen capture | xcap (X11/Wayland) | xcap | Windows capture APIs |
| Input control | xdotool/ydotool | macOS Accessibility | Windows input APIs |
| File operations | ✓ | ✓ | ✓ |
| Git integration | ✓ | ✓ | ✓ |
| Docker backend | ✓ | ✓ | ✓ (Docker Desktop) |
| SSH backend | ✓ | ✓ | ✓ |

See [WINDOWS_SETUP.md](WINDOWS_SETUP.md) for Windows-specific instructions.

---

## Security Model

See [SECURITY.md](SECURITY.md) for the full security model, threat assumptions, and hardening options.

Key principles:
- **SQLite WAL mode** for concurrent read/write integrity
- **AES-GCM encryption** for stored API keys (`cade-store/src/crypto.rs`)
- **Permission modes** control tool execution (default, acceptEdits, plan, bypassPermissions)
- **Hooks** allow user-defined audit/block scripts at lifecycle events
- **Zero-panic safety** — no unhandled `unwrap()`/`expect()` in production code

---

## Editor Integrations

| Editor | Location | Features |
|--------|----------|----------|
| Neovim | `plugins/cade.nvim/` | Ghost-text completions, streaming SSE, partial acceptance |
| VS Code | `editors/vscode/` | Inline completions, streaming, status bar toggle |

Both use the stateless `/v1/agents/:id/complete` endpoint — no conversation pollution.

---

Built by [EzekTec Inc.](https://github.com/EzekTec-Inc) · Apache-2.0 / MIT
