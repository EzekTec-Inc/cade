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

---

## 2026-03-07 UTC — Phase 1 Viewport Fixes (V-01 through V-04)

**Summary**: Four targeted fixes to the TUI viewport in `src/ui/app.rs`, inspired by the pi interactive coding agent's non-disruptive scroll model.

**Files modified**:
- `src/ui/app.rs` only

### V-01: Non-disruptive streaming snap

**Problem**: `push_streaming_chunk()` unconditionally set `scroll = 0` on the first streaming chunk, and `commit_streaming()` unconditionally set `scroll = 0` at end of turn. Both forced the user's viewport to snap to the bottom even when they had scrolled up to read prior context.

**Previous behaviour**: Viewport always snapped to bottom the instant the agent started streaming — interrupting reading of history.

**New behaviour**: Neither `push_streaming_chunk` nor `commit_streaming` changes `scroll` when `scroll > 0`. The viewport stays wherever the user left it.

**Rollback**: In `push_streaming_chunk()`, restore `if !self.streaming_active { self.scroll = 0; }`. In `commit_streaming()`, restore `self.scroll = 0;`.

### V-02: Scroll indicator ("new content below")

**Problem**: When scrolled up during streaming, the user had no feedback that the agent was responding.

**Previous behaviour**: No indicator — user couldn't tell if agent was working while reading history.

**New behaviour**: A `pending_lines: usize` field on `TuiApp` counts committed lines pushed while `scroll > 0`. When the user is scrolled up, the status row appends: `↓ streaming… (Shift+J to follow)` during active streaming, or `↓ N new (Shift+J to follow)` for committed lines. Resets to 0 when `scroll` returns to 0 (on Enter, Shift+J to bottom, or mouse scroll-down to bottom).

**Rollback**: Remove `pending_lines` field from `TuiApp` and all references to it; revert status row rendering to original; revert all `if self.scroll > 0 { self.pending_lines += 1; }` guards.

### V-03: Fix count_wrapped_rows for newline-containing spans

**Problem**: `count_wrapped_rows()` concatenated all spans before counting, missing `\n` within span content (e.g., code blocks). Each `\n` forces a new visual row in ratatui regardless of word-wrap width, so the scroll calculation was wrong for content with embedded newlines.

**Previous behaviour**: Single-pass word-wrap count without `\n` splitting — undercounted visual rows for multi-line span content.

**New behaviour**: `count_wrapped_rows` splits the concatenated text on `\n` first, then calls `count_wrapped_segment` (extracted helper) on each segment, and sums. Matches ratatui's actual rendering.

**Rollback**: Restore original `count_wrapped_rows` body (remove the `split('\n')` outer loop and `count_wrapped_segment` helper).

### V-04: Clamp scroll on content shrink

**Problem**: After `commit_streaming`, committed content sometimes wraps to fewer rows than the streaming buffer (markdown rendering differs). `self.scroll` could exceed the new `max_skip`, leaving the viewport stuck at empty space — Shift+J had no effect until `self.scroll` naturally decremented below the new `max_skip`.

**Previous behaviour**: `self.scroll` was never clamped — could be larger than `max_skip` after content change.

**New behaviour**: `render_frame` now returns `max_skip: u16`. After each `draw_impl()` call, `self.scroll` is clamped: `if self.scroll > max_skip as usize { self.scroll = max_skip as usize; }`. The closure is also changed from `move` to non-`move` to allow capturing `max_skip` by `&mut` reference.

**Rollback**: Change `render_frame` back to `-> ()`, remove `max_skip` return; restore `move` on closure; remove clamping lines in `draw_impl`.

---

## 2026-03-07 UTC — Phase 2 Footer & Separator Enhancements (U-01, U-02)

**Summary**: Added CWD and context-window usage % to the footer (U-01), and a cyan pulse on the top separator during agent activity (U-02). Inspired by the pi interactive agent footer design.

**Files modified**:
- `src/ui/app.rs` — struct fields, `new()`, `render_frame`, `abbreviate_cwd`, `set_context_pct`
- `src/cli/repl.rs` — context_pct computation in `usage_statistics` SSE branch

