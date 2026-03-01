# CADE

**Coding AI assistant with Desktop Extensions**

A stateful, self-improving Rust CLI coding agent. CADE gives an AI agent full access to your local development environment — including your desktop — and ships its own server, so no external platform account is required.

---

## Features

### Core coding tools
| Tool | Description |
|------|-------------|
| `bash` | Execute shell commands (build, test, git, etc.) |
| `read_file` | Read files with line numbers and optional offset/limit |
| `write_file` | Write files, auto-creating parent directories |
| `edit_file` | Str-replace editing — precise, diff-like changes |
| `apply_patch` | Unified diff patching (used with OpenAI/Codex toolset) |
| `grep` | Regex search across the codebase |
| `glob` | Find files by pattern, sorted by modification time |

### Desktop Extensions (the "D" in CADE)
| Tool | Description |
|------|-------------|
| `desktop_screenshot` | Capture screen or a specific window → base64 PNG |
| `desktop_list_windows` | List all visible window titles |
| `desktop_control` | Focus windows, type text, key presses, mouse control |
| `desktop_notify` | Send OS desktop notifications |
| System tray | Run CADE as a background service (`--tray`) |

### Meta tools (always available)
| Tool | Description |
|------|-------------|
| `update_memory` | Persist facts across sessions (persona / human / project blocks) |
| `load_skill` | Load a skill's full content into context |
| `install_skill` | Install a skill from a URL or path |
| `run_subagent` | Spawn a background subagent for parallel tasks |

---

## Quickstart

```bash
# 1. Build
cargo build --release

# 2. Start the CADE server (in one terminal)
ANTHROPIC_API_KEY=sk-ant-... ./target/release/cade-server

# 3. Start the CLI (in another terminal — auto-connects to localhost:8284)
./target/release/cade
```

On first launch, CADE creates a new agent and remembers it per directory.

---

## CLI Usage

```
cade                              # Interactive REPL (auto-resumes last agent)
cade --new                        # Start a fresh conversation (same agent)
cade --new-agent                  # Create a brand-new agent
cade --agent <id>                 # Resume a specific agent by ID
cade --name <partial>             # Resume agent by name (partial, case-insensitive)
cade --resume                     # Browse past conversations interactively
cade --continue                   # Resume last session (suppress env re-injection)
cade -p "..."                     # Headless prompt (non-interactive)
cade -m <model>                   # Specify model (e.g. anthropic/claude-sonnet-4-5)
cade --yolo                       # Bypass all permission prompts
cade --permission-mode plan       # Read-only mode
cade --tray                       # Start with system tray icon
cade --info                       # Show session info and exit
cade --rename <name>              # Rename the current agent and exit
cade --tools "bash,read_file"     # Restrict tools registered to the agent
cade --link                       # Re-attach all tools to the agent
cade --unlink                     # Detach all tools from the agent
```

### Permission modes

| Mode | Behaviour |
|------|-----------|
| `default` | Prompts for approval on write/execute tool calls |
| `acceptEdits` | Auto-approves file write/edit only |
| `plan` | Read-only — blocks bash/write/edit |
| `bypassPermissions` | Auto-approves everything (`--yolo`) |

### Tool filtering

`--tools` controls what is **registered in the LLM's context window** (different from `--allowed-tools` which is a runtime permission gate):

```bash
cade --tools "bash,read_file,grep"   # Only these tools visible to the agent
cade --tools ""                      # No tools — analysis-only mode
```

### Output formats (headless mode)

```bash
cade -p "..." --output-format text         # Default plain text
cade -p "..." --output-format json         # Structured JSON
cade -p "..." --output-format stream-json  # SSE JSON stream
cade -p "..." --no-stream                  # Wait for full response before printing
```

---

## Interactive slash commands

### Navigation
| Command | Description |
|---------|-------------|
| `/help` | Show all commands |
| `/exit` | Quit |
| `/clear` | Clear the screen |
| `/info` | Show session info (agent, model, mode) |

### Agent management
| Command | Description |
|---------|-------------|
| `/new` | Start a fresh conversation (same agent) |
| `/new-agent` | Create a new agent |
| `/agents` | List all agents |
| `/agent` | Show current agent ID |
| `/name <n>` | Resume agent by name |
| `/resume` | Browse past conversations |
| `/delete [id]` | Delete an agent |
| `/rename <name>` | Rename the current agent |
| `/pin` | Pin current agent as default |

### Memory & skills
| Command | Description |
|---------|-------------|
| `/memory` | Show all memory blocks |
| `/remember <text>` | Append a note to project memory |
| `/init` | Inject current environment context |
| `/skills [filter]` | List available skills |

### Tools & MCP
| Command | Description |
|---------|-------------|
| `/link` | Re-attach all native + MCP tools to the agent |
| `/unlink` | Detach all tools from the agent |
| `/mcp` | List connected MCP servers and their tools |
| `/toolset [name]` | Show or switch toolset (default / codex / gemini) |

