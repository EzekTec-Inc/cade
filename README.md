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

### Advanced Features
- **Intelligent Tool Selection (ITS)**: Reranks and filters tools using a local ONNX cross-encoder (`ms-marco-MiniLM-L-6-v2`) or cloud APIs before passing them to the LLM.
- **Dynamic Pricing Registry**: Real-time token cost estimation using an efficient, JSON-driven `ModelRegistry`.
- **Heuristic Evaluator Layer**: A fast, dynamically configurable subagent layer that intercepts user input to evaluate intent, safety, and pathfinding *before* any tool executes, ensuring strict adherence to project constraints while conserving the main context window.
- **Zero-Panic Safety**: Enforces the `rust10x` standard (no unhandled `unwrap()`/`expect()` in production code) and SQLite WAL mode for high integrity.

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

## Terminal UI Features

CADE features a highly responsive, custom-built terminal user interface (TUI) powered by Ratatui:

*   **Flicker-Free Rendering:** Uses CSI 2026 synchronized output for atomic screen updates, eliminating tearing on supported terminals (Kitty, WezTerm, iTerm2).
*   **Modern Clean Viewport:** Features a typography-driven timeline with inline `thinking...` animations and dynamic padding, avoiding the clutter of classic ASCII line-drawing.
*   **Native tmTheme Support:** Drop any TextMate `.tmTheme` file (like Tokyonight, Catppuccin, Gruvbox) into `~/.cade/themes/` to natively skin the entire UI and markdown syntax blocks instantly without needing external Lua plugins.
*   **Bracketed Paste:** Safely handles large text pastes. Pasting >10 lines collapses into a compact `[paste #1 +50 lines]` marker, keeping the input field usable. The full text is transparently expanded before sending to the LLM.
*   **Pluggable Autocomplete:** Press `Tab` for intelligent path completion, or type `@` to open a fuzzy-search file picker overlay.
*   **Multi-line Input:** Press `Shift+Enter` (or `Alt+Enter`) to insert a newline; plain `Enter` submits.
*   **Bash Shortcuts:** Prefix a line with `!command` to run it in the shell and send the output to the LLM, or `!!command` to run it silently (local output only).
*   **Undo / Redo:** `Ctrl+Z` / `Ctrl+Y` undo and redo edits in the input field (up to 100 levels).
*   **Standard Editing Keys:** `Ctrl+U` (clear to start), `Ctrl+K` (clear to end of line), `Ctrl+W` (delete word), `Alt+←`/`Alt+→` (word jump). See [docs/keybindings.md](docs/keybindings.md) for the full reference.
*   **Non-Disruptive Scrolling:** Scrolling up to read conversation history prevents the viewport from snapping to the bottom when the agent streams a response, preserving your reading context.

---

## Interactive slash commands

Type `/help` or `/?` inside any CADE session to open the interactive command browser.

### Session
| Command | Description |
|---------|-------------|
| `/help` or `/?` | Open the full-screen command browser |
| `/info` | Show agent, model, mode, cwd, and version |
| `/agent` | Show current agent name and ID |
| `/agents` | List and switch agents (r = rename, d = delete) |
| `/new-agent` | Create a brand-new agent |
| `/rename <name>` | Rename the current agent |
| `/delete [name]` | Delete an agent by name or ID |
| `/pin` | Pin current agent to settings as default |
| `/new` | Start a fresh conversation on the current agent |
| `/resume` | Browse past conversations and switch to one |
| `/checkpoint [label]` | Save a checkpoint of the current working-tree state |
| `/tree` | Browse and restore checkpoints (fullscreen picker) |
| `/fork [label]` | Create a new conversation branched from a checkpoint |
| `/artifacts` | List stored artifacts (logs, diffs, reports) |
| `/exit` or `/quit` | Quit CADE |

### Model & Mode
| Command | Description |
|---------|-------------|
| `/model [provider/name]` | Interactive model picker, or switch directly |
| `/reasoning [none\|low\|medium\|high\|xhigh]` | Set reasoning effort level |
| `/toolset [default\|codex\|gemini]` | Show or switch toolset |
| `/mode [name]` | Show or set permission mode |
| `/plan` | Switch to read-only plan mode |
| `/todos` | Toggle visibility of the active plan / checklist |
| `/default` | Return to default permission mode |
| `/yolo` | Bypass all permission prompts (auto-approve all tools) |
| `/approve-always <pattern>` | Add a permanent allow rule for matching tools |
| `/deny-always <pattern>` | Add a permanent deny rule for matching tools |
| `/permissions` | Show current permission mode and rules |

### Memory
| Command | Description |
|---------|-------------|
| `/memory` | List all memory blocks |
| `/memory view <label>` | Show the full content of a memory block |
| `/memory set <label> <value>` | Set a memory block value |
| `/memory edit <label>` | Interactively edit a memory block |
| `/memory delete <label>` | Delete a memory block |
| `/memory history <label>` | Show last 5 revisions of a memory block |
| `/init` | Analyse project and populate memory |
| `/remember <text>` | Ask the agent to update memory with the given text |
| `/reflect [focus]` | Trigger reflection to extract memory from conversation history |

