# Changelog

All notable changes to CADE are documented here.
Format follows [Keep a Changelog](https://keepachangelog.com/en/1.0.0/); versions follow [Semantic Versioning](https://semver.org/).

---

## [Unreleased]

## [0.2.4] - 2026-06-11

### Added

- **Provider-Native Structured Completions:** Added native structured completions for OpenAI (`json_schema`), Anthropic (forced tool-use), and Gemini (`responseSchema` + `responseMimeType`) providers in `crates/cade-ai` with robust fallback.
- **Lightweight Virtual Sandboxing Backend:** Added secure virtual restricted local execution sandbox (`VirtualSandboxBackend` / `ExecutionBackendKind::Virtual`) in `crates/cade-agent` with strict path verification and environment-variable sanitization.
- **Webhook Workflow Router Execution:** Upgraded the `/v1/workflows/{workflow_name}` endpoint to dynamically load configurations from `.cade/workflows/{name}.json`, resolve/create CADE agents, conversations, and run standard background agentic execution loops.
- **CLI/UI Controls for Backend Selection:** Added support for `/backend virtual` to dynamically select and hot-swap between execution backends, and updated the TUI menu items.

### Fixed

- **Model Pricing & Floating-Point Accuracy Tests**: Resolved the pricing tests in `crates/cade-ai` for `gpt-4.1` and `gpt-5` model series, correcting token cost validation against the external `llm_providers` DB database precedence rules, and fixed the Anthropic Claude cache-read pricing test to use safe approximate floating-point comparisons to prevent precision failures.
- **OpenAI/OpenRouter Tool Compatibility Docs:** Documented OpenAI-compatible tool serialization around GPT-5-style Responses API flat function tools, `strict: false`, OpenAI's 128-tool cap, priority preservation for critical meta/MCP tools, and OpenRouter provider-prefix routing.
- **Input Field Rendering Artifacts & Cursor Drift:** Added `ratatui::widgets::Clear` pass on the input chunk inside `crates/cade-tui/src/app/render.rs` before rendering the input field, preventing ghosting artifacts, and refactored `clamped_visual_y` to compute relative `relative_visual_y` taking `tui-textarea`'s vertical scrolling viewport into account.
- **Centralized HTTP Connection Pooling:** Standardized and pooled outgoing connections across all first-party providers (`OpenAiProvider`, `AnthropicProvider`, `GeminiProvider`), utilizing a unified HTTP client built with standard keepalive (60s), connection timeout (15s), and stream timeout (120s) configurations to optimize connection reuse.
- **Cassette-Based (VCR) Mock Testing:** Developed an offline-capable, deterministic integration testing harness (`crates/cade-ai/src/vcr.rs`) with standard key, auth, and bearer token redaction, enabling cost-free, offline provider test execution in CI/CD pipelines.
- **Decoupled Embedding & Vector Indexes:** Exposed abstract `Embedder` and `VectorIndex` traits (`crates/cade-store/src/sqlite/embedding.rs`) to separate vector persistence from SQLite, making CADE's storage layer completely database-agnostic.
- **Hybrid Compile-Time Tools:** Created the strongly-typed `BuiltInTool` and `CoreToolAdapter` traits (`crates/cade-agent/src/tools/traits.rs`) to wrap high-performance local tools at compile-time with zero-copy JSON erasure, running them safely alongside the runtime dynamic MCP server dispatch loop.
- **Stateful TUI Autocomplete Controller:** Added reactive, live-filtering autocomplete overlays to the TUI input field (`crates/cade-tui`), utilizing `as_any_mut` upcasting to intercept and delegate editor keystrokes dynamically as the user types.
- **Automated Secret Scanning Workflow:** Configured a production-ready `.github/workflows/secret-scan.yml` workflow using `gitleaks` to automatically scan commits and PRs for API keys and credential leaks.

### Fixed

- **Eliminated TUI & Server Warnings:** Cleaned up compilation and Clippy lints across `test_rich.rs` (removed unused `super::*` imports) and `crates/cade-server/src/server/state.rs` (removed unnecessary `mut` keywords).
- **Hardened Autocomplete Boundary Slicing:** Clamped cursor offsets and word start indices safely to valid UTF-8 character boundaries inside `AutocompleteOverlay::update_suggestions`, entirely preventing multi-byte slicing panics.
- **Sanitized Tool Parse Errors:** Redacted and shielded raw serde deserialization and Rust trace internals from public-facing tool execution errors to prevent information disclosure.
- **Resolved Clippy Nesting Lints:** Resolved `clippy::collapsible_if` warnings in both `crates/cade-ai/src/catalogue.rs` and `crates/cade-tui/src/app/input.rs` using elegant and highly functional `and_then` combinations.

#### Mouse text selection works without any command
- Removed `EnableMouseCapture` from TUI startup so the terminal handles mouse events natively
- Users can now click-and-drag to select text and right-click (or Ctrl+Shift+C) to copy without typing `/mouse`
- Scroll-wheel capture is now opt-in via `/mouse`; viewport scrolling via keyboard (PgUp/PgDn, arrows, Ctrl+U/D) is unaffected
- Root cause: xterm's `?1000h` mode (the minimum needed for scroll-wheel reporting) also captures click events, blocking native text selection

#### Stability — eliminate silent crashes and deadlocks
- Changed `panic = "abort"` → `panic = "unwind"` in release profile so panics produce stack traces instead of silent process death
- Added panic hook to `cade-server` to log panics via `tracing::error` before unwinding
- Replaced `join_all(...).flatten()` in parallel tool runner with explicit `JoinError` handling so panicked tasks are logged instead of silently dropped
- Eliminated `spawn_blocking` + `parking_lot::Mutex` deadlock in the password handler by adding `ask_password_async()` to `TuiApp` (push overlay, return `oneshot::Receiver` — no lock held during await)
- Un-deprecated `ask_question_async` — its deprecation note incorrectly recommended `spawn_blocking` which is itself the deadlock source
- Hardened MCP `reload()` `.expect()` → `let-else` with `tracing::error` so a logic bug logs and continues instead of panicking

#### Startup hangs, timeouts, and first-try reliability (4-phase remediation)
- **Phase 1** — `auto_start_server()` now uses exponential backoff (100ms → 1.6s capped) with a 15s total timeout (30s on first run when `~/.cade/cade.db` is missing); retains the `Child` handle and checks `try_wait()` each iteration so a crashed server fails immediately instead of waiting the full timeout
- **Phase 2** — `McpManager::start()` and `::reload()` wrap each `connect_server` call in `tokio::time::timeout(15s)`; timed-out servers are logged and skipped; `start()` returns a `Vec<McpStartResult>` (`Ok`/`Failed`/`Timeout`) so callers can report per-server status
- **Phase 3** — `StartupProgress` now shows elapsed time in spinners (`{elapsed_precise}`); added `start_mcp_server(name)` for per-server progress lines and `finish_skip()` for timed-out servers; failed/timed-out servers print to stderr with reasons
- **Phase 4** — Synchronous MCP boot with per-server spinners (deferred-background variant explored and reverted — synchronous boot with timeouts gives better UX than instant-but-blank TUI)
- Added `McpManager::merge_from()` and `McpStartResult` enum for future deferred-startup work

### Added

#### Model catalogue — modern Anthropic and OpenAI token limits
- Added `Claude Opus 4.7` to the static catalogue with 128k output / 1M context
- Updated `Claude Opus 4.5`, `Sonnet 4.6/4.5/3.7` to 128k output and 1M context (was 8k / 200k)
- Updated `Claude Haiku 4.5` to 128k output
- Updated OpenAI reasoning models (`o3`, `o3-mini`, `o4-mini`) to 100k output for chain-of-thought headroom (was 16k)
- Fixed GPT-4.1 context window typo `1_047_576` → `1_048_576`
- `max_tokens_for_model` fallback: uncatalogued `anthropic/claude-*` now returns 128k; uncatalogued `openai/o*` returns 100k
- `context_window_for_model` fallback: uncatalogued `anthropic/` models containing `opus` or `sonnet` now return 1M instead of 200k
- Resolves `claude-opus-4-7` being interrupted mid-response — the uncatalogued model was hitting the legacy 4096 output token default, which starved adaptive thinking of headroom

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