### U-01: Footer CWD and context usage %

**Problem**: Footer showed only mode label, agent name, and model. No working directory or context saturation — both useful for situational awareness.

**Previous behaviour**: Footer: `[mode label] [glyph]  …padding…  agent-name  [model]`

**New behaviour**: Footer: `[mode label] [glyph]  …padding…  ~/…/cwd   agent-name  [model]  N%`
- `cwd` field added to `TuiApp`; populated at construction via `abbreviate_cwd()` (last 2 path components, `~/` prefix when under home dir, `~/…/last2` for deeper paths).
- `context_pct: Option<u8>` field added; updated via `set_context_pct(pct)`.
- In `repl.rs:usage_statistics` branch: after `record_usage`, computes `pct = (input_tokens + cache_read_tokens) * 100 / context_window_for_model(model)` and calls `app_arc.lock().unwrap().set_context_pct(pct)`.
- Shown as `N%` in dark gray after the model name; hidden (no span) when `context_pct` is `None`.

**Rollback**: Remove `cwd` and `context_pct` fields from `TuiApp`; remove `abbreviate_cwd` helper; remove `set_context_pct` method; revert footer rendering to original; remove the context_pct block from `repl.rs` `usage_statistics` branch.

### U-02: Top separator thinking level indicator

**Problem**: No peripheral visual signal that the agent was actively working. Users had to watch the status bar spinner.

**Previous behaviour**: Both separators always showed the static mode color.

**New behaviour**: The TOP separator (between messages pane and input) pulses through the same 4-step cyan palette `(80,190,255)→(120,215,255)→(160,235,255)→(100,200,255)` at 400 ms/step when `thinking_elapsed.is_some()` (animated). When only `streaming.is_some()` (text streaming, thinking animation stopped), shows a fixed `Rgb(80,190,255)`. When idle, reverts to `mode_sep_color(mode)`. The BOTTOM separator always uses `mode_sep_color(mode)`.

**Rollback**: Revert the separators section to use `mode_sep_color(mode)` for both chunk[4] and chunk[6] with no conditional branching.

---

## 2026-03-07 UTC — Phase 3 Input UX Enhancements (I-01, I-02, I-03)

**Summary**: Three input UX improvements inspired by the pi interactive agent: message queue during agent turns, Tab path completion, and `!!cmd`/`!cmd` bash distinction.

**Files modified**:
- `src/cli/repl.rs` — I-01 (queue fields, tick task extension, drain), I-03 (`!!` / `!` distinction)
- `src/ui/app.rs` — I-02 (`complete_path`, `common_prefix`, Tab handler)

### I-01: Message queue — Enter vs Alt+Enter semantics

**Problem**: During an agent turn the user could only cancel (Esc) or scroll. Typing a message required waiting for the turn to finish.

**Previous behaviour**: No input editable during a turn. Enter had no effect.

**New behaviour**:
- User can type into the input field at any time during an agent turn.
- `Enter` (plain) — queues a **steering** message: stores it in `Repl::queued_steering`, clears the input, and sets `cancel_turn = true`. The current turn is interrupted; the queued message runs as the next turn.
- `Alt+Enter` / `Shift+Enter` — queues a **follow-up** message: stores it in `Repl::queued_followup`, clears the input, does NOT cancel. The follow-up runs immediately after the current turn finishes.
- `Esc` (during turn, after 200 ms grace) — if input is non-empty, clears it (discards draft); if input is empty, cancels the turn as before.
- Regular characters (`KeyModifiers::NONE | SHIFT`) are inserted into `app.input`; `Backspace` removes the character before the cursor.
- After every `agent_turn()` call in the main loop: `queued_followup` is drained first (follow-up takes priority), then `queued_steering`, into `pending_input`.

**New fields on `Repl`**:
- `queued_steering: Arc<Mutex<Option<String>>>`
- `queued_followup: Arc<Mutex<Option<String>>>`

**Rollback**: Remove the two struct fields and their `Repl::new()` initialization; remove the new match arms (Char/Backspace/Enter/Alt+Enter and the revised Esc) from the tick task; remove the queue-drain block after `agent_turn()`.

