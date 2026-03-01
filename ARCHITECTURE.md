# CADE Architecture

> **CADE** — Coding AI Assistant with Desktop Extensions  
> A stateful, multi-provider AI coding agent built in Rust.

---

## Table of Contents

1. [Overview](#overview)
2. [High-Level Architecture](#high-level-architecture)
3. [Binary Layout](#binary-layout)
4. [Module Map](#module-map)
5. [Core Subsystems](#core-subsystems)
   - [cade-server (Agent Backend)](#cade-server-agent-backend)
   - [Agent Client](#agent-client)
   - [REPL / CLI](#repl--cli)
   - [MCP Integration](#mcp-integration)
   - [Tool System](#tool-system)
   - [Memory System](#memory-system)
   - [Skills System](#skills-system)
   - [Subagent System](#subagent-system)
   - [Hook Engine](#hook-engine)
   - [Permission System](#permission-system)
   - [Desktop Extensions](#desktop-extensions)
6. [Request Lifecycle](#request-lifecycle)
7. [Data Flow](#data-flow)
8. [Settings & Configuration](#settings--configuration)
9. [Key Dependencies](#key-dependencies)

---

## Overview

CADE is a terminal-native AI coding agent. It consists of two Rust binaries that
collaborate over a local HTTP API:

| Binary | Role |
|---|---|
| `cade-server` | Stateful agent backend — manages agents, memory, message history, LLM routing |
| `cade` | Interactive frontend — REPL/CLI, MCP clients, tool execution, desktop extensions |

The two processes communicate via a REST + Server-Sent Events (SSE) streaming API,
making the architecture cleanly separable. `cade-server` can be replaced by any
Letta-compatible server; `cade` is the opinionated local front-end.

---

## High-Level Architecture

```
┌─────────────────────────────────────────────────────────────────────────────┐
│                              User's Terminal                                │
│                                                                             │
│  ┌─────────────────────────────────────────────────────────────────────┐   │
│  │                         cade  (CLI binary)                          │   │
│  │                                                                     │   │
│  │  ┌──────────────┐  ┌──────────────┐  ┌──────────────────────────┐  │   │
│  │  │   REPL / UI  │  │  CLI / Args  │  │    Headless / Pipe mode  │  │   │
│  │  │  (ratatui +  │  │  (clap)      │  │    (stdin → stdout)      │  │   │
│  │  │  crossterm)  │  │              │  │                          │  │   │
│  │  └──────┬───────┘  └──────┬───────┘  └───────────┬─────────────┘  │   │
│  │         └─────────────────┴──────────────────────┘                 │   │
│  │                           │ user prompt                             │   │
│  │                    ┌──────▼───────────────────┐                    │   │
│  │                    │       Agent Client        │                    │   │
│  │                    │  (REST + SSE streaming)   │                    │   │
│  │                    └──────┬────────────────────┘                    │   │
│  │                           │ HTTP/SSE                                │   │
│  │  ┌────────────────────────│──────────────────────────────────────┐  │   │
│  │  │                 Tool Execution Layer                          │  │   │
│  │  │  ┌──────────┐ ┌──────────┐ ┌───────────┐ ┌────────────────┐ │  │   │
│  │  │  │ Built-in │ │   MCP    │ │  Desktop  │ │ Permission +   │ │  │   │
│  │  │  │  Tools   │ │ Manager  │ │Extensions │ │  Hook Engine   │ │  │   │
│  │  │  │bash/fs/  │ │(rmcp)    │ │capture/   │ │                │ │  │   │
│  │  │  │grep/glob │ │          │ │notify/tray│ │                │ │  │   │
│  │  │  └──────────┘ └────┬─────┘ └───────────┘ └────────────────┘ │  │   │
│  │  └───────────────────────────────────────────────────────────────┘  │   │
│  │                        │ stdio (MCP protocol)                        │   │
│  │              ┌─────────┴──────────────────────────┐                 │   │
│  │              │         MCP Servers (child procs)   │                 │   │
│  │              │  git │ developer │ desktop-commander │                 │   │
│  │              │  lsp-rust │ lsp-typescript │ context7│                │   │
│  │              └────────────────────────────────────┘                 │   │
│  └─────────────────────────────────────────────────────────────────────┘   │
│                           │ HTTP REST + SSE                                 │
│  ┌─────────────────────────────────────────────────────────────────────┐   │
│  │                    cade-server  (Axum HTTP server)                  │   │
│  │                                                                     │   │
│  │  ┌─────────────┐  ┌──────────────┐  ┌──────────────────────────┐  │   │
│  │  │  REST API   │  │  LLM Router  │  │     SQLite Storage        │  │   │
│  │  │  /agents    │  │ Anthropic /  │  │  agents │ messages │      │  │   │
│  │  │  /messages  │  │ OpenAI /     │  │  memory │ tools │ runs    │  │   │
│  │  │  /runs      │  │ Gemini /     │  │                          │  │   │
│  │  │  /tools     │  │ Ollama       │  │                          │  │   │
│  │  │  /providers │  │              │  │                          │  │   │
│  │  └─────────────┘  └──────────────┘  └──────────────────────────┘  │   │
│  └─────────────────────────────────────────────────────────────────────┘   │
│                           │ HTTPS                                           │
│              ┌────────────┴──────────────────────────┐                     │
│              │         LLM Provider APIs              │                     │
│              │  Anthropic │ OpenAI │ Gemini │ Ollama  │                     │
│              └────────────────────────────────────────┘                    │
└─────────────────────────────────────────────────────────────────────────────┘
```

---

## Binary Layout

```
CADE/
├── src/
│   ├── main.rs              ← cade binary entry point (REPL + tool execution)
│   ├── lib.rs               ← shared library (all modules re-exported)
│   ├── bin/
│   │   └── cade-server.rs   ← cade-server binary entry point (Axum HTTP server)
│   │
│   ├── agent/               ← REST client for cade-server
│   │   ├── client.rs        ← CadeClient (HTTP + SSE)
│   │   ├── session.rs       ← SessionStore (agent ID persistence)
│   │   └── tools.rs         ← built-in tool schema registration
│   │
│   ├── cli/                 ← user-facing interfaces
│   │   ├── args.rs          ← clap CLI argument definitions
│   │   ├── repl.rs          ← interactive REPL (ratatui + crossterm)
│   │   └── headless.rs      ← pipe/stdin headless mode
│   │
│   ├── mcp/                 ← MCP server integration (rmcp)
│   │   └── mod.rs           ← McpManager: spawn, handshake, route tool calls
│   │
│   ├── tools/               ← built-in tool implementations
│   │   ├── bash.rs          ← shell command execution
│   │   ├── fs.rs            ← file read/write/edit/glob
│   │   ├── search.rs        ← grep (regex file search)
│   │   ├── desktop.rs       ← screenshot, notify, window control
│   │   └── manager.rs       ← tool dispatch + schema registry
│   │
│   ├── toolsets/            ← model-specific tool family selection
│   │   └── mod.rs           ← Default (Claude) / Codex (OpenAI) / Gemini
│   │
│   ├── skills/              ← skill discovery + on-demand loading
│   │   └── mod.rs
│   │
│   ├── subagents/           ← subagent definitions + spawning
│   │   └── mod.rs
│   │
│   ├── hooks/               ← lifecycle hook engine
│   │   └── mod.rs
│   │
│   ├── permissions/         ← allow/deny rule matching
│   │   └── mod.rs
│   │
│   ├── desktop/             ← desktop extension primitives
│   │   ├── capture.rs       ← xcap screen capture
│   │   ├── control.rs       ← xdotool/ydotool input control
│   │   ├── notify.rs        ← OS notifications (notify-rust)
│   │   └── tray.rs          ← system tray (ksni / D-Bus)
│   │
│   ├── server/              ← cade-server internals
│   │   ├── api/             ← Axum route handlers
│   │   │   ├── agents.rs    ← CRUD + memory management
│   │   │   ├── messages.rs  ← send message + SSE stream
│   │   │   ├── runs.rs      ← run lifecycle
│   │   │   ├── tools.rs     ← tool registry API
│   │   │   ├── providers.rs ← LLM provider management
│   │   │   ├── models.rs    ← model listing
│   │   │   └── health.rs    ← health check
│   │   ├── llm/             ← LLM provider adapters
│   │   │   ├── anthropic.rs
│   │   │   ├── openai.rs
│   │   │   ├── gemini.rs
│   │   │   ├── ollama.rs
│   │   │   └── catalogue.rs ← model catalogue
│   │   ├── storage/
│   │   │   └── sqlite.rs    ← all DB operations (rusqlite)
│   │   ├── config.rs        ← ServerConfig (env-driven)
│   │   └── state.rs         ← AppState (shared across handlers)
│   │
│   ├── settings/            ← settings.json loader
│   │   └── manager.rs       ← SettingsManager, McpServerConfig, HooksConfig
│   │
│   └── ui/                  ← terminal rendering helpers
│       ├── input.rs         ← keyboard/mouse input handling
│       └── output.rs        ← markdown rendering, streaming output
│
├── .cade/                   ← runtime config (gitignored)
│   ├── settings.local.json  ← local overrides (MCP servers, hooks, etc.)
│   └── agents/              ← custom subagent definitions (.md files)
│
└── .skills/                 ← project-scoped skills
    ├── conventional-commits/
    └── example/
```

---

## Module Map

```
lib.rs
 ├── agent      ← CadeClient (REST), SessionStore, tool schema registration
 ├── cli        ← Args (clap), Repl (ratatui), Headless (stdin/stdout)
 ├── desktop    ← capture, control, notify, tray
 ├── hooks      ← HookEngine, HookOutcome (PreToolUse / PostToolUse / …)
 ├── mcp        ← McpManager (rmcp, stdio transport)
 ├── permissions← PermissionManager, PermissionRule, PermissionMode
 ├── server     ← Axum API, LlmRouter, SQLite storage, AppState
 ├── settings   ← SettingsManager, McpServerConfig, HooksConfig
 ├── skills     ← discover_all_skills, skills_listing, load_skill
 ├── subagents  ← SubagentDef, SubagentTools, SubagentScope
 ├── toolsets   ← Toolset (Default / Codex / Gemini)
 ├── tools      ← bash, fs, search, desktop; ToolManager dispatch
 └── ui         ← input handling, markdown output rendering
```

---

## Core Subsystems

### cade-server (Agent Backend)

`cade-server` is a self-contained Axum HTTP server — the stateful brain of CADE.
It is modeled after the Letta server API, making the two interchangeable.

```
┌─────────────────────────────────────────────────────┐
│                    cade-server                      │
│                                                     │
│  REST Endpoints                                     │
│  ─────────────────────────────────────────────────  │
│  POST   /v1/agents                 create agent     │
│  GET    /v1/agents                 list agents      │
│  GET    /v1/agents/:id             get agent        │
│  DELETE /v1/agents/:id             delete agent     │
│  GET    /v1/agents/:id/memory      get memory       │
│  PATCH  /v1/agents/:id/memory/…    upsert block     │
│  POST   /v1/agents/:id/messages    send + stream    │
│  GET    /v1/agents/:id/messages    message history  │
│  GET    /v1/runs/:id               run status       │
│  GET    /v1/tools                  list tools       │
│  POST   /v1/tools                  register tool    │
│  GET    /v1/providers              list providers   │
│  POST   /v1/providers              add provider     │
│  GET    /v1/models                 list models      │
│  GET    /v1/health                 health check     │
│                                                     │
│  LLM Router                                         │
│  ─────────────────────────────────────────────────  │
│  ┌──────────┐ ┌──────────┐ ┌────────┐ ┌─────────┐  │
│  │Anthropic │ │ OpenAI   │ │ Gemini │ │ Ollama  │  │
│  └──────────┘ └──────────┘ └────────┘ └─────────┘  │
│  Auto-detected from env keys; DB-overridable        │
│                                                     │
│  SQLite Storage                                     │
│  ─────────────────────────────────────────────────  │
│  tables: agents, memory_blocks, messages,           │
│           tools, runs, providers                    │
└─────────────────────────────────────────────────────┘
```

**LLM Provider detection priority:**
1. `CADE_LLM_PROVIDER` env var (explicit override)
2. First found API key: `ANTHROPIC_API_KEY` → `OPENAI_API_KEY` → `GOOGLE_API_KEY`
3. Ollama (always available as local fallback)

**Default models by provider:**

| Provider | Default Model |
|---|---|
| Anthropic | `claude-opus-4-5` |
| OpenAI | `gpt-4o` |
| Gemini | `gemini-2.0-flash` |
| Ollama | `llama3.2` |

---

### Agent Client

`src/agent/client.rs` — `CadeClient` wraps `reqwest` to speak the cade-server REST API.
It handles:
- Agent creation and retrieval
- Memory block upserts
- Streaming message sends (SSE via `reqwest-eventsource`)
- Tool registration

```
CadeClient
  .create_agent(req)        → AgentState
  .send_message(id, msg)    → impl Stream<Item = CadeMessage>
  .upsert_memory(id, label) → ()
  .create_tool(req)         → ()
  .get_messages(id)         → Vec<CadeMessage>
```

`CadeMessage` is the wire format for all streamed events:

```
message_type = "assistant_message"      ← text to display
             = "tool_call_message"      ← tool invocation request
             = "tool_return_message"    ← tool result echo
             = "stream_start"           ← run_id assigned
             = "stream_end"             ← turn complete
```

---

### REPL / CLI

The user interface lives in `src/cli/`:

```
Args (clap)
 ├── --model          override LLM model
 ├── --server         cade-server URL (default: http://localhost:8284)
 ├── --agent          reuse a named agent
 ├── --permission-mode (auto / ask / deny)
 ├── --toolset        (default / codex / gemini)
 └── -p / --print     headless: prompt from CLI arg

Repl (ratatui + crossterm)
 ├── Markdown rendering (custom parse_markdown_lines: headings, bullets, code fences, bold/italic)
 ├── Single-line spinner during generation; insert_before re-render on completion
 ├── Slash commands: /help /memory /mcp /skills /subagents /clear /exit …
 └── Multi-line input (Shift+Enter)

Headless mode (headless.rs)
 └── stdin → cade-server → stdout (scriptable / pipe-friendly)
```

---

### MCP Integration

`src/mcp/mod.rs` — `McpManager` uses the `rmcp` crate to spawn MCP servers as child
processes over stdio. Each server exposes a set of tools that are automatically
namespaced with a `{server_key}__` prefix to avoid collisions.

```
Startup sequence:
  settings.json
       │  mcpServers config
       ▼
  McpManager::start()
       │  for each enabled server:
       │    Command::new(cmd) + args + env
       │    TokioChildProcess transport
       │    rmcp handshake (initialize)
       │    list_tools() → Vec<McpToolSchema>
       ▼
  McpManager (ready)
       │  tools prefixed: "git__status", "developer__bash", …
       ▼
  REPL tool dispatch
```

**Configured MCP servers:**

| Server key | What it provides |
|---|---|
| `git` | Git operations (`git__add`, `git__commit`, `git__push`, …) |
| `developer` | Shell, file editor, grep, screen capture, LSP wrappers |
| `desktop-commander` | Long-running processes, interactive sessions, system info |
| `lsp-rust` | rust-analyzer LSP (diagnostics, hover, completions, …) |
| `lsp-typescript` | TypeScript language server |
| `context7` | Up-to-date library docs via Upstash Context7 API |

---

### Tool System

CADE has three categories of tools, all dispatched through the same loop:

```
Tool call from LLM
        │
        ▼
  PermissionManager.check()  ──deny──► block + inform agent
        │ allow
        ▼
  HookEngine.pre_tool_use()  ──block──► block + inform agent
        │ allow
        ▼
  ┌─────────────────────────────────────┐
  │        Tool Dispatch                │
  │                                     │
  │  is MCP-prefixed (contains "__")?   │
  │  ├─yes─► McpManager.call_tool()    │
  │  │                                  │
  │  │  is meta-tool?                   │
  │  ├─yes─► update_memory             │
  │  │       load_skill                │
  │  │       install_skill             │
  │  │       run_subagent              │
  │  │                                  │
  │  └─no──► Built-in tools:           │
  │           bash          (bash.rs)  │
  │           read_file     (fs.rs)    │
  │           write_file    (fs.rs)    │
  │           edit_file     (fs.rs)    │
  │           apply_patch   (fs.rs)    │
  │           grep          (search.rs)│
  │           glob          (fs.rs)    │
  │           desktop_*     (desktop.rs│
  └─────────────────────────────────────┘
        │
        ▼
  HookEngine.post_tool_use()  → optional additionalContext appended to result
        │
        ▼
  tool result sent back to agent
```

**Toolsets** — different model families get different editing tools:

| Toolset | Models | Edit tool |
|---|---|---|
| `Default` | Claude, Llama, Mistral | `edit_file` (string-replace) |
| `Codex` | GPT-4, o1, o3, o4 | `apply_patch` (unified diff) |
| `Gemini` | Gemini | `edit_file` (string-replace) |

---

### Memory System

Memory is stored server-side in SQLite as named blocks. The agent can read and
update them via the `update_memory` meta-tool. Blocks are injected into every
system prompt turn.

```
Default memory blocks:
  ┌────────────┬──────────────────────────────────────────────────┐
  │ Label      │ Purpose                                          │
  ├────────────┼──────────────────────────────────────────────────┤
  │ persona    │ Agent identity, style, and behavioral defaults   │
  │ human      │ User name, preferences, working style            │
  │ project    │ Active project, stack, conventions, ongoing work │
  │ skills     │ Auto-injected: available skills listing          │
  └────────────┴──────────────────────────────────────────────────┘

update_memory operations:
  set    → replace the block entirely
  append → concatenate new content to existing block
```

---

### Skills System

Skills are Markdown documents that give CADE domain-specific knowledge and
workflows. They are loaded **on-demand** — only when relevant — to keep context
clean.

```
Skill discovery (at startup):
  ~/.cade/skills/        ← global skills  (scope: global)
  .skills/               ← project skills (scope: project)

Skills listing injected into system prompt as:
  ## Available Skills
  - skill-name [scope] [tags]: description
  …

Agent uses load_skill("id") tool to pull full content when task matches.
Agent uses install_skill("url") tool to download + install new skills.

Skill file format (SKILL.MD):
  ---
  name: my-skill
  description: What this skill does
  tags: [git, testing]
  scope: project
  ---

  # Full skill content here...
```

---

### Subagent System

`run_subagent` spawns a focused child agent that executes a task and returns
only its final answer — keeping the main agent's context clean.

```
run_subagent call
      │
      ▼
  SubagentDef resolved
  ├── builtin: explore / general-purpose / coder / reviewer
  ├── global:  ~/.cade/agents/*.md
  └── project: .cade/agents/*.md

      │
      ▼
  CadeClient.create_agent()  ← new ephemeral agent on cade-server
      │
      ├── background=false: stream until done, return final answer
      └── background=true:  return immediately; notify on completion

Subagent tool access levels:
  All       → full tool access (same as parent)
  Readonly  → bash (read-only), read_file, glob, grep only
  List(…)   → explicit named subset
```

---

### Hook Engine

`src/hooks/mod.rs` — `HookEngine` fires user-defined shell scripts at key
lifecycle events. Scripts receive a JSON payload on stdin and signal outcomes
via exit code.

```
Events:
  PreToolUse          → can block tool execution (exit 2)
  PostToolUse         → can inject additionalContext into result
  PostToolUseFailure  → notified on tool error
  PermissionRequest   → triggered before interactive permission prompt
  UserPromptSubmit    → fires when user submits a message
  Stop                → fires when agent finishes a turn
  SubagentStop        → fires when a subagent completes
  SessionStart        → fires on REPL startup
  SessionEnd          → fires on REPL exit
  Notification        → general notification event

Exit code semantics:
  0  → allow / continue normally
  1  → log the stderr output; continue
  2  → block the action; send stderr to agent as context

Hook matchers:
  matcher: null / ""  → match all tools
  matcher: "bash"     → match only the bash tool
  matcher: ".*_file"  → regex match against tool name
```

---

### Permission System

`src/permissions/mod.rs` — controls which tools run automatically vs. require
interactive approval.

```
Permission modes (--permission-mode flag):
  auto   → all tools run without asking
  ask    → write/destructive tools prompt the user
  deny   → write/destructive tools are blocked

Rule syntax (in settings.json allowedTools / deniedTools):
  Bash                    → all bash invocations
  Bash(cargo test)        → bash where command == "cargo test"
  Bash(rm -rf:*)          → bash where command starts with "rm -rf"
  Read(src/**)            → read_file where path is under src/
  Edit                    → any edit_file call

Evaluation order:
  1. deniedTools rules   → block if any match
  2. allowedTools rules  → allow if any match
  3. permission mode     → fallback (auto/ask/deny)
```

---

### Desktop Extensions

`src/desktop/` provides OS-level capabilities beyond the terminal:

```
┌────────────────────────────────────────────────────┐
│              Desktop Extensions                    │
│                                                    │
│  capture.rs  — xcap                                │
│    desktop_screenshot()  → base64 PNG              │
│    Supports multi-monitor; resizes large captures  │
│                                                    │
│  control.rs  — xdotool / ydotool                   │
│    desktop_control(focus_window|type_text|         │
│                    key_press|move_mouse|click)      │
│    Auto-detects X11 vs Wayland                     │
│                                                    │
│  notify.rs  — notify-rust                          │
│    desktop_notify(title, body, urgency)            │
│    Sends native OS desktop notifications           │
│                                                    │
│  tray.rs  — ksni (D-Bus)                           │
│    spawn_tray()                                    │
│    System tray icon (Linux/D-Bus)                  │
└────────────────────────────────────────────────────┘
```

---

## Request Lifecycle

A complete turn from user input to assistant response:

```
 User types a message in REPL
          │
          ▼
 HookEngine.user_prompt_submit()
          │
          ▼
 CadeClient.send_message(agent_id, text)
          │  POST /v1/agents/:id/messages
          ▼
 cade-server receives request
  ├── load agent + memory from SQLite
  ├── build system prompt  (base_prompt + memory blocks + skills listing)
  ├── assemble message history
  └── call LLM (streaming SSE)
          │
          ▼
 SSE stream back to cade binary:
  ┌─ stream_start       → capture run_id
  ├─ assistant_message  → render token-by-token in REPL
  ├─ tool_call_message  → dispatch tool (see Tool System above)
  │    ├── permission check
  │    ├── pre-tool hook
  │    ├── execute tool
  │    ├── post-tool hook
  │    └── POST /v1/runs/:id/tools  (submit result back to server)
  │              │
  │              └── server continues LLM stream with tool result
  │                  (may produce more tool_call_messages → loop)
  └─ stream_end         → turn complete
          │
          ▼
 HookEngine.stop()
          │
          ▼
 REPL shows prompt again
```

---

## Data Flow

```
                    ┌──────────────────────────────────┐
                    │         settings.json            │
                    │  mcpServers, hooks, permissions  │
                    └──────────┬───────────────────────┘
                               │ SettingsManager
                               ▼
┌──────────┐         ┌─────────────────────┐
│  .skills/│────────►│   Skills Registry   │
│ ~/.cade/ │         │  (in-memory index)  │
│  skills/ │         └─────────┬───────────┘
└──────────┘                   │ skills_listing injected into
                               │ system prompt
┌──────────┐         ┌─────────▼───────────┐        ┌────────────┐
│ .cade/   │────────►│   SubagentDefs      │        │  SQLite DB │
│ agents/  │         │  (builtin+custom)   │        │ (server)   │
└──────────┘         └─────────────────────┘        │            │
                                                     │ agents     │
┌──────────┐         ┌─────────────────────┐        │ memory     │
│  MCP     │────────►│   McpManager        │        │ messages   │
│  servers │  stdio  │  tool schemas       │        │ tools      │
│(children)│◄────────│  + routing          │        │ runs       │
└──────────┘         └─────────────────────┘        └────────────┘
```

---

## Settings & Configuration

CADE merges settings from three layers (lowest → highest priority):

```
1. ~/.cade/settings.json      ← global defaults
2. .cade/settings.json        ← project overrides
3. .cade/settings.local.json  ← local secrets (gitignored)

Merged settings control:
  mcpServers:   { key: { command, args, env, disabled } }
  hooks:        { PreToolUse: [...], PostToolUse: [...], … }
  permissions:  { allowedTools: [...], deniedTools: [...] }
```

**Environment variables** (for `cade-server`):

| Variable | Default | Purpose |
|---|---|---|
| `CADE_SERVER_PORT` | `8284` | Server listen port |
| `CADE_DB_PATH` | `~/.cade/cade.db` | SQLite database path |
| `CADE_LLM_PROVIDER` | auto-detect | Force a provider |
| `CADE_DEFAULT_MODEL` | provider default | Force a model |
| `CADE_API_KEY` | none | Auth token for server requests |
| `ANTHROPIC_API_KEY` | — | Anthropic API key |
| `OPENAI_API_KEY` | — | OpenAI API key |
| `GOOGLE_API_KEY` | — | Google Gemini API key |

---

## Key Dependencies

| Crate | Version | Role |
|---|---|---|
| `tokio` | 1 | Async runtime (full features) |
| `axum` | 0.7 | HTTP server framework (cade-server) |
| `reqwest` | 0.12 | HTTP client + SSE streaming (cade) |
| `reqwest-eventsource` | 0.6 | SSE client |
| `rmcp` | 0.2 | MCP client (child-process stdio transport) |
| `clap` | 4.5 | CLI argument parsing |
| `ratatui` | 0.29 | Terminal UI rendering |
| `crossterm` | 0.28 | Cross-platform terminal control |
| `rusqlite` | 0.31 | SQLite (bundled) |
| `serde` / `serde_json` | 1 | Serialization |
| `xcap` | 0.8 | Cross-platform screen capture |
| `notify-rust` | 4 | OS desktop notifications |
| `ksni` | 0.2 | Linux system tray (D-Bus) |
| `tracing` | 0.1 | Structured logging |
| `anyhow` / `thiserror` | 1/2 | Error handling |
| `globset` / `ignore` | 0.4 | Glob patterns + `.gitignore` |
| `regex` | 1 | Grep tool pattern matching |
| `chrono` | 0.4 | Timestamps |
| `uuid` | 1 | Unique IDs |
