## 2026-04-16T01:15:00Z — feat: install_skill supports bare repo URLs and skill selection

**Summary:** Enhanced `install_skill` tool to support the `npx skills add` ecosystem pattern. Users can now install skills from bare GitHub repo URLs (e.g., `https://github.com/github/awesome-copilot`) and `owner/repo` shorthand by providing a `skill` parameter to select which skill to install from a multi-skill repository.

**Files modified:**
- `crates/cade-core/src/skills/watcher.rs` — Added `resolve_github_repo_skill_url()` function; updated `install_skill_from_url()` signature to accept `skill_name: Option<&str>`; added resolution chain: repo+skill → tree/blob → direct URL
- `crates/cade-core/src/skills/tests.rs` — Added 8 new tests for `resolve_github_repo_skill_url` (bare URL, shorthand, trailing slash, missing skill, non-GitHub, invalid owner/repo, path traversal)
- `crates/cade-agent/src/tools/meta.rs` — Added `skill` parameter to `install_skill` tool schema
- `crates/cade-agent/src/tools/runtime/skills.rs` — Extract and pass `skill` parameter to `install_skill_from_url()`

**Previous behavior:** `install_skill` only accepted GitHub tree/blob URLs or direct SKILL.MD URLs. Bare repo URLs like `https://github.com/github/awesome-copilot` would fail.
**New behavior:** `install_skill(url="https://github.com/github/awesome-copilot", skill="rust-mcp-server-generator")` resolves to the raw SKILL.md URL and installs it. Also supports `owner/repo` shorthand.
**Rollback:** Revert commit or restore checkpoint `before-install-skill-enhancement`.

## 2026-04-12T21:09:00Z — TUI: Nerd Font icons for tool calls and results

**Summary:** Added Nerd Font glyph icons for all tool call types (bash, file read/write, git, GitHub, memory, skills, subagents, web, etc.) and tool result status badges (success/error). Icons render automatically when `use_nerd_fonts` is true (default). Falls back to plain ASCII/Unicode (`▶`, `✓`, `✗`) when disabled.
**Files modified:**
- `crates/cade-tui/src/icons.rs` — NEW: const icon map with `tool_icon()`, `success_icon()`, `error_icon()` functions + 5 unit tests
- `crates/cade-tui/src/lib.rs` — registered `icons` module
- `crates/cade-tui/src/app/mod.rs` — added `use_nerd_fonts: bool` field to `TuiApp`, threaded `nerd` through `render_frame` call and test callsites
- `crates/cade-tui/src/app/render.rs` — added `nerd: bool` param to `render_frame`, passed through to timeline rendering
- `crates/cade-tui/src/app/state.rs` — passed `use_nerd_fonts` to `visual_rows_with_state`
- `crates/cade-tui/src/app/timeline/render_item.rs` — `render_tool_call_item` uses `tool_icon()` instead of hardcoded `"▶ TOOL "`; `render_tool_result_item` uses `success_icon()`/`error_icon()`
- `crates/cade-tui/src/app/timeline/mod.rs` — threaded `nerd: bool` through `render_into`, `visual_rows`, `render_with_state`, `visual_rows_with_state`, `prepare_timeline_entries`
**Reason:** Nerd Font icons provide instant visual differentiation of tool call types without reading the tool name.
**Previous behavior:** All tool calls showed `▶ TOOL <name>(...)`. Results showed `✓ OK` / `✗ ERR`.
**New behavior:** Tool calls show a type-specific Nerd Font icon (e.g. `` for bash, `` for file read, `` for git). Results show `` / `` in nerd mode. ASCII fallback preserved when `use_nerd_fonts = false`.
**Tests:** 26/26 cade-tui tests pass (5 new icon tests). Binary size unchanged (15M release).
**Rollback steps:** `git revert HEAD`

## 2026-04-12T20:51:00Z — TUI: Rounded borders on all bordered panels

**Summary:** Applied `BorderType::Rounded` to all 9 `Borders::ALL` callsites across the TUI. Sidebar panels (`Borders::LEFT` only) intentionally left unchanged — rounding a single edge produces broken glyphs.
**Files modified:**
- `crates/cade-tui/src/overlay.rs` — overlay shell border
- `crates/cade-tui/src/app/mod.rs` — added `BorderType` to ratatui widget import
- `crates/cade-tui/src/app/render.rs` — Todos/plan panel border + added `BorderType` import
- `crates/cade-tui/src/app/layout/toast.rs` — toast notification border
- `crates/cade-tui/src/app/layout/pickers.rs` — theme picker table + filter borders
- `crates/cade-tui/src/skills.rs` — skills table + preview borders
- `crates/cade-tui/src/mcp_picker.rs` — MCP servers table + config preview borders
**Reason:** Rounded borders (╭╮╰╯) are the modern TUI standard; sharp borders (┌┐└┘) look dated.
**Previous behavior:** All bordered blocks used default sharp corners (`BorderType::Plain`).
**New behavior:** All `Borders::ALL` blocks use `BorderType::Rounded`. `Borders::LEFT`-only sidebar blocks unchanged.
**Tests:** 14/14 cade-tui tests pass. Binary size unchanged (15M release).
**Rollback steps:** `git revert 596a208`