### I-02: Tab path completion

**Problem**: Tab only cycled permission modes; no filesystem completion existed.

**Previous behaviour**: Tab always returned `__TAB__` sentinel → mode cycle.

**New behaviour**: Tab first calls `complete_path(input, cursor_pos)`. If the token at the cursor starts with `/`, `./`, `~/`, or contains `/`, the function lists the matching directory entries, computes their common prefix, and replaces the token in-place. If exactly one match and it's a directory, a trailing `/` is appended. If no path context or no completions found, falls through to the original `__TAB__` mode-cycle behaviour.

**New helpers in `src/ui/app.rs`**:
- `fn complete_path(input: &str, cursor: usize) -> Option<(String, usize)>`
- `fn common_prefix(words: &[String]) -> String`

**Rollback**: Remove `complete_path` and `common_prefix`; revert Tab handler to unconditionally return `Ok(Some(Some("__TAB__".to_string())))`.

### I-03: `!!cmd` / `!cmd` bash shorthand

**Problem**: `!cmd` ran a bash command and showed output but never sent it to the agent. There was no way to run a command and feed the result into the LLM in one step.

**Previous behaviour**: `!cmd` → run command, display output, `continue` (no agent turn).

**New behaviour**:
- `!!cmd` → run silently: display output to user, no agent turn (preserves prior `!cmd` behaviour).
- `!cmd` → run and forward: display output to user AND run `agent_turn` with `"Command: \`cmd\`\n\nOutput:\n\`\`\`\n{output}\n\`\`\`"` as the message, so the agent can reason about the output.

**Rollback**: Remove the `!!` prefix check; restore original single-prefix `!cmd` block with `continue` and no `agent_turn` call.

---

## 2026-03-07 UTC — Phase 4 Advanced Features (A-01, A-02)

**Summary**: `@` file fuzzy picker overlay (A-01) and extensibility slots `header_lines` / `footer_extra` (A-02).

**Files modified**:
- `src/ui/app.rs` only

### A-01: `@` file fuzzy picker

**Problem**: No way to reference project files in input without manually typing paths.

**Previous behaviour**: Typing `@` inserted a literal `@` character with no special action.

**New behaviour**:
- Typing `@` in the input activates a picker overlay at the bottom of the messages pane showing matching project files (up to 50, depth-limited to 3 levels, skipping `.hidden`, `target`, `node_modules`, `.git`).
- While the picker is active: Up/Down navigate; any printable char appends to the query and filters matches live; Backspace removes the last query char (removing `@` dismisses the picker); Enter inserts the selected path at the `@` position; Esc dismisses without inserting.
- All normal key handling is bypassed while the picker is active — it intercepts keys first and returns `Ok(None)` to stay in the input loop.

**New structs**: `PickerState { at_pos, query, matches, cursor }` on `TuiApp`.
**New helpers**: `collect_files(root, query) -> Vec<String>` (depth-limited walk), `collect_files_inner(...)`, `render_picker(frame, pk, area)` (dark-background overlay with dashed separator, `@ query` header, highlighted entries).
**New field**: `pub picker: Option<PickerState>` on `TuiApp`.

**Rollback**: Remove `PickerState` struct; remove `picker` field from `TuiApp` and `new()`; remove picker routing block from `handle_key_input`; remove `@` activation in char handler; remove `collect_files`, `collect_files_inner`, `render_picker`; remove picker overlay rendering from `render_frame`; remove `picker` snapshot from `draw_impl` and parameter from `render_frame` signature.

### A-02: Extensibility slots (header_lines / footer_extra)

**Problem**: No mechanism for callers to populate named layout regions. The startup banner scrolled into the messages pane and disappeared; no extension status line existed.

**Previous behaviour**: All content went into `lines` (scrollable). No pinned header. No second footer row.

