# CADE

**Coding AI assistant with Desktop Extensions**

A stateful, self-improving Rust CLI coding agent built on the [Letta](https://letta.com) platform. CADE gives an AI agent full access to your local development environment — including your desktop.

---

## Features

### Core coding tools
| Tool | Description |
|------|-------------|
| `bash` | Execute shell commands (build, test, git, etc.) |
| `read_file` | Read files with line numbers and optional offset/limit |
| `write_file` | Write files, auto-creating parent directories |
| `edit_file` | Str-replace editing — precise, diff-like changes |
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

---

## Quickstart

```bash
# Set your Letta API key
export LETTA_API_KEY=your_key_here

# Build
cargo build --release

# Run (creates a new agent on first launch)
./target/release/cade

# Headless prompt
cade -p "What files are in the current directory?"

# New agent
cade --new

# Resume specific agent
cade --agent agent-abc123
```

---

## Usage

```
cade                         # Interactive REPL (auto-resumes last agent)
cade --new                   # Create a new agent
cade --agent <id>            # Use a specific agent
cade -p "..."                # Headless prompt
cade -m <model>              # Specify model
cade --yolo                  # Bypass all permission prompts
cade --permission-mode plan  # Read-only mode
cade --tray                  # Start with system tray
cade --info                  # Show session info
```

### Permission modes

| Mode | Behaviour |
|------|-----------|
| `default` | Prompts for approval on each tool call |
| `acceptEdits` | Auto-approves file write/edit only |
| `plan` | Read-only — blocks bash/write/edit |
| `bypassPermissions` | Auto-approves everything (`--yolo`) |

### Interactive slash commands

```
/help       — show commands
/agent      — show current agent ID
/info       — show session info
/yolo       — disable permission prompts
/plan       — enable read-only mode
/clear      — clear the screen
/exit       — quit
```

---

## Session Persistence

CADE automatically remembers the last agent per directory:

- **Local** (`.cade/settings.local.json`, gitignored): last agent for this project
- **Global** (`~/.cade/settings.json`): API keys, global last agent

---

## Skills

Place `SKILL.MD` files in a `.skills/` directory to give the agent domain knowledge:

```
.skills/
├── my-project/
│   └── SKILL.MD
└── deployment/
    └── SKILL.MD
```

Each `SKILL.MD` uses YAML frontmatter:

```markdown
---
name: Deployment Guide
description: How to deploy this project to production
category: DevOps
tags: [deploy, aws, lambda]
---

# Deployment Steps
...
```

Custom skills directory: `cade --skills /path/to/skills`

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
# Dependencies (for screen capture on Wayland)
sudo apt install libpipewire-0.3-dev libclang-dev libgbm-dev

# Debug build
cargo build

# Release binary
cargo build --release

# Install globally
cargo install --path .
```

---

## Self-hosting with cade-server

CADE ships its own server — no third-party dependencies required.

```bash
# 1. Set your LLM provider API key
export ANTHROPIC_API_KEY=sk-ant-...     # or OPENAI_API_KEY / GOOGLE_API_KEY

# 2. Start the CADE server (defaults to :8284)
cade-server

# 3. In another terminal — the CLI auto-connects to localhost:8284
cade
```

### Server env vars
| Variable | Default | Description |
|----------|---------|-------------|
| `CADE_SERVER_PORT` | `8284` | Port to listen on |
| `CADE_LLM_PROVIDER` | `anthropic` | `anthropic` \| `openai` \| `gemini` \| `ollama` |
| `CADE_DB_PATH` | `~/.cade/cade.db` | SQLite database path |
| `ANTHROPIC_API_KEY` | — | Required for Anthropic provider |
| `OPENAI_API_KEY` | — | Required for OpenAI provider |
| `GOOGLE_API_KEY` | — | Required for Gemini provider |
| `OLLAMA_BASE_URL` | `http://localhost:11434` | Ollama base URL |
| `CADE_API_KEY` | — | Optional auth token for the server |

### CLI env vars (client side)
| Variable | Default | Description |
|----------|---------|-------------|
| `CADE_API_KEY` | — | Auth token sent to cade-server |
| `CADE_SERVER_URL` | `http://localhost:8284` | cade-server URL |

### Use with a remote server
```bash
export CADE_SERVER_URL=http://my-server:8284
export CADE_API_KEY=my-token
cade
```

---

## Project Structure

```
src/
├── main.rs                  # Entry point
├── agent/
│   ├── client.rs            # Letta REST API client
│   ├── session.rs           # Project-level session persistence
│   └── tools.rs             # Tool registration with Letta
├── cli/
│   ├── args.rs              # CLI argument parsing (clap)
│   ├── repl.rs              # Interactive REPL + tool execution loop
│   └── headless.rs          # Headless -p mode
├── tools/
│   ├── bash.rs              # Shell execution
│   ├── fs.rs                # Read/Write/Edit
│   ├── search.rs            # Grep/Glob
│   ├── desktop.rs           # Desktop tool wrappers
│   └── manager.rs           # Tool dispatch registry
├── desktop/
│   ├── capture.rs           # Screen capture (xcap)
│   ├── control.rs           # Window/app control (xdotool/ydotool)
│   ├── notify.rs            # OS notifications (notify-rust)
│   └── tray.rs              # System tray (ksni)
├── permissions/             # Permission modes
├── settings/                # Settings management
└── skills/                  # SKILL.MD discovery
```

---

Built by [EzekTec Inc.](https://github.com/EzekTec-Inc) · Apache-2.0 / MIT