## 2026-04-12T20:51:00Z — TUI: PageUp/PageDown viewport-aware scroll

**Summary:** Added `PageUp`/`PageDown` key handlers to the main conversation timeline. Scroll step equals the actual viewport content height (terminal height minus fixed UI rows), matching user expectation for page-based navigation. Extracted `scroll_page_up()` and `scroll_page_down()` pure functions with 7 unit tests covering all edge cases.
**Files modified:**
- `crates/cade-tui/src/app/input.rs` — Added `PageUp`/`PageDown` match arms in `handle_key_input`; added `scroll_page_up()`/`scroll_page_down()` helper functions; added 7 new unit tests; imported `FIXED_ROWS`/`MAX_INPUT_ROWS` constants.
**Reason:** Existing scroll keys (`K`=+10 lines, `J`=snap to bottom) are coarse. PageUp/PageDown provide standard, viewport-proportional scrolling with no keystroke collision risk.
**Previous behavior:** Only `Shift+K` (+10 lines), `Shift+J` (snap to bottom), and mouse wheel (±1 line) for timeline scrolling.
**New behavior:** `PageUp` scrolls up by one viewport height. `PageDown` scrolls down by one viewport height; reaching scroll=0 re-enables auto-follow. Viewport height = terminal rows − FIXED_ROWS − MAX_INPUT_ROWS.
**Tests:** 7 new tests (page_up from_bottom, already_scrolled, zero_viewport; page_down to_bottom, partial, already_at_bottom, zero_viewport). 21/21 cade-tui tests pass. Binary size unchanged (15M).
**Rollback steps:** `git revert HEAD`

## 2026-04-13T12:00:00Z — CADE-nvim Option B: Inline Completions Implementation
**Summary:** Implemented direct-HTTP inline code completions for the CADE-nvim Neovim plugin. Lua modules call the existing `POST /v1/agents/:id/complete` SSE endpoint — same backend as the VS Code extension — eliminating the MCP round-trip proposed in the original Option A plan.
**Files modified:**
- `~/.local/share/nvim/lazy/CADE-nvim/lua/cade/config.lua` — NEW: defaults + user config merge (port, agent_id, debounce, hl_group, etc.)
- `~/.local/share/nvim/lazy/CADE-nvim/lua/cade/ghost.lua` — NEW: extmark ghost-text renderer (virt_text inline for line 1, virt_lines below for remaining)
- `~/.local/share/nvim/lazy/CADE-nvim/lua/cade/http.lua` — NEW: async curl SSE client via vim.system with cancel() support
- `~/.local/share/nvim/lazy/CADE-nvim/lua/cade/trigger.lua` — NEW: debounced TextChangedI/CursorMovedI handler with in-flight cancellation
- `~/.local/share/nvim/lazy/CADE-nvim/lua/cade/init.lua` — NEW: public API (setup, accept, accept_line, accept_word, dismiss, toggle)
- `~/.local/share/nvim/lazy/CADE-nvim/plugin/cade.lua` — Extended: append autocmds + keymaps for completions
- `~/.config/nvim/lua/plugins/cade.lua` — NEW: lazy.nvim plugin spec pointing to local CADE-nvim directory
- `CADE-nvim-completions-plan-B.md` — NEW: Option B implementation plan document
**Reason:** The original Option A plan proposed adding completion tools to the MCP server.py and having CADE orchestrate completions via MCP. With the `/v1/complete` endpoint and VS Code extension already built, Option B avoids the MCP round-trip by having Neovim Lua call the HTTP endpoint directly — consistent with the VS Code architecture and lower latency.
**Previous behavior:** CADE-nvim had only socket setup + 3 MCP intercept tools (ide_read_buffer, ide_propose_edit, ide_apply_patch). No code completion support. Plugin was not loaded by lazy.nvim.
**New behavior:** Ghost-text completions appear after 300ms debounce, streamed incrementally via SSE. Accept with Tab (full), C-] (line), M-] (word), or dismiss with C-e. Toggle on/off with leader-ct. All keymaps use expr=true to pass through when no completion is visible.
**Tests:** All 5 Lua modules load cleanly. 3 autocmds registered (TextChangedI, CursorMovedI, InsertLeave). 4 insert-mode keymaps + 1 normal-mode keymap verified. Ghost state functions return correct defaults. Toggle flips enabled state. Full Neovim startup produces no errors.
**Rollback steps:** `cd ~/.local/share/nvim/lazy/CADE-nvim && git reset --hard HEAD~1` and `rm ~/.config/nvim/lua/plugins/cade.lua`

