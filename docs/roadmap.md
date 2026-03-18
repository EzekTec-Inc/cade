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

### Terminal UI
- [x] Ratatui-based TUI with CSI 2026 synchronized output
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

### Security
- [x] Opt-in filesystem sandbox (`CADE_FS_ROOT`)
- [x] Path traversal protection in `apply_patch`
- [x] Headless output sanitization (ANSI injection prevention)
- [x] Constant-time auth comparison
- [x] Secure random DB encryption key (`.cade-db.key`)
- [x] 0600 permissions on settings files

### Architecture
- [x] Cargo workspace split (6 crates)
- [x] LLM providers extracted to `cade-ai`
- [x] Clean acyclic dependency graph
- [x] Dead legacy code removed (~25K lines)

---

## Planned

### Short Term
- [ ] **Test coverage expansion**: Unit tests for server, storage, LLM providers, and tools
- [ ] **CI/CD pipeline**: GitHub Actions for build, test, and release
- [ ] **`cade-tui` extraction**: Separate Ratatui rendering into its own crate
- [ ] **`cade-mcp` extraction**: Isolate MCP client from `cade-agent`

### Medium Term
- [ ] **Plugin system**: Dynamic tool loading via shared libraries or WASM
- [ ] **Multi-agent collaboration**: Named agents with message passing
- [ ] **Web UI**: Browser-based interface alongside the terminal
- [ ] **Windows/macOS support**: Cross-platform desktop extensions
- [ ] **Session replay**: Re-watch recorded coding sessions

### Long Term
- [ ] **Self-improvement loop**: Agent learns from past sessions automatically
- [ ] **Team features**: Shared agents, memory, and skills across a team
- [ ] **IDE integration**: VS Code / JetBrains extensions
