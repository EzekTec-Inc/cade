# CADE Change Log

---

## 2026-03-02 UTC — Reduce whitespace in display view

**Summary**: Eliminated blank-row gaps between the banner and agent output caused by InputWidget viewport cleanup.

**Root cause**: `InputWidget` pre-scrolls `viewport_height` (min 5) rows below the banner to make room for its inline viewport. After the user submits input, those rows are cleared to blank. Subsequent `with_insert_before` calls emitted `height` additional newlines, scrolling the 5 blank rows above newly written content and creating a visible gap on each turn.

**Fix**: Added `blank_rows_at_bottom: u16` tracking to `OutputRenderer`. After `InputWidget.read()` returns, the REPL records the cleared row count via `note_blank_rows(N)`. `with_insert_before` now reuses pre-existing blank rows (emitting `max(0, height-N)` newlines instead of `height`) and compacts any remaining gap using ANSI Delete-Line (`\x1b[nM]`), shifting written content up to be adjacent to the banner.

**Files modified**:
- `src/ui/output.rs` — `blank_rows_at_bottom` field; `note_blank_rows()`; `set_status_bar()` reset; `with_insert_before()` reuse + compact logic
- `src/ui/input.rs` — `last_viewport_height` field; recorded on cleanup
- `src/cli/repl.rs` — `note_blank_rows(last_viewport_height)` injected after `input_widget.read()` returns

---

### 2026-03-03T17:22 UTC — Viewport scroll: mouse wheel + keyboard remap
**Files modified**: `src/ui/app.rs`, `src/cli/repl.rs`
**Reason**: Mouse wheel scroll was completely absent (no `EnableMouseCapture`, no `Event::Mouse` handler). Keyboard scroll keymaps updated per user instruction.
**Previous behaviour**: `PageUp`/`PageDown`/`Alt+Up`/`Alt+Down` = ±10 rows keyboard-only; mouse ignored.
**New behaviour**:
- Mouse wheel `ScrollUp`/`ScrollDown` = ±3 rows (works during input-wait and streaming).
- Keyboard `Shift+K` = +10 rows up; `Shift+J` = -10 rows down (replaces PageUp/PageDown/Alt keymaps).
- Mouse capture enabled on `TuiApp::new()`, disabled on `Drop`.
**Rollback**: `git revert HEAD` (single commit).

---

## 2026-03-03 UTC — Gemini usage parsing & UI responsiveness

**Summary**: Fixed Gemini token usage rendering and improved UI snap-to-bottom behavior.

**Gemini Fix**: Modified `src/server/llm/gemini.rs` to parse `usageMetadata` from the root of the SSE JSON objects. Previously, it only checked within `candidates`, which missed usage updates sent in separate chunks.

**UI Fixes**: 
- Updated `src/ui/app.rs` to snap scroll to bottom (`scroll = 0`) when lines are pushed or streaming is committed.
- Thinking animation (`● tool_name...`) is now correctly initialized in `repl.rs` via `start_thinking`.

**Files modified**:
- `src/server/llm/gemini.rs`
- `src/ui/app.rs`
- `src/cli/repl.rs`

**Status**: Verified via code inspection; pending compiler check for uncommitted changes.

---

## 2026-03-03 UTC — Memory System Enhancements (Letta Code Alignment)

**Summary**: Implemented Shared Memory and Archival Memory structures to align with Letta Code concepts.

**Shared Memory**: 
- Added `shared_memory_blocks` and `agent_memory_blocks` tables to SQLite.
- Enables memory blocks to be shared across multiple agents rather than being strictly siloed.

**Archival Memory**:
- Implemented `messages_fts` using SQLite FTS5 for full-text search across conversation history.
- Added triggers (`messages_ai`, `messages_ad`, `messages_au`) to keep the search index synchronized with the `messages` table.

**Files modified**:
- `src/server/storage/sqlite.rs` — Schema updates for shared blocks and FTS5 virtual table.

**Status**: Backend structures implemented and verified via schema inspection.

## 2026-03-04 UTC - Resolve Issues Found during code review
- Fixed double `RenderLine::ToolResult` push when calling `ask_user_question`.
- Restored word count proxy to the `generating…` state on the thinking bar.
- Removed unused `_stdout` parameter from `handle_run_subagent` and `handle_install_skill`.
- Updated `metadata.json` track status to complete.


## 2026-03-04 UTC - Implement Token Efficiency Improvements
- Reduced `TOOL_RESULT_MAX_CHARS` to 8,192.
- Reduced `CONTEXT_CHAR_BUDGET` to 200,000.
- Reduced `HISTORY_LIMIT` to 100 rows.
- Moved `NO_INTRO_RULE` injection from every turn to agent creation.
- Curated tools for `general-purpose` and `coder` subagents to avoid full toollist injection.
- Disabled injection of memory block descriptions into system prompt.
- Deprecated `skills_context` to prevent token bloat.
- Optimized `auth()` calls in client by evaluating token format once.

## 2026-03-04 UTC - Fix stream_tool_return_cancellable silent error discard
- Explicitly handle `InvalidStatusCode` inside `stream_tool_return_cancellable` to mirror behavior of `stream_message_cancellable`. This correctly propagates server HTTP errors instead of falling back to a non-streaming endpoint.