### Tools & Providers
| Command | Description |
|---------|-------------|
| `/backend [local\|docker\|ssh\|readonly]` | Show or switch execution backend |
| `/link` | Register and attach all tools to the current agent |
| `/unlink` | Detach all tools from the current agent |
| `/mcp` | Show MCP server status and tools |
| `/mcp reload` | Reload MCP servers from config |
| `/connect [preset]` | Connect a new AI provider interactively |
| `/disconnect <name>` | Remove a configured provider |
| `/providers` | List configured LLM providers |

### Skills & Subagents
| Command | Description |
|---------|-------------|
| `/skills` | List loaded skills |
| `/skills create <name>` | Scaffold a new skill |
| `/skills show <id>` | Show full detail for a skill |
| `/skills reload` | Reload skills from disk |
| `/subagents` | List available subagent definitions |
| `/<skill-id>` | Run a loaded skill directly (e.g. `/commit`, `/review`) |

### Diagnostics
| Command | Description |
|---------|-------------|
| `/search <query>` | Search message history |
| `/context` | Show current context window usage |
| `/usage` | Token usage for this session |
| `/cost` | Estimate API cost for this session |
| `/stats` | Full session stats — tokens, tool calls, timing |
| `/stats model` | Per-model breakdown: requests, input, cache, output |
| `/stream` | Toggle streaming mode on/off |
| `/hooks` | Show configured lifecycle hooks |
| `/feedback` | Report issues or give feedback |
| `/debug-last` | Dump the last assistant message as stored on the server |

### Misc
| Command | Description |
|---------|-------------|
| `/copy` | Toggle copy mode (disables mouse scroll for text selection) |
| `/export [file.json]` | Export the current agent state to a JSON file |
| `/clear` | Clear screen and context window |
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
    "openviking": {
      "command": "/path/to/openviking/.venv/bin/python",
      "args": ["/path/to/openviking/openviking_mcp.py"]
    }
  }
}
```

View connected servers and tools with `/mcp`.

---

## Hooks

User-defined shell scripts that fire at lifecycle events. Configure in `~/.cade/settings.json`:

Hooks are applied in both interactive CADE TUI sessions and headless CLI runs (`--prompt`, `--output-format json|stream-json`).

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
| `GET` | `/v1/stream` | Stream HTTP from a remote URL |

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

CADE is a Cargo workspace with twelve independent crates:

```
src/
├── main.rs                     # `cade` CLI entry point
├── lib.rs                      # Re-exports workspace crates as cade::*
└── bin/cade-server.rs          # `cade-server` entry point

crates/
├── cade-core/                  # Shared types (no crate deps)
│   └── permissions, settings, skills, hooks, toolsets
├── cade-ai/                    # LLM providers (no crate deps)
│   └── anthropic, openai, gemini, ollama, catalogue
├── cade-desktop/               # Desktop extensions (no crate deps)
│   └── capture, control, notify, tray
├── cade-server/                # HTTP API + SQLite (→ cade-core, cade-ai)
│   └── api/, storage/, config, crypto, rate_limit
├── cade-agent/                 # Client + tools (→ cade-core, cade-desktop)
│   └── agent/, tools/, mcp/, subagents/
├── cade-cli/                   # TUI + REPL (→ cade-core, cade-agent, cade-ai)
│   └── cli/, ui/
├── cade-mcp/                   # MCP server integration
├── cade-web/                   # Web search and scraping capabilities
├── cade-tui/                   # Standalone TUI component library
├── cade-plugin/                # Plugin loading and manifests
└── cade-sdk/                   # Rust SDK for programmatic agent control
```

See [`ARCHITECTURE.md`](ARCHITECTURE.md) for the full dependency graph, module
descriptions, and data flow diagrams.

---

## Security

See [`SECURITY.md`](SECURITY.md) for CADE's security model, threat assumptions,
configuration hardening options, and reporting guidance.

---

## Capability Profiles

CADE supports three capability profiles that control which tools and features
are available at runtime. The default is **full** (all features enabled).

| Profile | Includes | Use when |
|---------|----------|----------|
| **core** | Coding tools, memory, checkpoints | You want a lean, focused assistant |
| **pro** | Core + subagents + code intelligence | Serious coding on large repositories |
| **full** | Everything (desktop, web, MCP, SDK…) | You want maximum power (default) |

### Selecting a profile

```bash
# CLI flag
cade --profile pro

# Environment variable
export CADE_PROFILE=pro

# Settings file (~/.cade/settings.json)
{
  "profile": "pro",
  "enable_capabilities": ["web"],    // add individual capabilities
  "disable_capabilities": ["tray"]   // remove individual capabilities
}
```

### Build profiles

CADE can also be compiled with a reduced feature set for smaller binaries:

```bash
cargo build --release                                     # full (default)
cargo build --release --no-default-features --features pro   # no desktop/web/mcp/sdk
cargo build --release --no-default-features --features lean  # minimal core only
```

---

Built by [EzekTec Inc.](https://github.com/EzekTec-Inc) · Apache-2.0 / MIT
