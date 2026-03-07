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

---

## 2026-03-07 UTC — Fix: buffered Esc cancels turn immediately (root cause of persistent interruption)

**Summary**: Added a 200 ms grace period to the tick task's Esc handler so that Esc key events
buffered in the terminal from before the agent turn started cannot immediately cancel the turn.

**Root cause**:
The terminal (via crossterm) buffers key events in an OS-level queue. When the user presses Esc
(e.g., to clear input) and then presses Enter to submit a message, the Enter is consumed by
`read_input()`. However, if the Esc was pressed very close to or after the Enter press, it can
remain in the terminal's input buffer and be read by the tick task's `EventStream` when the task
first polls for events.

The tick task is spawned during `agent_turn` (after `cancel_turn.store(false)` at line 2308).
Its first `await` point is inside `tokio::select!` where it polls `reader.next()`. At that
first poll, the runtime schedules the tick task and it immediately processes the buffered Esc.
The Esc handler sets `cancel_turn = true`. When `stream_turn` later reaches `es.next().await`
and receives `Event::Open`, the cancel check fires → "Turn interrupted" — before any LLM
content has been received.

This explains why the issue occurred "consistently": users who type quickly or habitually press
Esc near the end of their typed message would consistently see "Turn interrupted" on the first
response.

**Fix** (`repl.rs`, Esc handler in tick task, line ~2444):
Added a guard: `if tick_start.elapsed().as_millis() >= 200`. `tick_start` is already in scope
(cloned from `turn_start`). Esc events arriving within the first 200 ms of the turn are
silently discarded. After 200 ms, Esc works as before (interrupts the streaming turn).

**Previous behaviour**: Any buffered Esc event processed by the tick task immediately set
`cancel_turn = true`, cancelling the turn before content arrived.

**New behaviour**: Esc events within the first 200 ms of the turn are ignored. Esc pressed
200 ms+ after the turn started still interrupts streaming as expected.

**Files modified**:
- `src/cli/repl.rs` — Esc handler in tick task key-event spin-wait (line ~2444)

**Rollback**: Remove the `if tick_start.elapsed().as_millis() >= 200 { }` guard; restore the
original unconditional `tick_cancel.store(true, ...)` call.

---

## 2026-03-06 UTC — Fix: spurious "Turn interrupted" and blank viewport (root-cause investigation)

**Summary**: Investigation confirmed two remaining gaps in the `cancel_turn` guard coverage
in `execute_tool()`, and a resource leak in the per-turn SIGINT handler. Two targeted fixes
applied.

**Root cause (interruption)**:
`cancel_turn` (AtomicBool) is shared between the SIGINT handler task and the SSE streaming
client. The streaming client fires `__cancelled__` on the very first SSE event (`Event::Open`)
if `cancel_turn == true` — before any LLM content is received. This produces "Turn interrupted"
with zero agent content in the viewport (blank from user's perspective).

Two paths in `execute_tool()` returned early without clearing `cancel_turn`:
1. `is_blocked` permission check (line ~2885) — could carry a stale `true` from a prior
   cancelled loop iteration in `dispatch_tool_calls`.
2. `_sigint_guard` JoinHandle was stored as `_sigint_guard` (underscore prefix keeps the
   variable alive until end of scope, but the SIGINT handler task is NOT aborted on drop).
   After N agent turns, N SIGINT handler tasks accumulate. Each one shares `cancel_turn`.
   A Ctrl+C between turns fires all N tasks, leaving `cancel_turn = true` in a window where
   the next turn's `cancel_turn.store(false)` has not yet run.

**Root cause (blank viewport)**:
Secondary effect of the above: when `cancel_turn == true` at `stream_turn` entry, no
streaming chunk is ever pushed (`streaming_active` stays false), `commit_streaming()` commits
nothing, and only `RenderLine::ErrorMsg("Turn interrupted")` is pushed to `lines`.

**Fix B** (`repl.rs:2310`):
- Renamed `_sigint_guard` → `sigint_handle` so the JoinHandle is live until end of scope.
- Added `sigint_handle.abort()` immediately after `tick_handle.abort()` at the end of
  `agent_turn`, so the SIGINT handler task is explicitly cancelled each turn.
- **Previous behaviour**: JoinHandle dropped (not aborted); task runs indefinitely; N tasks
  accumulate after N turns.
- **New behaviour**: Task aborted at end of each turn; exactly one SIGINT handler active
  per running turn.

**Fix D** (`repl.rs:2885`):
- Added `self.cancel_turn.store(false, SeqCst)` before the early return in the `is_blocked`
  permission check, matching the pattern of the three existing clears at lines 2907/2928/2956.
- **Previous behaviour**: `is_blocked` returned with `cancel_turn` potentially stale `true`,
  causing the subsequent `stream_turn` in `dispatch_tool_calls` to immediately fire
  "Turn interrupted".
- **New behaviour**: `cancel_turn` cleared unconditionally before returning, same as all
  other early-return paths in `execute_tool()`.

**Files modified**:
- `src/cli/repl.rs` — lines ~2310 (rename + abort) and ~2885 (cancel_turn clear)

**Rollback**:
- Fix B: rename `sigint_handle` back to `_sigint_guard`, remove the `sigint_handle.abort()` line.
- Fix D: remove the `self.cancel_turn.store(false, ...)` line before the `is_blocked` return.

---

## 2026-03-07 UTC — P-01/P-02: Add `run_skill_script` and `load_skill_ref` intercepts in headless mode

**Summary**: Added intercept handlers for `run_skill_script` and `load_skill_ref` in `headless.rs:run_one_tool()`. Previously both tools fell through to `dispatch()` which returned "Unknown tool" in headless/CI mode. Now they behave identically to the REPL handlers.

**Root cause**: Both tools were added to `is_sequential_tool()` (preventing parallel dispatch) but no corresponding intercept was added to `run_one_tool()`, so the headless path silently returned an error for every call.

**Files modified**:
- `src/cli/headless.rs` — two intercept blocks inserted after the `// Intercept: load_skill` block, before `// Generic tool dispatch`

**Previous behaviour**: `run_skill_script` and `load_skill_ref` returned `(call_id, "Unknown tool: 'run_skill_script'", true)` in headless mode.

**New behaviour**: Both tools call `discover_all_skills()` to locate the skill, then either execute the script via `tokio::process::Command` (`run_skill_script`) or read the reference file via `std::fs::read_to_string` (`load_skill_ref`), matching the REPL handler logic exactly (minus the TUI `tui_dim` call, replaced with `tracing::info!`).

**Rollback**: Remove the two intercept blocks from `src/cli/headless.rs` (lines between `return (call_id, msg, err); }` for `load_skill` and `// Generic tool dispatch`).

---

## 2026-03-07 UTC — P-03/P-04: Bump version to 0.2.0 and date CHANGELOG

**Summary**: Incremented crate version from `0.1.0` to `0.2.0` and converted the `[Unreleased]` CHANGELOG section to `[0.2.0] — 2026-03-07`.

**Files modified**:
- `Cargo.toml` — `version` field: `"0.1.0"` → `"0.2.0"`
- `CHANGELOG.md` — `## [Unreleased]` → `## [0.2.0] — 2026-03-07`

**Previous behaviour**: `cargo pkgid` reported `cade@0.1.0`; `X-Cade-Version` header emitted `0.1.0`; CHANGELOG had undated `[Unreleased]` section.

**New behaviour**: Version is `0.2.0` across binary, server header, and CHANGELOG.

**Rollback**: Revert `Cargo.toml` version to `"0.1.0"` and `CHANGELOG.md` heading back to `## [Unreleased]`.
