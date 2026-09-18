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
- DeepSeek — `DEEPSEEK_API_KEY=sk-...`
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
(HTTP server). The workspace `default-members` include both binaries, so a normal
`cargo build --release` keeps the client and server in sync. Restart any running
`cade-server` after rebuilding provider code.

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

## 5. Practical Walkthrough Examples

### Example A: Interactive Coding & Auto-Verification
Ask CADE to inspect code, make surgical edits, and prove correctness:
```text
> Find all unused imports in crates/cade-core/src/lib.rs, remove them, and run cargo check
```
CADE executes an autonomous tool turn:
1. `read_file(path="crates/cade-core/src/lib.rs")` — inspects AST structure.
2. `edit_file(path="crates/cade-core/src/lib.rs", old_string="...", new_string="...")` — cleans imports.
3. `bash(command="cargo check")` — verifies clean compilation.
4. Reports final diff and compiler exit code directly in the terminal.

### Example B: Structured Multi-Step Tasks with the Plan Panel
When giving CADE complex, multi-stage requests:
```text
> Refactor the error handling in crates/cade-plugin, add unit tests, and verify clippy
```
CADE immediately initializes an interactive plan checklist rendered in the TUI:
```text
╭─ Tasks (1 of 3 completed) ─────────────────────────────────────────────────╮
│  ✓ 1. Add Error::IntegrityError to crates/cade-plugin/src/error.rs         │
│  ● 2. Write regression tests in crates/cade-plugin/src/tests.rs            │
│    3. Run cargo clippy -- -D warnings and format codebase                  │
╰────────────────────────────────────────────────────────────────────────────╯
```
Press **`Ctrl+T`** at any time to toggle the plan checklist on or off.

### Example C: Seamlessly Switching Between Cloud & Local Models
Pivoting between reasoning frontiers and offline local models is instantaneous:
```bash
# Switch to Claude 3.5 Sonnet for complex architecture reasoning
/model anthropic/claude-sonnet-4-5

# Switch to local Ollama for zero-cost, private offline coding
/model ollama/qwen2.5-coder:7b
```

### Example D: Launching and Using the Web Dashboard
CADE serves a reactive WASM dashboard alongside the terminal shell:
1. Open `http://localhost:8284/dashboard` in your browser.
2. Press **`Cmd+K`** or **`Ctrl+K`** to open the Global Command Palette.
3. Switch views:
   - **Model Arena**: Compare two models side-by-side with live token speed gauges.
   - **Workflows DAG**: View visual dependency graphs and click **Run Pipeline**.
   - **Memory Studio**: Inspect live Knowledge Graph Triples from SQLite.

## 6. Common next steps

- **Set project context** — `/init` writes a starter `project` memory block
  by inspecting the current directory.
- **Add an MCP server** — see [mcp-servers.md](mcp-servers.md).
- **Cap your spend** — add `"max_session_cost_usd": 2.00` to
  `.cade/settings.json` (committable, shared with your team) or run
  `export CADE_MAX_SESSION_COST_USD=2.00` to abort the agentic loop once
  cumulative cost crosses $2. Full list in
  [configuration.md](configuration.md).
- **Open the WASM dashboard** — visit `http://localhost:8284/dashboard` while
  the server is running. Details in [gui-dashboard.md](gui-dashboard.md).

## 6. Where to next

- New to the codebase? Start with [architecture.md](architecture.md).
- Want a specific command? See [slash-commands.md](slash-commands.md).
- Something not working? Check `~/.cade/cade.log` and the
  [hooks.md](hooks.md) doc for `SessionStart` failures.
