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
- [x] SQLite persistence with encrypted fields (AES-GCM)
- [x] Per-agent rate limiting
- [x] Provider hot-reload via `/connect`
- [x] Live model listing from all providers
- [x] Streamable HTTP capability (`/v1/stream`)
- [x] Background runs (`/v1/runs/:id`) for long detached tasks

### Security (2026 Apr)
- [x] Opt-in filesystem sandbox (`CADE_FS_ROOT`)
- [x] Path traversal protection in `apply_patch`
- [x] Headless output sanitization (ANSI injection prevention)
- [x] Constant-time auth comparison
- [x] Secure random DB encryption key (`~/.cade/db.key`)
- [x] 0600 permissions on settings files
- [x] Always-on path protection (denies writes to `.git`, `.ssh`, `.env`, DB key — even in YOLO)
- [x] Mandatory bearer-token auth with auto-generated token
- [x] Global request body size limit (8 MiB)
- [x] SSRF proxy lockdown
- [x] Argon2id key derivation
- [x] SSH `StrictHostKeyChecking=yes` + cwd shell quoting
- [x] Origin header CSRF check
- [x] Generic 500 error responses (no internal details leaked)
- [x] `cargo-audit` CI workflow

### Architecture
- [x] Cargo workspace split (17 crates)
- [x] LLM providers extracted to `cade-ai`
- [x] `cade-tui` extracted as standalone TUI library
- [x] `cade-mcp` isolated from `cade-agent`
- [x] `cade-gui` (WASM dashboard) added
- [x] Clean acyclic dependency graph
- [x] Dead legacy code removed (~25K lines)
- [x] Rust Edition 2024, resolver 3
- [x] rust10x compliance (Tier 1–4 fixes)

### IDE & Cross-platform
- [x] `cade-ide-mcp` bridge with TCP loopback adapter transport
- [x] Neovim adapter (`cade-neovim`)
- [x] VS Code adapter (`cade-vscode`)
- [x] JetBrains adapter (`cade-jetbrains`)
- [x] Cross-platform desktop tools (Linux, macOS, Windows)
- [x] Linux Wayland + X11 screen capture, window control, notifications

### GUI Dashboard
- [x] WASM dashboard served at `/dashboard` (rust-embed)
- [x] Sidebar + timeline + input bar (TUI-matched)
- [x] SSE streaming with pure-Rust parser (native-testable)
- [x] Slash-command palette (Ctrl+P)
- [x] Memory editor, checkpoints, artifacts, MCP/tools/skills overlays
- [x] Inline question widget (replaces blocking modals)
- [x] Auto-scroll with manual ↓ button + scroll-velocity heuristic

### Cost Optimisation (2026 Apr)
- [x] **P1** — `skills` block as `system_static` cache anchor (~90% input saving on payload)
- [x] **P2** — full cache_read / cache_write accounting in `AgentMetrics`
- [x] **P3** — auto-cheapest compaction model resolver per provider
- [x] **P4** — `CADE_MAX_SESSION_COST_USD` hard $-cap
- [x] **P5** — `compress_tool_schema` for unused non-pinned tools (~75% byte reduction)
- [x] **P6** — `CADE_TOOL_TURN_MAX_TOKENS` per-turn output cap on tool dispatch
- [x] **P7** — `CADE_GEMINI_CACHE_TTL_SECS` adaptive TTL
- [x] **P8** — `tool_executions.output_chars` column for per-call cost telemetry

### Quality
- [x] Test coverage: 1,501+ tests across the workspace
- [x] CI/CD pipeline: GitHub Actions (build, test, release)
- [x] Plugin system: dynamic loading via shared libraries / WASM
- [x] Multi-agent collaboration: named agents with message passing
- [x] Self-improvement loop: reflection subagent updates memory automatically
- [x] Session replay: timeline export
- [x] **Clippy clean** under `-D warnings` across the entire workspace
- [x] **TUI IME — hardware cursor sync (Phase 1)** — implemented in
      `crates/cade-tui/src/app/mod.rs`; `MoveTo` + `Show`/`Hide` is
      emitted after every frame so OS IMEs spawn their candidate window
      at the correct text cursor position

---

## Recently Completed

### Subagent System Refactoring (2026 May)
- [x] **Phase 1** — Isolated server subagent memory via ephemeral DB rows
- [x] **Phase 2** — Extracted `SubagentConfig` for shared arg parsing/validation
- [x] **Phase 3** — Split `run.rs` monolith (4,485 lines) into `run/` submodules

### GUI & TUI Polish (2026 May)
- [x] **Mobile / responsive dashboard** — viewport breakpoints, drawer sidebar,
      adaptive overlays, touch gestures (WI-9)
- [x] **Subagent tracker cards** — glass-style cards with live tool names
- [x] **Input field fixes** — word wrap, newline insertion, horizontal overflow
- [x] **Question modal polish** — excess padding removed, multiline height fix

See `IMPLEMENTATION_PLAN.md` for detailed commit history.

---

## Planned

### Near term (all shipped ✅)
- [x] **TUI overlay stack migration** (WI-2)
- [x] **TUI slot rendering** (WI-3)
- [x] **System prompt optimization** (WI-7)
- [x] **Semantic memory search** (WI-6)
- [x] **Mobile / responsive dashboard** (WI-9)

### Medium term
- [x] **StorageBackend trait extraction** — unify CLI (HttpTransport) and server
      (direct DB) meta-tool handlers behind a shared async trait. Eliminates
      ~1,500 lines of duplication across `cade-agent/src/tools/runtime/` and
      `cade-server/src/server/api/run/meta_tools.rs`.
- [ ] **Benchmark suite** — latency and throughput benchmarks for LLM streaming,
      tool dispatch, and memory operations (criterion.rs)
- [ ] **Structured logging** — migrate from `tracing` ad-hoc spans to
      consistent structured fields across all crates

### Long term
- [ ] **Team features** — shared agents, memory, and skills across a team.
      Requires multi-tenant DB schema, RBAC, and agent-to-agent auth.
- [ ] **Voice mode** — speech-to-text input + audio output. Likely via
      WebRTC (browser) or local Whisper (CLI). Needs streaming TTS for
      natural response delivery.
- [ ] **Plugin marketplace** — discover, install, and rate community skills
      and MCP server configs from a central registry
