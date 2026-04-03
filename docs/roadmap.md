# CADE Roadmap

## Completed (v0.2.0)

### Core Agent
- [x] Multi-provider LLM support (Anthropic, OpenAI, Gemini, Ollama)
- [x] OpenAI Responses API for reasoning models (o1, o3, o4, gpt-5)
- [x] Gemini thought_signature support for reasoning models
- [x] Model-specific toolsets (Default, Codex, Gemini)
- [x] Reasoning effort control (`/reasoning` command)
- [x] Auto-compaction: summarize old turns when context ≥ 98%
- [x] Subagent runner with seed memory and result writeback
- [x] MCP server integration with hot-reload
- [x] Heuristic Evaluator Layer for strict intent, safety, and pathfinding checks

### Terminal UI
- [x] Ratatui-based TUI with CSI 2026 synchronized output
- [x] Modern typography-driven timeline viewport
- [x] Native TextMate `.tmTheme` parsing for 100% accurate colorscheme syncing
- [x] Editor component with undo/redo (100 levels, coalesced)
- [x] Bracketed paste support (large pastes collapse to markers)
- [x] Tab path completion and `@` fuzzy file picker
- [x] Markdown rendering via pulldown-cmark AST parser
- [x] Live-streaming bash output in viewport
- [x] Non-disruptive scroll during streaming
- [x] Image paste via Ctrl+V (clipboard → LLM vision)
- [x] Message queue during agent turns
- [x] 60 FPS render throttle, async event buffering (R-01..R-04)

### Server
- [x] Axum REST API with full agent lifecycle
- [x] SQLite persistence with encrypted fields
- [x] Per-agent rate limiting
- [x] Provider hot-reload via `/connect`
- [x] Live model listing from all providers
- [x] Streamable HTTP capability (`/v1/stream`)

### Security
- [x] Opt-in filesystem sandbox (`CADE_FS_ROOT`)
- [x] Path traversal protection in `apply_patch`
- [x] Headless output sanitization (ANSI injection prevention)
- [x] Constant-time auth comparison
- [x] Secure random DB encryption key (`.cade-db.key`)
- [x] 0600 permissions on settings files

### Architecture
- [x] Cargo workspace split (12 crates)
- [x] LLM providers extracted to `cade-ai`
- [x] Clean acyclic dependency graph
- [x] Dead legacy code removed (~25K lines)

---

## Planned

### Short Term
- [x] **Test coverage expansion**: 295 tests across workspace (server, storage, LLM providers, tools)
- [x] **CI/CD pipeline**: GitHub Actions for build, test, and release
- [x] **`cade-tui` extraction**: Separate Ratatui rendering into its own crate
- [x] **`cade-mcp` extraction**: Isolate MCP client from `cade-agent`
- [x] **Rust Edition 2024**: Workspace upgraded to Edition 2024, resolver 3
- [x] **rust10x compliance**: Tier 1–4 fixes (lint guards, regions, dependency sections, macro imports)

### Medium Term
- [x] **Plugin system**: Dynamic tool loading via shared libraries or WASM
- [x] **Multi-agent collaboration**: Named agents with message passing
- [ ] **Web UI**: Browser-based interface alongside the terminal
- [x] **Windows/macOS support**: Cross-platform desktop extensions
- [x] **Session replay**: Re-watch recorded coding sessions

### Long Term
- [x] **Self-improvement loop**: Agent learns from past sessions automatically
- [ ] **Team features**: Shared agents, memory, and skills across a team
- [ ] **IDE integration**: VS Code / JetBrains extensions