### Model & mode
| Command | Description |
|---------|-------------|
| `/model <m>` | Switch model mid-session |
| `/mode [name]` | Show or switch permission mode |
| `/yolo` | Disable all permission prompts |
| `/plan` | Enable read-only mode |
| `/default` | Restore default permission mode |
| `/stream` | Toggle SSE streaming on/off |

### Permissions
| Command | Description |
|---------|-------------|
| `/permissions` | Show current permission rules |
| `/approve-always <pattern>` | Always approve matching tools |
| `/deny-always <pattern>` | Always deny matching tools |

### Other
| Command | Description |
|---------|-------------|
| `/providers` | List configured LLM providers |
| `/connect <url>` | Connect to a different CADE server |
| `/disconnect` | Disconnect from server |
| `/hooks` | Show configured lifecycle hooks |
| `/subagents` | List running subagents |
| `/usage` | Show token usage for this session |
| `/search <query>` | Search message history |
| `/feedback` | Send feedback |
| `/logout` | Clear stored API key and exit |

---

## Toolsets

CADE ships three toolsets optimised for different model families:

| Toolset | Models | Edit style |
|---------|--------|------------|
| `default` | Claude, Llama, Mistral, etc. | String-replace (`edit_file`) |
| `codex` | GPT, o1, o3, o4 | Unified diff (`apply_patch`) |
| `gemini` | Gemini | String-replace (`edit_file`) |

The toolset is auto-detected from the model name. Override with `--toolset <name>` or `/toolset <name>`.

---

## Session Persistence

CADE remembers the last agent per directory:

| File | Scope | Contents |
|------|-------|----------|
| `.cade/settings.local.json` | Per-project (gitignored) | Last agent for this directory |
| `~/.cade/settings.json` | Global | API key, global last agent, permissions, hooks |

---

## Memory System

The agent has three persistent memory blocks, updated via `update_memory`:

| Block | Purpose |
|-------|---------|
| `persona` | Agent identity and working style |
| `human` | Facts about the user (name, preferences) |
| `project` | Current project context, tech stack, conventions |

Memory persists across sessions — the agent builds up context over time.

---

## Skills

Skills are markdown files that inject domain knowledge into the agent's context.

### Directory layout
```
.skills/                    # Project-scoped skills (committed with the repo)
│   └── deployment/
│       └── SKILL.md
~/.cade/skills/             # Global skills (available in all projects)
    └── my-tool/
        └── SKILL.md
```

### SKILL.md format
```markdown
---
name: Deployment Guide
description: How to deploy this project to production
triggers: [deploy, aws, production]
---

# Deployment Steps
...
```

Custom skills directory: `cade --skills /path/to/skills`

---

## MCP Servers

CADE supports MCP (Model Context Protocol) servers, exposing their tools to the agent with a `{server}__` prefix.

Configure in `~/.cade/settings.json`:

```json
{
  "mcpServers": {
    "git": {
      "command": "/path/to/git-mcp-server"
    },
    "my-server": {
      "command": "node",
      "args": ["/path/to/server/dist/index.js"]
    }
  }
}
```

View connected servers and tools with `/mcp`.

---

## Hooks

User-defined shell scripts that fire at lifecycle events. Configure in `~/.cade/settings.json`:

```json
{
  "hooks": {
    "PreToolUse": [
      {
        "matcher": "bash",
        "hooks": [{ "type": "command", "command": "my-audit-script.sh" }]
      }
    ],
    "PostToolUse": [],
    "PostToolUseFailure": [],
    "PermissionRequest": [],
    "UserPromptSubmit": [],
    "Stop": [],
    "SubagentStop": [],
    "SessionStart": [],
    "SessionEnd": [],
    "Notification": []
  }
}
```

Hook scripts receive a JSON payload on stdin. Exit codes:

| Exit code | Meaning |
|-----------|---------|
| `0` | Allow — proceed normally |
| `1` | Log and continue |
| `2` | Block — stderr is fed back to the agent |

`PostToolUse` hooks can inject additional context by printing `{"additionalContext": "..."}` to stdout.

---

## Self-hosting with cade-server

CADE ships its own server — no third-party platform required.

```bash
# 1. Set your LLM provider API key
export ANTHROPIC_API_KEY=sk-ant-...     # or OPENAI_API_KEY / GOOGLE_API_KEY

# 2. Start the server (defaults to :8284)
./target/release/cade-server

# 3. The CLI auto-connects to localhost:8284
./target/release/cade
```

### LLM provider auto-detection

The server picks a provider by scanning env vars in priority order:

```
ANTHROPIC_API_KEY → OpenAI → GOOGLE_API_KEY → Ollama (local fallback)
```

Override explicitly:
```bash
CADE_LLM_PROVIDER=openai OPENAI_API_KEY=... cade-server
```

### Server env vars