## 2026-04-12T04:15:00Z — Context Efficiency: Polishing P5-B and P4-C
**Summary:** Added proactive consolidation trigger for length (P5-B) and blocking endpoint test coverage (P4-C).
**Files modified:**
- `crates/cade-server/src/server/api/messages/context.rs` — Set `needs_consolidation` if post-marker turns exceed 20, improving summarization sensitivity.
- `crates/cade-server/src/server/api/messages/tests.rs` — Added test to ensure blocking endpoint respects proactive consolidation limits.
**Reason:** Prevent context token bloat in long conversations that have not yet reached the 80% token utilization threshold, and solidify testing coverage.
**Tests:** Existing 129 tests passed cleanly.
**Rollback steps:** `git reset --hard HEAD~1`

## 2026-04-12T03:30:00Z — Context Efficiency: P4-B to P6-B (Completion)
**Summary:** Finalized the remaining context efficiency phases. Reflection (`/reflect`) now respects compaction boundaries (P5-A); `session_summary` is forced to remain pinned across restarts (P5-C); `conversation_search` identifies pre-compaction snippets (P4-B); metrics for efficiency tracking were exposed via `/v1/agents/:id/metrics` (P6-A); and `compaction_model` configuration was exposed via the CLI (`/compaction-model`) and API (P6-B).
**Files modified:**
- `crates/cade-server/src/server/reflection.rs` — Uses `get_context_window` to avoid redundant reflection on compressed history.
- `crates/cade-server/src/server/consolidation.rs` — Sets `session_summary` tier to `pinned`.
- `crates/cade-store/src/sqlite/tools.rs` — Appends note to FTS snippets before compaction markers.
- `crates/cade-server/src/server/state.rs` & `crates/cade-server/src/server/api/agents.rs` — Added `AgentMetrics` and exposed endpoint.
- `crates/cade-tui/src/menu.rs` & `crates/cade-cli/src/cli/repl/slash.rs` — CLI `/compaction-model` command.
**Reason:** Addressed operational gaps identified post-P4-A (stale history scanning, lost session continuity, missing observability, and missing UX for configuration).
**Tests:** Existing 129 tests passed cleanly.
**Rollback steps:** `git revert c81c742`

