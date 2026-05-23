# Getting Started

Up and running in five minutes.

## 1. Installation

The fastest way to get started is using the official quick-install scripts, which automatically detect your system, download the latest binaries from GitHub, configure your `PATH`, and launch CADE for the first time.

**Linux / macOS**
```bash
curl -fsSL https://raw.githubusercontent.com/EzekTec-Inc/CADE/master/install.sh | bash
```

**Windows**
Open PowerShell as an Administrator and run:
```powershell
iwr https://raw.githubusercontent.com/EzekTec-Inc/CADE/master/install.ps1 -useb | iex
```

## 2. API Keys

CADE requires access to a Large Language Model to operate. You must configure an API key for your preferred provider by setting an environment variable (or placing it in a `.env` file):

- Anthropic — `ANTHROPIC_API_KEY=sk-ant-...`
- OpenAI — `OPENAI_API_KEY=sk-...`
- Google Gemini — `GOOGLE_API_KEY=...`
- Local Ollama — no key needed; just have `ollama` running

## 3. Building from Source (Alternative)

If you prefer to compile CADE from source, ensure you have the Rust toolchain (1.85+ required — Edition 2024).

```bash
# Optional Linux extras for screen capture / window control:
sudo apt install libpipewire-0.3-dev libclang-dev libgbm-dev xdotool

git clone https://github.com/EzekTec-Inc/CADE
cd CADE
cargo build --release
```

The release binaries are `target/release/cade` (CLI) and `target/release/cade-server`
(HTTP server).

### Semantic Memory Search (optional)

The default release build keeps the binary lean and uses keyword/fuzzy memory
search. To include local embedding-based ranking (fastembed + sqlite-vec), build
with the root `semantic-search` feature:

```bash
cargo build --release --features semantic-search
```

This adds the embedding dependencies and downloads the model on first use. If you
need the smallest binary, keep the default build.

## 3. First session

```bash
# Terminal 1 — start the server
ANTHROPIC_API_KEY=sk-ant-... ./target/release/cade-server

# Terminal 2 — open the TUI
./target/release/cade
```

You should see the welcome screen with a prompt input at the bottom.
Type a message and hit Enter.

## 4. Quick orientation

| Action | How |
|---|---|
| Open the slash-command palette | `Ctrl+P` |
| List all commands | `/help` |
| Switch model | `/model` (interactive picker) or `/model anthropic/claude-sonnet-4-5` |
| View memory blocks | `/memory` |
| Save a checkpoint before risky edits | `/checkpoint pre-refactor` |
| Quit | `/exit` or `Ctrl+C` twice |

## 5. Common next steps

- **Set project context** — `/init` writes a starter `project` memory block
  by inspecting the current directory.
- **Add an MCP server** — see [mcp-servers.md](mcp-servers.md).
- **Cap your spend** — `export CADE_MAX_SESSION_COST_USD=2.00` aborts the
  agentic loop once cumulative cost crosses $2. Full list in
  [configuration.md](configuration.md).
- **Open the WASM dashboard** — visit `http://localhost:8284/dashboard` while
  the server is running. Details in [gui-dashboard.md](gui-dashboard.md).

## 6. Where to next

- New to the codebase? Start with [architecture.md](architecture.md).
- Want a specific command? See [slash-commands.md](slash-commands.md).
- Something not working? Check `~/.cade/cade.log` and the
  [hooks.md](hooks.md) doc for `SessionStart` failures.