**New behaviour**:
- `pub header_lines: Vec<RenderLine>` on `TuiApp`: rendered as a **fixed, non-scrollable strip** at the top of the messages pane. The scrollable content area uses the remaining height. The header_h is capped at 1/3 of the content pane. Does not contribute to `max_skip` (scroll) calculation.
- `pub footer_extra: Option<String>`: when `Some`, rendered as a second dim-gray row immediately below the footer. Adds 1 to `bottom_rows` so the layout correctly reserves the space.

**New fields**: `pub header_lines: Vec<RenderLine>`, `pub footer_extra: Option<String>` (both initialized to empty/None in `new()`).

**Rollback**: Remove both fields from `TuiApp` and `new()`; remove their snapshots from `draw_impl`; remove from `render_frame` signature; remove `footer_extra_h`, header split, and footer_extra rendering from `render_frame` body.

---

## 2026-03-07 UTC — Dead code cleanup (deep reflection audit)

**Summary**: Removed 1,704 lines of legacy UI files and 54 lines of dead code from active files, confirmed by full codebase grep. Zero functional changes — compilation verified clean before and after.

**Files deleted**:
- `src/ui/output.rs` (994 lines) — Pre-TuiApp `OutputRenderer` using DECSTBM + Inline viewport hybrid. Not declared in `ui/mod.rs`; never compiled; fully superseded by `TuiApp`.
- `src/ui/input.rs` (596 lines) — Pre-TuiApp `InputWidget` using `Viewport::Inline`. Not declared in `ui/mod.rs`; never compiled; fully superseded by `TuiApp::handle_key_input`.
- `src/ui/status.rs` (114 lines) — Pre-TuiApp `ThinkingBar`. Not declared in `ui/mod.rs`; never compiled; superseded by `TuiApp::ThinkingState`.

**Code removed from active files**:
- `src/ui/app.rs` — Removed `// ── Compatibility shims` section: `pub struct RawModeGuard` + `impl` (6 lines) and `pub fn make_relative_path` (21 lines). Both were exported from `ui/mod.rs` but confirmed unused by grep across entire `src/`.
- `src/ui/mod.rs` — Removed `RawModeGuard` and `make_relative_path` from `pub use app::{...}` re-export.
- `src/agent/client.rs` — Removed `#[allow(dead_code)]` struct `SendMessageRequest` (9 lines) and `#[allow(dead_code)]` struct `ToolReturnRequest` (7 lines). Removed `#[allow(dead_code)]` attribute from `list_agents()` (method is actively used by `/agents` REPL command and `export_import.rs`; the suppression was wrong).

**Rollback**: `git show HEAD~1` — all deleted files recoverable from git history. In-file removals: restore the `// ── Compatibility shims` section in `app.rs`, restore both structs in `client.rs`, restore the `#[allow(dead_code)]` attribute on `list_agents`.

---

## 2026-03-07 UTC — fix(approval): I-01 Enter handler cancels turn after modal close

**Summary**: Added a 300 ms grace period to the I-01 steering Enter handler in the tick task, preventing a buffered confirmation Enter from a blocking question modal from cancelling the subsequent `stream_turn` call. Mirrors the existing 200 ms Esc grace period pattern.

**Root cause**: The Phase 3 I-01 implementation added an Enter key handler in the tick task (repl.rs ~line 2483) that sets `cancel_turn = true` when `app.input` is non-empty. This handler correctly fires during normal turns to queue steering messages. However, after a blocking question modal closes (`blocking_question_active = false`), a buffered Enter key event from the user's modal confirmation can linger in the crossterm event queue for up to ~300 ms. The tick task processes this Enter, finds `app.input` non-empty (from chars typed while agent was running), and sets `cancel_turn = true`. The subsequent `stream_turn` call (to get the LLM's response after the tool result) is immediately cancelled, producing "Turn interrupted" (pushed to lines but potentially invisible due to V-01 non-snapping scroll).

**Fix**: Added `last_modal_close_ms: Arc<AtomicU64>` to `Repl`. It is set to `SystemTime::now().as_millis()` at both `blocking_question_active.store(false)` call sites (in `prompt_approval` and `handle_ask_user_question`). The I-01 Enter handler now checks: if `now - last_modal_close_ms < 300 ms`, skip the cancel logic entirely.