| Variable | Default | Description |
|----------|---------|-------------|
| `CADE_SERVER_PORT` | `8284` | Port to listen on |
| `CADE_LLM_PROVIDER` | auto-detect | `anthropic` \| `openai` \| `gemini` \| `ollama` |
| `CADE_DEFAULT_MODEL` | provider default | Override the default model |
| `CADE_DB_PATH` | `~/.cade/cade.db` | SQLite database path |
| `ANTHROPIC_API_KEY` | — | Anthropic / Claude |
| `OPENAI_API_KEY` | — | OpenAI / GPT |
| `GOOGLE_API_KEY` | — | Google / Gemini |
| `OLLAMA_BASE_URL` | `http://localhost:11434` | Ollama base URL |
| `CADE_API_KEY` | — | Optional auth token for the server |

Default models per provider:

| Provider | Default model |
|----------|--------------|
| Anthropic | `claude-opus-4-5` |
| OpenAI | `gpt-4o` |
| Gemini | `gemini-2.0-flash` |
| Ollama | `llama3.2` |

### CLI env vars

| Variable | Default | Description |
|----------|---------|-------------|
| `CADE_SERVER_URL` | `http://localhost:8284` | cade-server URL |
| `CADE_API_KEY` | — | Auth token sent to cade-server |

### Remote server
```bash
export CADE_SERVER_URL=http://my-server:8284
export CADE_API_KEY=my-token
cade
```

### Server API (summary)

| Method | Path | Description |
|--------|------|-------------|
| `GET` | `/v1/health` | Health check |
| `GET/POST` | `/v1/agents` | List / create agents |
| `GET/PATCH/DELETE` | `/v1/agents/:id` | Get / update / delete agent |
| `GET/POST/DELETE` | `/v1/agents/:id/tools` | List / attach / detach tools |
| `GET/PUT/DELETE` | `/v1/agents/:id/memory/:label` | Read / write / delete memory block |
| `POST/DELETE/GET` | `/v1/agents/:id/messages` | Send / clear / search messages |
| `POST` | `/v1/agents/:id/messages/stream` | Send message with SSE streaming |
| `GET/GET` | `/v1/runs/:id` / `/v1/runs/:id/stream` | Background run status / stream |
| `GET/POST` | `/v1/tools` | List / register tools |
| `GET` | `/v1/models` | List available models |
| `GET/POST/DELETE` | `/v1/providers` | List / add / remove LLM providers |

---

## Desktop Extensions

CADE runs on Linux with Wayland or X11.

**Screen capture** requires no extra dependencies (uses `xcap`).

**Window control** requires `xdotool` (X11) or `ydotool` (Wayland):
```bash
sudo apt install xdotool     # X11
sudo apt install ydotool     # Wayland
```

**Notifications** use the system DBus notification daemon (pre-installed on most desktops).

**System tray** requires a DBus-compatible desktop environment (GNOME, KDE, COSMIC, etc.).

---

## Build

```bash
# Optional: dependencies for screen capture on Wayland
sudo apt install libpipewire-0.3-dev libclang-dev libgbm-dev

# Debug build
cargo build

# Release binaries
cargo build --release

# Install globally
cargo install --path .
```

---

## Project Structure

```
src/
├── main.rs                  # Entry point, CLI arg handling, agent bootstrap
├── lib.rs                   # Module declarations
├── agent/
│   ├── client.rs            # REST API client (agents, tools, memory, messages)
│   ├── session.rs           # Per-directory session persistence
│   └── tools.rs             # Tool registration with the server
├── cli/
│   ├── args.rs              # CLI argument parsing (clap)
│   ├── repl.rs              # Interactive REPL, slash commands, tool execution loop
│   └── headless.rs          # Headless -p mode
├── server/
│   ├── api/                 # axum route handlers (agents, messages, tools, runs…)
│   ├── llm/                 # LLM provider abstraction (Anthropic, OpenAI, Gemini, Ollama)
│   ├── storage/             # SQLite persistence
│   ├── config.rs            # Server config from env vars
│   └── state.rs             # Shared server state
├── tools/
│   ├── bash.rs              # Shell execution
│   ├── fs.rs                # Read / Write / Edit / ApplyPatch
│   ├── search.rs            # Grep / Glob
│   ├── desktop.rs           # Desktop tool wrappers
│   └── manager.rs           # Tool dispatch registry, schema registry
├── toolsets/
│   └── mod.rs               # Toolset definitions (Default / Codex / Gemini)
├── desktop/
│   ├── capture.rs           # Screen capture (xcap)
│   ├── control.rs           # Window/app control (xdotool/ydotool)
│   ├── notify.rs            # OS notifications (notify-rust)
│   └── tray.rs              # System tray (ksni)
├── mcp/                     # MCP client — spawn and call local MCP servers
├── hooks/                   # Lifecycle hook engine
├── permissions/             # Permission mode management
├── settings/                # Settings manager (~/.cade/settings.json)
├── skills/                  # SKILL.md discovery and loading
├── subagents/               # Subagent runner
└── bin/
    └── cade-server.rs       # cade-server entry point
```

---

Built by [EzekTec Inc.](https://github.com/EzekTec-Inc) · Apache-2.0 / MIT
