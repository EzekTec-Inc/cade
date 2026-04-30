# Getting Started

Up and running in five minutes.

## 1. Prerequisites

- Rust toolchain (1.85+ required — Edition 2024) — `rustup` from <https://rustup.rs>
- An LLM API key (one of):
  - Anthropic — `ANTHROPIC_API_KEY=sk-ant-...`
  - OpenAI — `OPENAI_API_KEY=sk-...`
  - Google Gemini — `GOOGLE_API_KEY=...`
  - Local Ollama — no key needed; just have `ollama` running
- Optional Linux extras for screen capture / window control:
  ```bash
  sudo apt install libpipewire-0.3-dev libclang-dev libgbm-dev xdotool
  ```

## 2. Build

```bash
git clone https://github.com/EzekTec-Inc/CADE
cd CADE
cargo build --release
```

The release binary is `target/release/cade` (CLI) and `target/release/cade-server`
(HTTP server).

### Optional: Semantic Memory Search

To enable local embedding-based memory search (hybrid keyword + cosine similarity):

```bash
cargo build --release --features semantic-search
```

This adds ~50MB to the binary (bundles an ONNX model for AllMiniLML6V2 embeddings). The model downloads automatically on first use. Without this flag, memory search uses keyword matching only — still fully functional.

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