**Files modified**:
- `src/cli/repl.rs` — struct field + init; timestamp set at 2 call sites; tick Arc clone; Enter handler guard

**Previous behaviour**: Enter key confirmation in blocking modal could race into the I-01 steering handler and cancel the subsequent stream_turn, causing missing/interrupted agent responses.

**New behaviour**: Enter events within 300 ms of a modal close are silently discarded by the I-01 handler. After 300 ms, Enter with non-empty input still works as designed (queues steering message, cancels turn).

**Rollback**: Remove `last_modal_close_ms` field and init; remove the two `self.last_modal_close_ms.store(...)` blocks; remove `tick_modal_close_ms` clone; revert the Enter handler to the original (remove the `post_modal` guard).

---

## 2026-03-07 UTC — fix(approval): skip cancel check on Event::Open in tool-return stream

**Summary**: Root-cause fix for the "agent stops silently after tool approval" bug. The previous fix (300 ms grace period in the I-01 Enter handler) addressed one contributing factor but not the core issue. The actual problem is that `stream_tool_return_cancellable` checks `cancel_turn` on the very first SSE event (`Event::Open`), which fires as soon as the HTTP connection is established — before any agent response content arrives. Any residual `cancel_turn = true` (from the approval modal Enter, a buffered Esc, a prior SIGINT, or any other source) would silently kill the tool-return stream at that point, preventing the agent's continued response from ever being received or displayed.

