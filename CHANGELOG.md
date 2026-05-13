# Changelog

All notable changes to CADE are documented here.
Format follows [Keep a Changelog](https://keepachangelog.com/en/1.0.0/); versions follow [Semantic Versioning](https://semver.org/).

---

## [Unreleased]

### Changed

#### Connection pooling (P0-B)
- Replaced `Db = Arc<parking_lot::Mutex<rusqlite::Connection>>` with `Db = r2d2::Pool<SqliteConnectionManager>` for real concurrent reads/writes through a managed pool
- Added `r2d2` 0.8 and `r2d2_sqlite` 0.24 as workspace dependencies (rusqlite 0.31 compatible); `bundled-sqlite` feature now also drives `r2d2_sqlite/bundled`
- Pool defaults: `max_size = 8` for file-backed databases, `max_size = 1` for `:memory:` (so all callers share the same DB), `connection_timeout = 30s`
- Per-connection `PRAGMA journal_mode=WAL; PRAGMA foreign_keys=ON;` is now applied through `SqliteConnectionManager::with_init`, so every pooled connection gets the same setup — not only the first one
- Added `cade_store::Error::R2d2(r2d2::Error)` variant; the cade-server `IntoResponse for Error` maps it to a 5xx response
- All 180 `db.lock()` / `state.db.lock()` / `self.state.db.lock()` call sites across 21 files migrated to `db.get()` (fallible) — most propagate via `?`, four infallible memory helpers (`bump_block_access`, `recall_chunks`, `rechunk_block`, `stamp_provenance`) log + early-return on pool errors, and `get_tool_id_by_name` uses `db.get().ok()?`
- Removed direct `parking_lot` dependency from cade-store (still transitively used elsewhere)

### Added

#### Semantic Memory Search
- Hybrid memory search combining keyword (LIKE), fuzzy word-match, and cosine similarity via `fastembed` + `sqlite-vec`, merged with Reciprocal Rank Fusion (k=60)
- Local text embeddings using AllMiniLML6V2 (384-dim, ~50MB ONNX model, downloaded on first use)
- `sqlite-vec` virtual tables (`vec_memory_blocks`, `vec_archival_memory`, `vec_messages`) for vector similarity search
- Feature-gated behind `--features semantic-search` to keep default binary lean
- Embeddings auto-computed on memory block write
- Migration 8: vec0 virtual tables with graceful fallback when extension unavailable

#### Memory System Improvements (P1–P8)
- **P1**: Observation capture — records tool calls with importance scoring (1–5 scale) and injects high-signal observations into agent context
- **P3**: Event-driven consolidation priority — consolidation triggers based on context pressure, not just turn count
- **P4**: Structured session handoff — `/new` builds a handoff summary so the next conversation inherits key state
- **P5**: Consolidation fidelity tuning — improved summarization quality during auto-compaction
- **P6**: Auto-type memory blocks on write — infers `memory_type` from content heuristics (decision, constraint, convention) for confidence boost
- **P7**: Auto-update `active_goal` during consolidation — ensures task state survives context rotation
- **P8**: Prune stale observations during consolidation — keeps observation trail compact

#### TUI Refactoring (Phases 2–4)
- **Phase 2**: `EditorComponent` trait — pluggable editor with `DefaultEditor` wrapper; `TuiApp.editor` is now `Box<dyn EditorComponent>`
- **Phase 3**: Dynamic overlay stack — `Vec<Box<dyn OverlayComponent>>` replaces 4 legacy `Option<...>` fields (summary, command palette, theme picker, file picker); -333 lines
- **Phase 4**: UI extension slots — `SlotManager` with `Header`, `Footer`, `Sidebar` regions; render + input fully wired

#### Askpass Integration
- `cade-askpass` crate: IPC server with token-based authentication for SSH/GPG password prompts
- Protocol layer (`protocol.rs`) with `RequestMessage`/`ResponseMessage` envelopes
- Tokio-based Unix domain socket server (`server.rs`)
- TUI password modal wired into `BashTool` via `SSH_ASKPASS` environment variable
- 23 tests (17 lib + 6 integration)

#### TUI Rendering
- Accurate Markdown rendering via pulldown-cmark AST parser with viewport-aware widths
- Table rendering with proper column alignment and Unicode borders
- Code block text wrapping for long lines
- Image alt-text display in Markdown blocks
- Content overflow prevention in viewport

#### System Prompt
- Dynamic tool filtering note — explains Intelligent Tool Selection to the agent
- Search-first lookup guidance — prefer `semantic_search` (~50 tokens) over blind grep (~2000+ tokens)
- `/memory pin` guidance — tells agent how to keep critical blocks permanently active
- Capability-gated prompt fragments — strip guidance for disabled capabilities to save tokens

#### Other
- `SlotComponent` trait with `render()`, `handle_input()`, `preferred_height()` for plugin widgets
- `OverlayComponent` trait with `OverlayInputResult` enum for modal dispatch
- Shared memory blocks (Phase 1) — `shared_memory_blocks` + `agent_memory_blocks` tables
- Mandatory planning in system prompt + `Ctrl+T` plan toggle in TUI

### Changed
- Replaced the `esc to interrupt` prompt text with `Ctrl+c to interrupt` in the REPL's thinking animation loop for better clarity
- Updated `Cargo.lock` and `Cargo.toml` dependencies via `cargo update` to address security vulnerabilities in `ring` (RUSTSEC-2025-0009) and `rustls-webpki` (RUSTSEC-2026-0099)
- MCP prefix stripping for edit tracking — `strip_mcp_prefix` + `is_file_edit_tool` ensures subagent file edits are recorded regardless of MCP server prefix
- Meta-tools now route through intercept in subagent loop for consistent handling
- Theme picker: fixed Enter key on empty filter, removed `q` as close key (conflicted with typing), live preview uses `builtin_by_name()` registry
- Ctrl+T matches both Kitty-protocol and legacy VT forms

### Fixed
- **Deadlock in `search_memory()`**: `parking_lot::Mutex` held while fuzzy fallback tried to re-acquire same lock — scoped lock acquisition to release before fallback
- **Blocking DB calls in async context**: Wrapped meta-handlers (`handle_search_memory_meta`, `handle_archival_memory_search_meta`, `handle_query_event_log_meta`) and HTTP endpoints with `tokio::task::spawn_blocking()` + 10s timeout
- **Missing HTTP client timeouts**: Added `timeout(30s)` + `connect_timeout(10s)` to `HttpTransport` in cade-agent
- Dual-store file corruption where `SessionStore` and `SettingsManager` overwrote each other in `.cade/settings.local.json` — session store now uses dedicated `.cade/session.json`
- `/agents` REPL command not updating the project's local session file
- `--agent` and `--name` CLI flags now properly persist the user's explicit selection
- Missing cross-syncing between global last agent and local session agent during agent resolution
- UTF-8 safe truncation in server message handling
- Full DB persist for skill cache invalidation
- Subagent file edit tracking for MCP-prefixed tool names
- Table and content overflow in TUI viewport
- `Send + Sync` bounds on `OverlayComponent` and `SlotComponent` traits
- 12 Clippy lints across cade-server/cli/gui

### Removed
- Unused `unicode_width::UnicodeWidthStr` import from TUI
- Stale `drift_check.rs` debug script
- `TOOL_RESPONSE_RULE` duplication in system prompt

---

## [0.2.0] — 2026-03-07

### Added
- `X-Cade-Version` response header on every server response (version middleware in `cade-server`)
- Client-side version mismatch warning at startup — logs a warning when client and server versions differ
- `GET /v1/health` and `GET /v1/config` responses now include a `version` field (`CARGO_PKG_VERSION`)
- `created_at` (ISO-8601) field in `AgentResponse` — clients can now sort agents by creation time
- `server_version()` method on `CadeClient` for programmatic version queries

### Changed
- Hook pattern matching now uses the `regex` crate — full regex syntax supported (anchoring, character classes, groups, alternation with `|`, etc.). Invalid patterns fall back to case-insensitive substring match for backwards compatibility.
- Startup log messages (`"cade-server not running — starting…"` and `"cade-server ready."`) converted from `eprintln!` to `tracing::info!` for consistent log filtering via `RUST_LOG`

### Fixed
- `AgentRow` and all SQL queries now correctly read `created_at` from the database (previously the field was written but never read back)

---

## [0.1.0] — 2026-03-04

### Added
- Initial release of CADE (Coding AI assistant with Desktop Extensions)
- Multi-provider LLM support: Anthropic Claude, OpenAI GPT/o-series, Google Gemini, Ollama (local)
- Persistent agent state with SQLite backend
- TUI REPL with streaming responses, tool call display, reasoning collapse
- Tool suite: `bash`, `read_file`, `write_file`, `edit_file`, `grep`, `glob`, `apply_patch`, desktop tools
- Skills system: scoped SKILL.MD files with frontmatter, live file watcher, trigger-based auto-activation
- Subagent support with configurable concurrency cap (`CADE_MAX_SUBAGENTS`)
- MCP server integration with automatic reconnect on crash
- Agent export/import (`--export-agent` / `--import-agent`, `/export`, `/import`)
- FTS5-backed semantic message search (`/search`)
- Per-agent token-bucket rate limiter on inference endpoints
- LLM retry with exponential backoff (3 attempts, 2× multiplier, 8 s cap)
- Dynamic context budget per model from catalogue (`context_window_for_model`)
- Headless mode with `--timeout-secs` global timeout and parallel tool dispatch
- Configurable server port via `CADE_SERVER_PORT` env var, respected by both client and server
- Approval modal for tool calls with session-allow rules
- `/stats` command with per-model token usage breakdown
- `/skills` command: list, show, create, edit, delete, reload
