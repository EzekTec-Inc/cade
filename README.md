# CADE — Your Intelligent Shell

[![License: MIT OR Apache-2.0](https://img.shields.io/badge/license-MIT%2FApache--2.0-blue.svg)](LICENSE-MIT)

**Coding AI-Assistant with Desktop Extensions (batteries included)**

CADE is a smart shell that turns your terminal into a full AI-powered development environment. It sees your screen, edits your files, remembers what you taught it yesterday, and runs entirely on your machine — no platform accounts, no cloud lock-in. Bring your harness to CADE and build your dreams.

```
You type:   "set up a new Rust web API with auth, tests, and Docker"
CADE does:  scaffolds → writes code → runs tests → builds Docker image → reports back
```

---

## Why CADE?

| What you get | How it works |
|---|---|
| **A shell that understands you** | Persistent memory across sessions — CADE remembers your project, preferences, and past decisions |
| **Batteries included** | Ships its own server, TUI, desktop control, MCP support, and 30+ built-in skills — nothing else to install |
| **Your machine, your rules** | Runs 100% locally. Your code never leaves your filesystem unless you choose to call an LLM API |
| **Any LLM, one interface** | Anthropic, OpenAI, Google Gemini, Ollama, OpenRouter — switch models mid-conversation with `/model` |
| **Desktop-aware** | Screenshots, window control, clipboard, notifications — CADE can see what you see |

---

## Quickstart — Up and Running in 2 Minutes

```bash
# 1. Clone and build
git clone https://github.com/EzekTec-Inc/CADE && cd CADE
cargo build --release

# 2. Start the server (pick your LLM provider)
ANTHROPIC_API_KEY=sk-ant-... ./target/release/cade-server

# 3. Open your smart shell
./target/release/cade
```

That's it. CADE auto-creates an agent, remembers it per directory, and you're ready to build. Type a message and hit Enter.

> 💡 **First thing to try:** Type `/init` — CADE scans your project and writes a starter memory block so it understands your codebase from the start.

📖 Full setup details (including Ollama, Windows, and optional dependencies) → [docs/getting-started.md](docs/getting-started.md)

---

## What Can CADE Do?

### 🔧 Core Coding Tools
| Tool | What it does |
|------|-------------|
| `bash` | Run any shell command — build, test, git, deploy |
| `read_file` | Read files with line numbers and optional ranges |
| `write_file` | Create or overwrite files (auto-creates parent dirs) |
| `edit_file` | Surgical string-replace edits — precise, diff-like |
| `grep` | Regex search across your entire codebase |
| `glob` | Find files by pattern, sorted by modification time |

### 🖥️ Desktop Extensions (the "D" in CADE)
| Tool | What it does |
|------|-------------|
| `desktop_screenshot` | Capture your screen or a specific window → base64 PNG |
| `desktop_list_windows` | List all visible window titles |
| `desktop_control` | Focus windows, type text, press keys, move the mouse |
| `desktop_notify` | Send OS desktop notifications |

### 🧠 Memory & Intelligence
| Tool | What it does |
|------|-------------|
| `update_memory` | Teach CADE facts that persist across sessions |
| `search_memory` | Search everything CADE knows — semantic + keyword hybrid |
| `load_skill` | Activate domain-specific knowledge (Rust, TypeScript, PDF, etc.) |
| `run_subagent` | Spawn parallel workers for complex tasks |
| `recall` | Federated search across all memory sources at once |

### 🔌 Extensible by Design
| Feature | What it does |
|---------|-------------|
| **MCP Servers** | Connect external tool servers (GitHub, Jira, databases, etc.) |
| **Skills** | Drop a Markdown file into `.cade/skills/` to teach CADE new domains |
| **Hooks** | Wire shell scripts into lifecycle events (before/after tool calls) |
| **Subagents & Teams** | Dispatch work to specialized agent teams that run in parallel |

---

## The Terminal Experience

CADE's TUI is built for speed and comfort — no flickering, no clutter:

- **Flicker-free rendering** — CSI 2026 synchronized output for atomic screen updates
- **Smart paste** — Large pastes auto-collapse into compact markers, expanded transparently for the LLM
- **Tab completion** — Path completion and `@` fuzzy file picker
- **Multi-line input** — `Shift+Enter` for newlines, `Enter` to send
- **Undo/Redo** — `Ctrl+Z` / `Ctrl+Y` in the input field (100 levels)
- **Non-disruptive scroll** — Read history while the agent streams without losing your place
- **Native themes** — Drop any `.tmTheme` file into `~/.cade/themes/` for instant UI skinning
- **Bash shortcuts** — `!command` sends output to the LLM; `!!command` runs locally only

📖 Full keybinding reference → [docs/keybindings.md](docs/keybindings.md)

---

## Slash Commands — Your Control Panel

Type `/help` inside any session to browse all commands. Here are the essentials:

| Command | What it does |
|---------|-------------|
| `/model` | Switch LLM models interactively |
| `/memory` | View and edit persistent memory blocks |
| `/skills` | Manage loaded skills |
| `/init` | Scan your project and populate memory |
| `/checkpoint` | Save your working tree before risky changes |
| `/mcp` | Interactive picker to manage MCP servers |
| `/link` | Sync and re-register tools to active session |
| `/yolo` | Auto-approve everything (when you trust the task) |
| `/plan` | Read-only mode — explore without changing anything |
| `/cost` | See what you've spent this session |
| `/context` | Visualise context window usage |
| `/backend [name]` | Switch execution backend (local, docker, ssh, readonly) |

📖 Complete reference → [docs/slash-commands.md](docs/slash-commands.md)

---

## CLI Usage

```bash
cade                              # Interactive smart shell (resumes last agent)
cade --new                        # Fresh conversation, same agent
cade --new-agent                  # Brand new agent
cade -p "fix the auth bug"        # Headless one-shot (no TUI)
cade -m anthropic/claude-sonnet-4-5  # Pin a specific model
cade --yolo                       # Auto-approve all tool calls
cade --permission-mode plan       # Read-only exploration
```

### Permission Modes

| Mode | What happens |
|------|-------------|
| `default` | Asks before write/execute operations |
| `acceptEdits` | Auto-approves file edits, asks for everything else |
| `plan` | Read-only — all writes blocked |
| `bypassPermissions` | Auto-approves everything (`--yolo`) |

### Headless Output Formats

```bash
cade -p "..." --output-format text         # Plain text (default)
cade -p "..." --output-format json         # Structured JSON
cade -p "..." --output-format stream-json  # SSE JSON stream
```

---

## Toolsets

CADE auto-detects the best editing style for your model:

| Toolset | Models | Edit style |
|---------|--------|------------|
| `default` | Claude, Llama, Mistral | String-replace (`edit_file`) |
| `codex` | GPT, o1, o3, o4 | Unified diff (`apply_patch`) |
| `gemini` | Gemini | String-replace (`edit_file`) |

Override with `--toolset <name>` or `/toolset <name>`.

---

## Session Persistence — CADE Remembers

CADE stores state per-project so you pick up right where you left off:

| File | What it holds |
|------|--------------|
| `.cade/session.json` | Agent ID, conversation, run state (gitignored) |
| `.cade/settings.json` | Project-specific config (commit this) |
| `~/.cade/settings.json` | Global preferences |
| `~/.cade/cade.db` | All agents, messages, memory (encrypted at rest) |

---

## Advanced Features

- **Intelligent Tool Selection (ITS)** — A local ONNX cross-encoder reranks and filters tools before each LLM call, so the model only sees what's relevant. [Learn more →](docs/intelligent-tool-selection.md)
- **Semantic Memory Search** — Hybrid keyword + cosine similarity search via local embeddings (`fastembed` + `sqlite-vec`), merged with Reciprocal Rank Fusion.
- **Cost Guardrails** — Set `CADE_MAX_SESSION_COST_USD=2.00` to auto-stop when spend crosses your threshold. [Learn more →](docs/cost-and-pricing.md)
- **Execution Backends** — Run tools locally, inside Docker containers, or over SSH. [Learn more →](docs/execution-backends.md)
- **WASM Dashboard** — Visit `http://localhost:8284/dashboard` for a browser-based view of your agents. [Learn more →](docs/gui-dashboard.md)

---

## Documentation

📖 **[Full documentation →](docs/index.md)**

| Getting started | Deep dives |
|---|---|
| [Installation & first session](docs/getting-started.md) | [Memory system](docs/memory-system.md) |
| [Usage guide](docs/usage-guide.md) | [Skills](docs/skills.md) |
| [Configuration](docs/configuration.md) | [Subagents & teams](docs/subagents.md) |
| [Slash commands](docs/slash-commands.md) | [MCP servers](docs/mcp-servers.md) |
| [Keybindings](docs/keybindings.md) | [Hooks](docs/hooks.md) |
| [Themes](docs/themes.md) | [Permissions](docs/permissions.md) |

---

## Building from Source

```bash
# Prerequisites: Rust 1.85+ (Edition 2024)
# Optional Linux deps for desktop extensions:
sudo apt install libpipewire-0.3-dev libclang-dev libgbm-dev xdotool

git clone https://github.com/EzekTec-Inc/CADE && cd CADE
cargo build --release
```

For a leaner binary without semantic search (~50MB smaller):
```bash
cargo build --release -p cade-store --no-default-features --features bundled-sqlite
cargo build --release
```

📖 Windows users → [WINDOWS_SETUP.md](WINDOWS_SETUP.md)

---

## Architecture

CADE is a Cargo workspace of 16 crates with a strict downward dependency graph:

```
src/          → CLI binary + server binary (entry points)
cade-core     → Shared types, permissions, skills, hooks
cade-ai       → LLM providers (Anthropic, OpenAI, Gemini, Ollama)
cade-store    → SQLite persistence, AES-GCM encryption, embeddings
cade-server   → Axum HTTP API, context building, consolidation
cade-agent    → Tool implementations, subagents, teams
cade-cli      → TUI setup, REPL, slash commands
cade-tui      → Ratatui terminal UI components
cade-gui      → WASM dashboard (egui/eframe)
cade-mcp      → Model Context Protocol integration
cade-desktop  → Screen capture, window control, notifications
```

📖 Full architecture guide → [docs/architecture.md](docs/architecture.md)

---

## Contributing

```bash
cargo build --workspace
cargo clippy --workspace -- -D warnings
cargo test --workspace
cargo fmt --all -- --check
```

📖 Developer setup → [docs/development.md](docs/development.md)

---

## License

Dual-licensed under MIT and Apache 2.0. See [LICENSE-MIT](LICENSE-MIT) and [LICENSE-APACHE](LICENSE-APACHE).