**Root cause**: `stream_tool_return_cancellable` in `src/agent/client.rs` checked the cancel flag at the top of the event loop, before the `match` statement, meaning it fired on every event including `Event::Open`. The tool result HTTP POST is fully delivered to the server before `Event::Open` fires (it's the connection establishment response), so cancelling there serves no purpose — but it reliably swallowed the agent's follow-up response.

**File modified**: `src/agent/client.rs` only.

**Previous behaviour**: Cancel check ran on every SSE event including `Event::Open`. Any `cancel_turn = true` present when `Event::Open` arrived → `__cancelled__` returned → "Turn interrupted" pushed (possibly off-screen due to V-01) → agent's response after tool never displayed.

**New behaviour**: Added `let mut opened = false;` before the loop. The cancel check is guarded by `opened &&` so it fires only AFTER `Event::Open` sets `opened = true`. For all subsequent message events, cancellation still works normally (Esc during streaming still interrupts). The initial message stream (`stream_message_cancellable`) is unchanged — cancelling before the agent replies to a user message is a valid and expected use case.

**Rollback**: Remove the `opened` boolean; change `if opened && cancel...` back to `if cancel...`; change `Ok(Event::Open) => { opened = true; }` back to `Ok(Event::Open) => {}`.

---

## 2026-03-07 UTC — fix(scroll): snap viewport to bottom after blocking modal approval

**Summary**: True root-cause fix for the "session appears to quit after modal approval" bug. After deep investigation it was confirmed the session does NOT actually quit — the tool results and agent responses are added to `lines` and generated correctly, but they land BELOW the user's current scroll position and are therefore invisible. The user sees nothing and perceives the session as having ended.

**Root cause**: `ask_question_blocking` (app.rs) sets `scroll = 0` when the modal OPENS, but NOT when it closes. While the modal is visible, the tick task continues to process mouse scroll events (`ScrollUp/Down`) which can change `scroll` to > 0. When the modal closes and tool execution + streaming follows, V-01's non-snapping design keeps the viewport at that elevated scroll position. Every push (ToolResult, AssistantText via commit_streaming) increments `pending_lines` instead of being visible. The `↓ N new (Shift+J to follow)` indicator appears in the status bar but is easy to miss.

**File modified**: `src/ui/app.rs` — `ask_question_blocking()` only.

**Previous behaviour**: After modal approval, `scroll` remained > 0 if the user (or any scroll event) had changed it during the modal. Subsequent content was invisible; user perceived session as having quit.

**New behaviour**: Two lines added after `self.active_question = None;` — `self.scroll = 0; self.pending_lines = 0;` — snap viewport to bottom unconditionally when any blocking modal (prompt_approval or ask_user_question) closes. Both `prompt_approval` and `handle_ask_user_question` call `ask_question_blocking`, so this single change covers both.

**Rollback**: Remove the two lines `self.scroll = 0;` and `self.pending_lines = 0;` from `ask_question_blocking` after `self.active_question = None;`.

---

## 2026-03-07 UTC — fix(scroll): snap on push() and commit_streaming() — tool results always visible

**Summary**: Tool results, tool calls, system messages, and completed agent responses were invisible when the user was scrolled up, because V-01's non-snapping design was applied too broadly to ALL content. The correct design is: only live streaming chunks (push_streaming_chunk) should preserve scroll; all committed content should snap to bottom.

**Problem**: V-01 changed `push()` to increment `pending_lines` instead of snapping. It also removed the snap from `commit_streaming()`. This meant: after a tool ran, its ToolResult was added to `lines` but the viewport stayed at the old scroll position. Same for any final agent response committed by `commit_streaming()`. Users would see nothing new even though the agent was working normally.

**Fix** (`src/ui/app.rs` only):
- `push()`: replaced `if self.scroll > 0 { self.pending_lines += 1; }` with `self.scroll = 0; self.pending_lines = 0;` — all committed pushes (ToolCall, ToolResult, SystemMsg, ErrorMsg, AssistantText, QuestionResult, etc.) now snap to bottom unconditionally.
- `commit_streaming()`: replaced the non-snap pending_lines increment with `self.scroll = 0; self.pending_lines = 0;` — when a streaming response completes and is committed to `lines`, the viewport shows it immediately.
- `push_streaming_chunk()`: **unchanged** — mid-stream chunks still preserve the user's scroll position (V-01 non-snap preserved for live typing).

**Net behavior**:
- User can scroll up while agent is streaming → reading preserved (V-01 still in effect)
- Agent finishes a streaming response → viewport snaps to show it
- Agent calls a tool → ToolCall visible immediately
- Tool returns a result → ToolResult visible immediately
- Any system/error message → visible immediately

**Rollback**: In `push()`, restore `if self.scroll > 0 { self.pending_lines += 1; }` and remove the two unconditional assignment lines. In `commit_streaming()`, restore `if self.scroll > 0 { self.pending_lines += 1; }` and remove the two unconditional assignment lines.

---

## 2026-03-07 UTC — fix(cancel): clear stale cancel_turn at Event::Open in stream_tool_return_cancellable

**Summary**: Definitive fix for "turn always quits after ask_question modal selection." Previous fixes (`opened &&` guard, 300ms grace, scroll snaps) addressed visible symptoms but left a race window between `Event::Open` and the first `Message` event where the tick task could still set `cancel_turn = true`. That cancel fired on the first actual message chunk, producing "Turn interrupted" and ending the turn before any response appeared.

**Root cause**: Any `cancel_turn = true` set between `Event::Open` arriving and the first `Message` event (e.g., tick task processing a stale Esc key or I-01 Enter with non-empty input) caused the stream to abort on the very next SSE event. The `opened &&` guard only prevented cancel at `Event::Open` itself — not afterwards.

**Fix** (`src/agent/client.rs`): Inside `stream_tool_return_cancellable`, in the `Ok(Event::Open)` handler: after setting `opened = true`, unconditionally clear `cancel_turn` via the passed `cancel` reference. This eliminates ALL stale cancel flags (from modal approval, SIGINT, buffered Esc, anything) at the exact moment the HTTP connection is established and the agent's response is about to start flowing. Any cancel after this point (user presses Esc during actual streaming) is still honoured.

**Previous behaviour**: Any cancel_turn=true accumulated before or during Event::Open → fired on first Message event → "Turn interrupted" → turn ends silently.

**New behaviour**: cancel_turn is cleared at Event::Open. The response streams in. Intentional cancellations (Esc during streaming) still work because they set cancel_turn AFTER the clear.

**Rollback**: Remove the `if let Some(c) = cancel { c.store(false, ...); }` block from inside `Ok(Event::Open)` arm; restore `Ok(Event::Open) => { opened = true; }`.