## 2026-04-12T02:45:00Z — Context Efficiency: P4-A Compaction Markers
**Summary:** Implemented compaction markers — DB-level sentinel messages (`role = 'compaction'`) that `get_context_window()` uses as a boundary to skip pre-summarized history. Addresses all 6 identified risks: LLM provider rejection (filtered in `db_row_to_llm`), FTS pollution (filtered in `search_messages`), consumer breakage (filtered in `list_messages_page`), recursive summarization (excluded via list filter), timestamp ordering (marker uses boundary message's timestamp), and backward compatibility (COALESCE falls back to 0 when no markers exist).
**Files modified:**
- `crates/cade-server/src/server/api/messages/persist.rs` — `db_row_to_llm()` returns empty vec for `role = "compaction"`
- `crates/cade-server/src/server/consolidation.rs` — Inserts compaction marker after writing session_summary, anchored to boundary message timestamp
- `crates/cade-store/src/sqlite/messages.rs` — `get_context_window()` SQL uses CTE boundary to scan only messages after latest marker; `list_messages_page()` excludes compaction markers; 4 new tests
- `crates/cade-store/src/sqlite/tools.rs` — `search_messages()` excludes compaction markers from FTS results
**Reason:** `get_context_window()` previously scanned ALL messages in the conversation (up to 500) on every request. With compaction markers, it only scans messages AFTER the most recent marker — drastically reducing the scan set for long sessions.
**Previous behavior:** Every `build_context()` call loaded and budgeted all messages from conversation start. Long sessions with 200+ messages had high DB query overhead.
**New behavior:** After Sleeptime consolidation runs, a `role = 'compaction'` sentinel is inserted at the boundary. Subsequent `get_context_window()` queries only scan messages inserted after that sentinel. Pre-marker messages remain in the DB for `conversation_search` recovery.
**Tests:** 4 new compaction marker tests (list exclusion, boundary stop, backward compat, multiple markers). 73 cade-store tests, 32 cade-server tests, 15 regression tests — all pass. Full cargo check clean.
**Rollback steps:** Revert to checkpoint `cp-1f990c6b` or remove compaction marker code from the 4 files.

## 2026-04-12T01:30:00Z — Context Efficiency: Full Phase 1-3 Implementation
**Summary:** Implemented all six planned context efficiency improvements (P1-A through P3-A). Changes derived from industry research comparing OpenCode, Gemini CLI, Aider, and MemGPT approaches.
**Files modified:**
- `crates/cade-server/src/server/consolidation.rs` — Structured 7-section compaction template (P1-A), inflation guard (P1-B), weak-model resolution for consolidation (P1-C)
- `crates/cade-server/src/server/api/messages/context.rs` — Proactive overflow signal at 80% usage (P2-B), surgical tool-output pruning integration (P2-A)
- `crates/cade-server/src/server/api/messages/mod.rs` — Per-tool output limits static map (P3-A)
- `crates/cade-store/src/sqlite/mod.rs` — DB migration v2: `compaction_model` column on `agents` table (P1-C)
- `crates/cade-store/src/sqlite/agents.rs` — `AgentRow.compaction_model` field, `update_agent_compaction_model()`, updated SELECTs
- `crates/cade-store/src/sqlite/messages.rs` — `compact_old_tool_outputs()` DB function (P2-A)
- `crates/cade-store/src/sqlite/{conversations,evidence,memory/tests,runs,tools}.rs` — `compaction_model: None` in all `AgentRow` test constructors
**Reason:** Industry research showed CADE's within-session token efficiency had gaps vs. competing agents. Six changes address: compaction quality (structured template), safety (inflation guard), cost (weak model), proactiveness (80% threshold), context reclamation (surgical pruning), and proportional limits (per-tool caps).
**Previous behavior:** Free-form consolidation prompt; no inflation guard; consolidation on main model only; reactive-only overflow detection; no surgical tool-output pruning; single global 8192-char tool result cap.
**New behavior:** Structured 7-section template; summaries ≥80% of source size rejected; configurable `compaction_model` per agent (falls back to main model); proactive consolidation at 80% context usage; old tool outputs beyond 120k-char protect window replaced with placeholder; per-tool output limit map (bash 4k, read_file 12k, grep 3k, memory 2k, default 8k).
**Tests:** 5 new inflation-guard unit tests, 2 compaction_model CRUD tests, 3 compact_old_tool_outputs tests. 69 cade-store tests pass, 32 cade-server tests pass, 15 regression tests pass. Full workspace cargo check clean.
**Rollback steps:** Revert via `git stash pop stash@{0}` from checkpoint `cp-d7ae709e` or revert the individual files.

## 2026-04-10T16:45:00Z — OpenRouter Architecture & Reasoning Stream Stability
**Summary:** Resolved severe stability, parsing, and context retention bugs when interfacing with OpenRouter and reasoning-capable models (e.g., qwen3.6-plus).
**Files modified:** `crates/cade-ai/src/openai.rs`, `crates/cade-cli/src/cli/repl/turn_loop/stream.rs`, `crates/cade-cli/src/cli/repl/turn_tools/runner.rs`, `crates/cade-server/src/server/api/messages/mod.rs`
**Reason:** The system panicked on SSE streams, stripped required model org prefixes resulting in 400 errors, failed to request reasoning tokens natively, discarded reasoning content from SQLite persistence, failed to flush reasoning to the TUI if the assistant returned no other content, and infinite-looped when encountering 429 rate limit errors.
**Previous behavior:** Crashed with slice indexing bounds panic; OpenRouter models failed to load; 429 errors created an infinite loop; reasoning streams were lost between turns.
**New behavior:** Safely parses SSE streams; injects `include_reasoning`, `HTTP-Referer`, and `X-Title` headers; preserves `google/` prefixes; flushes and persists reasoning streams in `<reasoning>` XML tags; exits gracefully on empty API responses.
**Rollback steps:** `git revert 0f3e290`

## 2026-04-12T18:21:00Z — cade.nvim: agent_id settings.json fallback
**Summary:** `config.lua` now falls back to `~/.cade/settings.json → last_agent` when `$CADE_AGENT_ID` is unset, making the plugin zero-config for users who already run the CADE TUI.
**Files modified:**
- `plugins/cade.nvim/lua/cade/config.lua` — Added `resolve_agent_id()` function: checks env var first, then reads and decodes `~/.cade/settings.json`, falls back to `""`. `setup()` accepts internal `_settings_path` key for test injection.
- `plugins/cade.nvim/spec/minimal_init.lua` — New. Minimal test init that adds lua/ to rtp and prevents plugin/cade.lua serverstart conflict.
- `plugins/cade.nvim/spec/config_spec.lua` — New. 3 plenary tests: file fallback, env-var priority, missing file graceful fallback.
**Previous behavior:** `agent_id` defaulted to `$CADE_AGENT_ID` only; plugin was silent/inert when the env var was unset.
**New behavior:** `agent_id` resolves via `$CADE_AGENT_ID → settings.json.last_agent → ""`.
**Tests:** 3/3 pass (plenary busted).
**Rollback steps:** Restore `config.lua` from commit `470989d`.

## 2026-04-12T18:35:00Z — cade.nvim: :CadeStatus command
**Summary:** Added `require("cade").status()` function and `:CadeStatus` user command. Displays completion status, agent ID, server reachability (via sync curl probe), API key presence, debounce, and current filetype.
**Files modified:**
- `plugins/cade.nvim/lua/cade/init.lua` — Added `_probe_server()` (uses `vim.system` sync curl) and `status()` (builds info string, calls `vim.notify()`). `_probe_server` is overridable for test injection.
- `plugins/cade.nvim/plugin/cade.lua` — Registered `CadeStatus` user command.
- `plugins/cade.nvim/spec/status_spec.lua` — New. 3 plenary tests: field presence, reachable icon, unreachable icon.
**Previous behavior:** No way to check plugin state or server reachability.
**New behavior:** `:CadeStatus` displays a formatted status block in `vim.notify()`.
**Tests:** 6/6 pass (3 config + 3 status).
**Rollback steps:** Revert `init.lua` and `plugin/cade.lua` from commit `470989d`.

## 2026-04-12T19:10:00Z — cade.nvim: ghost.lua test coverage
**Summary:** Added 9 plenary tests covering all public functions in ghost.lua. No implementation changes — tests confirm existing behaviour is correct.
**Files modified:**
- `plugins/cade.nvim/spec/ghost_spec.lua` — New. 9 tests: show() state tracking, show(nil/empty) no-op guards, clear() full reset, accept() no-pending guard, accept() full buffer insertion, accept_line() multi-line partial, accept_line() single-line clear, accept_word() leading-space inclusion.
**Previous behavior:** ghost.lua had zero test coverage.
**New behavior:** All 9 ghost behaviours verified. 9/9 pass.
**Rollback steps:** Delete `spec/ghost_spec.lua`.

## 2026-04-12T19:25:00Z — cade.nvim: http.lua test coverage + _parse_sse_line extraction
**Summary:** Extracted SSE parsing logic from the inline stdout callback into a public `_parse_sse_line()` pure function. Added 7 plenary tests covering all parse cases and fetch() guards.
**Files modified:**
- `plugins/cade.nvim/lua/cade/http.lua` — Added `M._parse_sse_line(line)` pure function (stream_delta, [DONE], error, nil-for-noise). Rewired stdout callback to call it. Zero behaviour change.
- `plugins/cade.nvim/spec/http_spec.lua` — New. 7 tests: 5 _parse_sse_line cases + fetch() empty-agent guard + fetch() cancel contract.
**Previous behavior:** SSE parsing was inline and untestable. http.lua had zero test coverage.
**New behavior:** All SSE parse logic testable in isolation. 7/7 pass. Full suite 22/22.
**Rollback steps:** Revert `http.lua` from commit `2482c51`. Delete `spec/http_spec.lua`.

## 2026-04-12T19:45:00Z — cade.nvim: completion latency telemetry
**Summary:** http.lua now records os.clock() timestamps for each fetch() call. status() / :CadeStatus displays a Latency line showing ttft (time-to-first-token) and total duration after at least one completion has fired.
**Files modified:**
- `plugins/cade.nvim/lua/cade/http.lua` — Added `M._last_request_at`, `M._last_first_token`, `M._last_done_at` module-level fields. Set in fetch(): request_at on entry, first_token on first delta, done_at on stream end or error.
- `plugins/cade.nvim/lua/cade/init.lua` — status() reads http telemetry fields and appends "Latency: ttft=Xms total=Xms" or "(no data)".
- `plugins/cade.nvim/spec/http_spec.lua` — +1 test: _last_request_at is a number after fetch() fires.
- `plugins/cade.nvim/spec/status_spec.lua` — +2 tests: Latency "(no data)" when no fetch, ttft=/total= when telemetry present.
**Previous behavior:** No timing data available. :CadeStatus showed no latency.
**New behavior:** After each completion, ttft and total latency visible in :CadeStatus. Full suite: 25/25.
**Rollback steps:** Revert `http.lua` and `init.lua`. Remove telemetry tests from specs.

## 2026-04-12T20:05:00Z — cade.nvim: customizable keymaps
**Summary:** Keymaps are now driven by config. Users can override individual keys or set keymaps=false to disable all bindings. plugin/cade.lua replaced hardcoded imap calls with a config-driven loop.
**Files modified:**
- `plugins/cade.nvim/lua/cade/config.lua` — Added `keymaps` table to M.defaults with 5 keys: accept, accept_line, accept_word, dismiss, toggle. Defaults match previous hardcoded values.
- `plugins/cade.nvim/plugin/cade.lua` — Replaced 5 hardcoded keymap calls with a loop over cfg.keymaps. Guards: `if cfg.keymaps ~= false` for the block, `if lhs` per binding (nil keys are skipped).
- `plugins/cade.nvim/spec/config_spec.lua` — +3 tests: default keys present, partial merge, keymaps=false.
**Previous behavior:** Keymaps were hardcoded. No way to remap or disable without editing the plugin file.
**New behavior:** Pass keymaps={accept="<C-y>"} to override one key; keymaps=false to disable all. Full suite: 28/28.
**Rollback steps:** Revert `config.lua` and `plugin/cade.lua`. Remove keymap tests from config_spec.

---

## 2026-04-12 — TUI: Refactor sidebar into SidebarState

**Summary:** Eliminated the 21-argument `render_sidebar` free-function signature by introducing a `SidebarState<'a>` struct. Extracted three formatting helpers (`format_activity`, `format_context`, `format_plan_summary`) as `pub(crate)` methods on the struct, making them independently unit-testable without a Ratatui frame. Added 7 unit tests covering all formatting branches. Removed the `#[allow(clippy::too_many_arguments)]` suppressor from `render_sidebar`.

**Files modified:**
- `crates/cade-tui/src/app/layout/sidebar.rs` — Added `SidebarState<'a>` struct; rewrote `render_sidebar` signature to `(frame, area, &SidebarState, colors)`; added `#[cfg(test)]` module with 7 tests.
- `crates/cade-tui/src/app/render.rs` — Updated import to include `SidebarState`; replaced 19-argument `render_sidebar(...)` call with `SidebarState { .. }` construction + 4-argument call.

**Reason:** Argument bloat, mixed concerns (formatting logic coupled to frame rendering), and zero unit-test coverage on sidebar formatting logic.

**Previous behaviour → New behaviour:** Identical visual output. `render_sidebar` now delegates formatting to `SidebarState` methods rather than computing strings inline.

**Rollback:** `git revert HEAD` or restore checkpoint `cp-abe2880d` (label: `before-sidebar-refactor`).
- **Timestamp (UTC)**: 2026-04-13T15:34:30Z
- **Summary of change**: Fixed Gemini API payload errors when caching tool schemas.
- **Files modified**: `crates/cade-ai/src/utils.rs`, `crates/cade-ai/src/gemini.rs`, `crates/cade-ai/src/tests.rs`
- **Exact reason**: The Gemini backend rejects JSON schemas with lowercase `type` strings when generating cached content (though it accepts them directly on standard completions). The schemas are now converted to uppercase (e.g. `STRING`, `OBJECT`) to fix `Proto field is not repeating` errors.
- **Previous behavior**: `clean_gemini_schema` mapped schema types to lowercase strings.
- **New behavior**: `clean_gemini_schema` casts schema types to uppercase strings.
- **Rollback instructions**: Revert `crates/cade-ai/src/utils.rs` and `crates/cade-ai/src/tests.rs` using git checkout.
- **Timestamp (UTC)**: 2026-04-13T16:45:54Z
- **Summary of change**: Drafted a comprehensive TUI refactor plan inspired by pi-coding-agent.
- **Files modified**: `docs/tui-refactor-plan.md` (created)
- **Reason**: The user requested a review of pi-coding-agent's TUI and a refactor plan for CADE based on those takeaways.
- **Previous behavior**: N/A (new document)
- **New behavior**: The repository now contains a formal blueprint for modernizing the TUI architecture (IME support, overlay stack, pluggable editor, UI slots).
- **Rollback steps**: Remove `docs/tui-refactor-plan.md`.
- **Timestamp (UTC)**: 2026-04-13T17:02:35Z
- **Summary of change**: Drafted a concise implementation plan for the TUI refactor.
- **Files modified**: `docs/tui-refactor-implementation.md` (created)
- **Reason**: The user requested a concise implementation plan for the TUI refactor.
- **Previous behavior**: N/A
- **New behavior**: The repository contains a 4-phase implementation plan.
- **Rollback steps**: Remove `docs/tui-refactor-implementation.md`.
- **Timestamp (UTC)**: 2026-04-13T17:50:23Z
- **Summary of change**: Implemented Phase 1 of the TUI refactor (hardware cursor sync).
- **Files modified**: `crates/cade-tui/src/app/mod.rs`, `crates/cade-tui/src/app/render.rs`
- **Reason**: The user asked me to implement Phase 1 from the refactor plan.
- **Previous behavior**: The cursor was drawn purely visually by the TUI widget, meaning standard IMEs didn't know where to open candidate windows.
- **New behavior**: CADE now queries the exact visual coordinate of the input prompt during the render step, and emits a `crossterm::cursor::MoveTo(x,y)` command immediately after terminal flush.
- **Rollback steps**: Revert changes to `crates/cade-tui/src/app/mod.rs` and `crates/cade-tui/src/app/render.rs` using git checkout.
- **Timestamp (UTC)**: 2026-04-13T18:36:57Z
- **Summary of change**: Reviewed CADE's UI styling and formatting logic compared to pi-coding-agent.
- **Files modified**: None
- **Reason**: The user asked for a comparison of UI styling and formatting logic between CADE and pi-coding-agent, and to identify parts that can be adopted in CADE.
- **Previous behavior**: N/A
- **New behavior**: N/A
- **Rollback steps**: N/A
- **Timestamp (UTC)**: 2026-04-13T18:42:13Z
- **Summary of change**: Drafted a concise implementation plan for Phase 2 of the TUI refactor.
- **Files modified**: `docs/tui-style-builder-plan.md` (created)
- **Reason**: The user requested an implementation plan to adopt pi's clean theme builder patterns inside CADE.
- **Previous behavior**: N/A
- **New behavior**: The repository contains a migration blueprint for CADE's Style Builders.
- **Rollback steps**: Remove `docs/tui-style-builder-plan.md`.
- **Timestamp (UTC)**: 2026-04-13T18:46:23Z
- **Summary of change**: Migrated CADE TUI to use clean style builder methods (Phase 2).
- **Files modified**: `crates/cade-tui/src/colors.rs`, `crates/cade-tui/src/app/render.rs`, and 14 other layout/component files.
- **Exact reason**: The user requested that CADE adopt the clean theme builder patterns observed in `pi-coding-agent`'s UI architecture.
- **Previous behavior**: The UI rendering logic relied on over 150 verbose instances of `Style::default().fg(colors.token)`.
- **New behavior**: Extended `ThemeColors` with style builders for every semantic color token. Refactored the UI components to use the concise `colors.token()` builder format instead.
- **Rollback instructions**: Use `git checkout` to revert the changes to `crates/cade-tui/src`.
- **Timestamp (UTC)**: 2026-04-13T22:26:06Z
- **Summary of change**: Halted TUI refactoring after Phase 2 to prevent architectural bloat.
- **Files modified**: None (Decision logged)
- **Exact reason**: Evaluated Phase 3 (Pluggable Editor) and Phase 4 (Overlay Stack) and determined they introduce unnecessary complexity (dynamic dispatch, borrow checker event routing, two-pass layout engines) into CADE's performant monolithic Rust loop.
- **Previous behavior**: N/A
- **New behavior**: N/A
- **Rollback instructions**: N/A
- **Timestamp (UTC)**: 2026-04-13T22:38:55Z
- **Summary of change**: Drafted a new implementation plan for a `/summarize` TUI modal.
- **Files modified**: `docs/tui-summarize-plan.md` (created)
- **Reason**: The user requested a plan to implement a pi-coding-agent style summary modal using CADE's existing background consolidation system.
- **Previous behavior**: N/A
- **New behavior**: The repository contains a blueprint for an instantaneous, cost-free conversation summary overlay.
- **Rollback steps**: Remove `docs/tui-summarize-plan.md`.
- **Timestamp (UTC)**: 2026-04-13T23:16:53Z
- **Summary of change**: Implemented an instant, zero-cost `/summarize` TUI modal.
- **Files modified**: `crates/cade-tui/src/app/layout/summary.rs`, `crates/cade-tui/src/app/render.rs`, `crates/cade-tui/src/app/input.rs`, `crates/cade-tui/src/app/mod.rs`, `crates/cade-cli/src/cli/repl/slash.rs`, `crates/cade-cli/src/cli/repl/commands.rs`
- **Exact reason**: The user requested a summarize mechanism similar to pi-coding-agent but built natively using CADE's existing background consolidation system.
- **Previous behavior**: Users had no interactive way to view the background-computed session summary.
- **New behavior**: Typing `/summarize` instantly pulls the `session_summary` memory block from the local SQLite database and displays it in a floating scrollable modal. If the conversation is too short for a summary, a toast notification is shown instead.
- **Rollback instructions**: Revert the commit `feat(tui): implement instant zero-cost /summarize modal` using git.
- **Timestamp (UTC)**: 2026-04-14T00:31:43Z
- **Summary of change**: Drafted an implementation plan to repurpose the `/copy` command as a programmatic clipboard extractor, renaming the original mouse capture toggle to `/mouse`.
- **Files modified**: `docs/copy-command-plan.md` (created)
- **Reason**: The user asked for a plan to make CADE's `/copy` command behave like pi-coding-agent's, which grabs the last message and copies it to the clipboard using OSC 52 and native OS APIs.
- **Previous behavior**: N/A
- **New behavior**: The repository now contains a blueprint for the `/copy` command refactor.
- **Rollback steps**: Remove `docs/copy-command-plan.md`.
## 2026-04-16T01:41:00Z — fix: dual-store file corruption causing agent not auto-loaded

**Summary:** Fixed a critical bug where `SessionStore` and `SettingsManager` both read/wrote `.cade/settings.local.json` with disjoint schemas. Each `save()` overwrote the other's fields, causing agent identity loss across restarts and mid-session agent switches.

**Root cause:** Two independent structs (`Session` with `agent_id`, `conversation_id` etc. and `LocalSettings` with `last_agent`, `pinned_agents` etc.) shared the same JSON file. Last writer won, destroying the other's data.

**Files modified:**
- `crates/cade-agent/src/agent/session.rs` — Moved `SessionStore` from `settings.local.json` to `session.json`; added backward-compat migration from legacy file; added `ensure_gitignore_entry()` helper; 5 new tests
- `crates/cade-mcp/src/watcher.rs` — Added `session.json` to watched filenames
- `crates/cade-core/src/permissions/manager.rs` — Added `session.json` to security guard for config file edits
- `crates/cade-cli/src/cli/repl/commands_agents.rs` — `/agents` Switch and DeleteMany branches now call `session.set_agent()` alongside `settings.set_last_agent()`
- `src/bootstrap/agents.rs` — `--agent` and `--name` branches now persist to both stores; happy-path lookups cross-sync between stores
- `README.md`, `SECURITY.md`, `WINDOWS_SETUP.md` — Updated file layout references

**Previous behavior:** Agent identity was randomly lost depending on which store saved last. `/agents` switch didn't persist to session. `--agent`/`--name` flags were forgotten on restart. Cross-project agent switching could load wrong agent.
**New behavior:** Each store has its own file. All agent resolution branches persist to both stores. Happy-path lookups cross-sync so both stores stay consistent.
**Rollback:** Restore checkpoint `before-dual-store-fix` (cp-ad662ffb).


## 2026-04-16T02:05:00Z — docs: update CHANGELOG.md

**Summary:** Updated `CHANGELOG.md` to reflect the session persistence fixes, the UI interrupt message refactoring, and the security dependency updates.


## 2026-04-16T02:30:00Z — test: add dual-store coexistence integration test

**Summary:** Added integration test proving `SessionStore` (session.json) and `SettingsManager` (settings.local.json) coexist without data loss. Test exercises interleaved writes and reloads from both stores, verifying no cross-contamination or clobbering.

**Files modified:**
- `crates/cade-agent/src/agent/session.rs` — added `dual_store_coexistence_no_data_loss` test

**Reason:** Phase 4 of dual-store file corruption fix. Validates that the file separation introduced in Phase 1 truly prevents the original bug.
**Previous behavior:** No integration test existed for dual-store safety.
**New behavior:** 31 tests total (25 original + 6 session tests), all passing.
**Rollback:** Remove the test function from session.rs.


## 2026-04-16T03:15:00Z — feat(tui): UI/UX polish batch (4 items)

**Summary:** Four low-effort, high-impact UI/UX improvements:

1. **Toast auto-dismiss** — Toasts now expire after their TTL (3s default). Added `Toast::is_expired()`, hooked into `draw()`, the REPL idle input loop, and the turn-loop tick task.
2. **Footer token counter** — Cumulative session token count shown in the footer bar in compact form (e.g. "1.2k↑", "50k↑"). Added `session_tokens` field to TuiApp, `format_token_count()` helper, and REPL sync.
3. **Startup context summary** — On resume, fetches the `working_set` memory block and displays the first 3 lines as a "Context:" line in the startup banner.
4. **Command menu section headers** — `/help` menu headers now include trailing rule lines. Inline command palette shows `[Section]` tags when filtering.

**Files modified:**
- `crates/cade-tui/src/app/mod.rs` — `Toast::is_expired()`, auto-dismiss in `draw()`, `session_tokens` field, test
- `crates/cade-tui/src/app/input.rs` — toast-aware redraw in idle input loop
- `crates/cade-tui/src/app/render.rs` — `session_tokens` param, footer token rendering
- `crates/cade-tui/src/app/layout/helpers.rs` — `format_token_count()` + test
- `crates/cade-tui/src/app/layout/command_palette.rs` — section tag rendering
- `crates/cade-tui/src/menu.rs` — section header rule lines
- `crates/cade-cli/src/cli/repl/mod.rs` — token sync to TuiApp, startup context fetch
- `crates/cade-cli/src/cli/repl/turn_loop/agent.rs` — toast in tick redraw condition

**Previous behavior:** Toasts persisted until overwritten. No token count in footer. No context on startup. Section headers minimal.
**New behavior:** Toasts auto-dismiss after 3s. Footer shows "1.2k↑" token badge. Startup shows "Context: ..." from working_set. Section headers have visual rules.
**Tests:** 574 workspace tests, all passing. New: `test_toast_expires_after_ttl`, `test_format_token_count`.
**Rollback:** Restore checkpoint `before-ui-polish` (cp-412d3888).

## 2026-04-16T05:29:00Z — chore: dependency modernization (security audit fixes)

**Summary:** Upgraded transitive dependencies to resolve 4 `cargo audit` advisories (all transitive). Simplified MCP HTTP transport code by leveraging rmcp 1.4's native auth/header support.

**Upgrades:**
- `scraper` 0.19 → **0.26** — fixes `fxhash` (RUSTSEC-2025-0057) + `rand 0.8` (unsound)
- `ratatui` 0.29 → **0.30** — fixes `lru 0.12.5` (RUSTSEC-2026-0002, unsound) + drops `paste`
- `tui-textarea` 0.7 → **`tui-textarea-2` 0.10** — maintained fork compatible with ratatui 0.30
- `ansi-to-tui` 7 → **8** — compatible with ratatui 0.30 (uses ratatui-core)
- `crossterm` 0.28 → **0.29** — aligned with ratatui 0.30
- `rmcp` 0.2 → **1.4** — fixes `paste` (RUSTSEC-2024-0436, uses `pastey` instead)

**Files modified:**
- `Cargo.toml` — workspace dependency versions (ratatui, crossterm, ansi-to-tui, rmcp)
- `crates/cade-web/Cargo.toml` — scraper 0.19 → 0.26
- `crates/cade-tui/Cargo.toml` — tui-textarea → tui-textarea-2
- `crates/cade-mcp/Cargo.toml` — removed reqwest dep, added http crate
- `crates/cade-mcp/src/lib.rs` — rmcp API migration: unified HTTP transport, builder-pattern CallToolRequestParams, RawContent wildcard arm

**Remaining advisories (accepted risk):**
- `bincode 1.3.3` via syntect — no upstream fix, syntect 5.3.0 is latest
- `rand 0.8.5` via phf_generator → termwiz — platform-gated (not compiled for our target)

**Previous behavior:** 5 cargo audit warnings, separate SSE/Streamable-HTTP code paths in MCP client
**New behavior:** 2 audit warnings (accepted), unified HTTP transport with native auth support
**Rollback:** restore checkpoint `before-dep-upgrades` (cp-4d230378)
