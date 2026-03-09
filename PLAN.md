# CADE Change Log

---

## 2026-03-08 UTC — Skills TUI cards overlay

**Summary**: Added a full-screen skills browser/editor overlay activated by `/skills`, `/skills show <id>`, and `/skills edit <id>`. Replaces the previous table/text dump with an interactive 3-mode UI.

**New behaviour**:
- `/skills` (non-empty) → opens full-screen overlay in List mode: bordered cards (j/k nav, Enter → Detail, e → Edit, Esc closes)
- `/skills show <id>` → opens overlay in Detail mode for the named skill
- `/skills edit <id>` → opens overlay in Edit mode (Tab between 6 fields, Ctrl+S saves, Esc cancels)
- Edit mode writes back to SKILL.MD on disk via `write_skill_to_disk()`
- Empty skills list still falls through to the existing info message path

**Files modified**:
- `src/skills/mod.rs` — added `write_skill_to_disk()` function
- `src/ui/app.rs` — added `SkillsMode` enum, `SkillsOverlayState` struct, `skills_overlay` field on `TuiApp`, `handle_skills_key()` method, `render_skills_overlay/list/detail/hint` render functions; threaded snapshot through `draw_impl`→`render_frame`; key intercept in `read_input`; added `Borders` import
- `src/ui/mod.rs` — re-exported `SkillsOverlayState` and `SkillsMode`
- `src/cli/repl.rs` — replaced `list`, `show`, `edit` sub-command output with overlay activation

**Previous behaviour**: `/skills` printed a table to the conversation view; `/skills show` dumped field pairs + body; `/skills edit` opened `$EDITOR` in a subprocess.

**Rollback**: Revert the four files above. The `write_skill_to_disk` function is purely additive so can remain.

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

---

## 2026-03-07 UTC — fix(scroll): scroll ToolResult into full view from ToolCall header

**Summary**: After a tool run, the ToolCall header (⚡) was scrolling off-screen when diff-preview lines sat between it and the ToolResult. `push()` was snapping to `scroll=0` (absolute bottom) for every line including each diff preview line, and `scroll=0` shows only the final rows. For a 20-line diff, the ToolCall ended up 22 rows from the bottom — outside a typical 18-row content area.

**Root cause**: `push()` used `scroll=0` unconditionally for all committed lines. Diff preview lines (one per old/new code line) were each snapping to absolute bottom, pushing the ToolCall progressively off-screen. By the time ToolResult arrived, the ToolCall was completely clipped.

**Fix** (`src/ui/app.rs`):
- Added `rows_from_last_tool_call(&self) -> usize` helper: iterates `self.lines` backwards, accumulates visual row counts via `render_line_to_text` + `count_wrapped_rows` until it hits the most recent `ToolCall`. Returns that total.
- `push()`: for `RenderLine::ToolResult`, sets `self.scroll = self.rows_from_last_tool_call()` instead of 0. This positions the viewport so the ToolCall header is at the TOP of the visible area and the ToolResult is at the bottom — the entire tool execution block scrolls into view as a unit.
- All other pushes still use `scroll=0` (snap to absolute bottom).

**Behaviour**:
- Simple tool (no diff): ToolCall at row 1 from bottom, ToolResult at row 0 → scroll=2 or similar, both fully visible.
- Large diff (20 lines): scroll=22, ToolCall at top, all diff lines + ToolResult below it.
- No ToolCall found (edge case): returns 0, falls back to absolute bottom.

**Rollback**: In `push()`, change `self.scroll = self.rows_from_last_tool_call()` back to `self.scroll = 0` for ToolResult; remove the `rows_from_last_tool_call` method.

---

## 2026-03-07 UTC — feat(ui): Claude Code-style tool rendering (● / ⎿ / show-N-lines)

**Summary**: Refactored ToolCall and ToolResult rendering in `src/ui/app.rs` to match Claude Code's visual language as shown in the user's screenshot.

**Files modified**: `src/ui/app.rs` (render_line_to_text function) only.

**ToolCall changes**:
- Symbol: `⚡` (yellow) → `●` (teal Rgb(100,207,180))
- Format: `● Name (args)` → `● Name(args)` — no space before `(`, matching `● Bash(cmd)` style
- Args truncation: appends `…)` when over budget rather than replacing entire args with `(…)`

**ToolResult changes**:
- Gutter symbol: `↳` → `⎿` — matches Claude Code's output indent glyph
- Collapsed (ctrl+o off): previously showed `output hidden (N lines)` (a single dim line with no content); now shows first 3 lines of actual content then `… +N lines (ctrl+o to expand)`
- Expanded (ctrl+o on): previously showed up to 10 lines; now shows up to 20 lines then `… +N lines`
- Both modes share the same first-line-bold + subsequent-indented-lines format
- Empty result: `↳ success` → `⎿  (no output)` italic

**Rollback**: Restore the original `⚡ ` / space-before-paren / `output hidden` / `↳` rendering in the ToolCall and ToolResult arms of `render_line_to_text`.

---

## 2026-03-07 UTC — fix(scroll): snap to bottom on first streaming chunk after tool run

**Summary**: After a tool completes, the agent's analysis/response was invisible during streaming. `push(ToolResult)` sets `scroll = rows_from_last_tool_call()` (a positive value to show the ToolCall header). `push_streaming_chunk` previously only snapped to bottom when `scroll == 0`, so every streaming chunk arrived below the visible area. The response appeared all at once only when `commit_streaming()` finally ran — making the turn feel frozen.

**Root cause**: `push_streaming_chunk`'s snap guard was `if !self.streaming_active && self.scroll == 0` — the `scroll == 0` condition blocked the snap when `push(ToolResult)` had scrolled up.

**Fix** (`src/ui/app.rs`): On the FIRST chunk of a new streaming session (`!self.streaming_active`), unconditionally set `scroll = 0` and `pending_lines = 0`. Subsequent chunks of the same response still preserve scroll (V-01 — user can scroll up mid-stream to read history).

**Behaviour now**: ToolResult appears with ToolCall header visible (scroll up). First streaming chunk → snaps to bottom so agent's analysis streams in live. User scrolls up mid-stream → reading preserved. Streaming commits → snap to show full response.

**Rollback**: Restore the guard to `if !self.streaming_active && self.scroll == 0 { self.scroll = 0; }` (the original no-op form).

---

## 2026-03-07 UTC — /skills page modern UI/UX refactor

**Summary:** Refactored all `/skills` subcommand output in the TUI for improved scannability and consistency.

**Files modified:** `src/cli/repl.rs`

**Reason:** Reduce visual noise, improve information density, and unify hint-line formatting across all `/skills` subcommands.

**Previous behaviour:**
- `/skills list`: per-scope `InfoHeader` banners + separate `Table` per scope; `Category` column; 5 separate `DimMsg` hint lines.
- `/skills show`: `Blank` after header; `InfoHeader` for section labels ("── Scripts ──", "── Body ──"); individual `Pair` per script; `|` hint separator.
- `/skills create`: hint said "Edit the file, then run /skills reload to activate it."
- `/skills edit`: hint said "Run /skills reload to pick up changes."
- `/skills delete`: no post-delete hint line.
- `/skills reload`: success said "✓ Reloaded: N skills (was M)".
- `other` arm: usage hints had extra "Usage:" label and 4-space indent.

**New behaviour:**
- `/skills list`: single unified `Table` with `Scope` column (replaces per-scope sections and `Category` column); 1 condensed `DimMsg` hint line with `·` separators.
- `/skills show`: no `Blank` after header (tighter); `DimMsg` for section labels; scripts rendered as `Table`; `·` hint separator.
- `/skills create`: hint shows `/skills edit <slug>` and `/skills reload` as command-first actions.
- `/skills edit`: hint is `/skills reload  to apply changes`.
- `/skills delete`: adds `/skills reload  to update agent context` hint.
- `/skills reload`: success is "✓ Skills reloaded  (N loaded, was M)".
- `other` arm: removed redundant "Usage:" label; hints left-aligned at 2 spaces.

**Rollback:** `git revert HEAD` — or restore the `SlashCmd::Skills` match arms in `src/cli/repl.rs` (~line 1630).

---

## 2026-03-07 UTC — Enable Shift+Enter newline via kitty keyboard enhancement

**Summary:** Enabled crossterm keyboard enhancement protocol (DISAMBIGUATE_ESCAPE_CODES) so terminals that support it can distinguish Shift+Enter from plain Enter, allowing Shift+Enter to insert a newline and expand the input field.

**Files modified:** `src/ui/app.rs`

**Reason:** Without `PushKeyboardEnhancementFlags`, terminals send identical byte sequences for Enter and Shift+Enter. The key handler already handled `KeyModifiers::SHIFT | KeyCode::Enter` correctly (inserting `\n` at cursor); the terminal simply never delivered the distinction.

**Previous behaviour:** Shift+Enter submitted input (indistinguishable from plain Enter in most terminals).

**New behaviour:** Shift+Enter inserts a newline and expands the input box on kitty-protocol-capable terminals (kitty, WezTerm, foot, etc.). Terminals without kitty support fall back gracefully — plain Enter still submits, Alt+Enter still inserts newlines universally.

**Changes:**
- Added `PushKeyboardEnhancementFlags`, `PopKeyboardEnhancementFlags`, `KeyboardEnhancementFlags` to crossterm imports.
- Added `use crossterm::terminal::supports_keyboard_enhancement`.
- `TuiApp::new()`: conditionally push `DISAMBIGUATE_ESCAPE_CODES` after `EnableMouseCapture`.
- `Drop for TuiApp`: conditionally pop enhancement flags before `DisableMouseCapture`.

**Rollback:** Remove the `supports_keyboard_enhancement` blocks from `TuiApp::new()` and `Drop for TuiApp`, and remove the three new import items.

---

## 2026-03-07 UTC — Fix V-05: input field visual artifact on tool error

**Summary:** Fixed a one-frame visual gap above the input field that appeared when any `ToolResult` (particularly `is_error: true`) was pushed in short/early conversations.

**Files modified:** `src/ui/app.rs`

**Reason:** V-04 clamped `self.scroll` AFTER `draw()` had already committed the overcorrected frame to the terminal. V-05 detects the clamp and immediately issues a corrective redraw so the first visible frame is always correct.

**Previous behaviour:** `rows_from_last_tool_call()` could return a scroll value larger than `max_skip` when conversation content was shorter than the viewport height (early sessions). The first frame after a `ToolResult` push would render with this overcorrected scroll, showing a blank gap above the input field's top separator. V-04 would correct `self.scroll` only after that frame was already visible.

**New behaviour:** After `draw()` in `push()`, if `self.scroll != scroll_before` (V-04 fired), `draw()` is called a second time immediately with the corrected value. The second draw only fires when V-04 actually fires — no overhead in the common case (long conversations where scroll is already valid).

**Change:** `push()` in `src/ui/app.rs` — added `scroll_before` local, changed terminal `self.draw()` to `self.draw()?`, added V-05 guard block (4 lines total).

**Rollback:** Remove `scroll_before` local and the V-05 `if` block from `push()`; change `self.draw()?` back to `self.draw()`.

---

## 2026-03-07 UTC — Add /context slash command

**Summary:** Added `/context` slash command showing context window usage: model name, context window size, approximate tokens used/free, and a 20-character visual bar.

**Files modified:** `src/cli/repl.rs`

**Reason:** User requested a `/context` view mirroring Claude Code's context display for visibility into context window consumption.

**Previous behaviour:** No `/context` command existed. Context usage was shown only as a bare percentage in the TUI footer (e.g., `69%`).

**New behaviour:** `/context` displays:
- Model name (provider prefix stripped)
- Context window size (from `context_window_for_model()` catalogue)
- Approximate used tokens and percentage with a 20-char `█░` visual bar
- Approximate free tokens and percentage
- Hint line pointing to `/stats` and `/stats model` for detailed breakdowns
- "No context data yet" message if invoked before the first agent turn

Token counts are derived as `pct × window / 100` (error ≤ 1% of window size); exact per-category breakdown (system/tools/messages) is not available from the API.

**Rollback:** Remove `Context` from `SlashCmd` enum, remove `"context"` parse arm in `parse_slash_with_skills()`, remove `SlashCmd::Context` match arm in `src/cli/repl.rs`.

---

## 2026-03-07 UTC — Fix Gemini 400: preserve thought_signature on tool calls

**Summary:** Fixed Gemini 400 "Function call is missing a thought_signature" error that occurred when using thinking/reasoning Gemini models with tools.

**Files modified:**
- `src/server/llm/mod.rs`
- `src/server/llm/gemini.rs`
- `src/server/llm/anthropic.rs`
- `src/server/llm/openai.rs`

**Reason:** When Gemini uses thinking/reasoning, each `functionCall` part in the model response includes an opaque `thought_signature` token. This must be echoed back verbatim in subsequent conversation turns. The code was silently discarding this field because `LlmToolCall` had no field to hold it, causing every second tool-using turn to 400.

**Root cause chain:**
1. `gemini.rs` parsed `functionCall` parts extracting only `name` and `args` — `thought_signature` was discarded
2. `LlmToolCall` struct had no `thought_signature` field
3. History reconstruction in `to_gemini_contents()` built `functionCall` JSON from `LlmToolCall` — `thought_signature` absent
4. Gemini rejected the request with 400

**Fix:**
- Added `thought_signature: Option<String>` (with `#[serde(default, skip_serializing_if = "Option::is_none")]`) to `LlmToolCall`
- Gemini streaming and non-streaming parsers now extract `fc["thought_signature"]` into the field
- `to_gemini_contents()` history reconstruction now includes `thought_signature` in the `functionCall` JSON when present
- Anthropic and OpenAI construction sites explicitly set `thought_signature: None`

**Backward compatibility:** `#[serde(default)]` ensures old SQLite rows (no `thought_signature` field) deserialize correctly. Non-Gemini providers are unaffected — field is omitted from serialization when `None`.

**Rollback:** Remove `thought_signature` field from `LlmToolCall`, revert the 3 Gemini code sites, remove `thought_signature: None` from Anthropic/OpenAI construction sites.

---

## 2026-03-07 UTC — Fix aggressive re-prompting in agent turn loop

**Summary:** Reduced over-triggering of the empty-response re-prompt by tracking
whether the model produced any text earlier in the same turn.

**Files modified:** `src/cli/repl.rs`

**Reason:** Re-prompting was firing after every tool in a multi-tool chain (because
`reprompt_done` reset to `false` on each tool-return), and also when the model
had already spoken before calling a tool and then finished silently. This caused
unnecessary "re-prompting" system messages and extra LLM calls even when the turn
was already complete from the user's perspective.

**Previous behaviour:** `dispatch_tool_calls(..., reprompt_done)` — re-prompt fired
whenever the LLM produced no text after a tool return, regardless of whether text
had been produced earlier in the turn. `reprompt_done` reset to `false` on every
new tool-return chain, so a 5-tool sequence could trigger 5 re-prompts.

**New behaviour:** Added `turn_has_text: bool` parameter to `dispatch_tool_calls`.
Re-prompt condition is now `empty && !reprompt_done && !turn_has_text`. The flag
accumulates as `turn_has_text || response_had_text` across all tool-chain steps.
Re-prompting only fires when the model has been completely silent throughout the
entire agent turn (no text anywhere before or after any tool call).

**Rollback:** Remove `turn_has_text` parameter from `dispatch_tool_calls`, remove
`response_had_text` computation, revert the re-prompt condition to
`assistant_msg.trim().is_empty() && !reprompt_done`, and update all call sites
to remove the last argument.

---

## 2026-03-08 UTC — Fix MCP errors + TUI tracing corruption

**Summary:** (1) Redirect tracing output to `/tmp/cade.log` to prevent WARN/ERROR log lines from appearing in the TUI input area. (2) Skip MCP reconnect for JSON-RPC protocol errors — the server is alive, reconnecting wastes 6 seconds.

**Files modified:** `src/main.rs`, `src/mcp/mod.rs`

**Bug 1 root cause:** `tracing_subscriber` wrote to stderr. In crossterm alternate-screen mode only stdout is redirected to the alt buffer — stderr writes go directly to the terminal at the current cursor position (the input field), corrupting the display. Confirmed by user screenshot showing raw WARN log lines appearing in the input area.

**Bug 2 root cause:** `call_tool()` in `mcp/mod.rs` treated ALL `Err(e)` from the rmcp peer as connection failures and triggered 3 reconnect attempts (2s delay each = 6s total). JSON-RPC protocol errors like `-32602` (Invalid params) mean the server received and understood the call but rejected the arguments — the connection is alive, reconnecting wastes time. The user saw `MCP reconnect attempt 1/3…` for every call with bad params.

**Changes:**
- `src/main.rs`: Open `/tmp/cade.log` as an append-mode file, wrap in `Mutex<Box<dyn Write + Send + Sync>>`, pass as `with_writer(...)` to `tracing_subscriber::fmt()`. Fallback to `std::io::sink()` (discard) if file can't be opened.
- `src/mcp/mod.rs`: Added `is_rpc_protocol_error(msg)` helper that detects "Mcp error:" prefix (rmcp's JSON-RPC error format). Added early-return guard before the reconnect loop to return the error immediately for protocol errors.

**Rollback:** Revert `with_writer()` back to `std::io::stderr` in `main.rs`; remove `is_rpc_protocol_error()` and the early-return guard in `mcp/mod.rs`.


---

## 2026-03-08 UTC — Efficiency: truncate safety, tool result cap, timeout, parallel dispatch, auto-wire

**Summary:** Five efficiency improvements across token consumption and tool execution.

**Files modified:**
- `src/cli/mod.rs`
- `src/server/api/messages.rs`
- `src/cli/repl.rs`
- `src/server/api/agents.rs`

---

### Fix 1 — byte-unsafe `truncate()` (`src/cli/mod.rs`)

**Previous behaviour:** `&s[..max]` indexed a UTF-8 string at a raw byte offset — would panic if `max` fell inside a multibyte codepoint (e.g. `─` = 3 bytes, `…` = 3 bytes).

**New behaviour:** Uses `s.char_indices().nth(max)` to find the correct byte boundary before slicing. Count check also switched from `s.len()` (bytes) to `s.chars().count()` (characters).

**Rollback:** Restore `if s.len() <= max` and `&s[..max]`.

---

### Fix 2 — `TOOL_RESULT_MAX_CHARS` 8 192 → 32 768 (`src/server/api/messages.rs`)

**Previous behaviour:** Tool results were truncated at 8 192 chars (~2.7k tokens) when building LLM context. This cut off legitimate outputs — large `git diff`, file reads, search results — losing context the LLM needed.

**New behaviour:** Cap raised to 32 768 chars (~10k tokens). Still bounds runaway outputs (raw images, massive logs) while giving the LLM enough content for complex tool outputs.

**Rollback:** Change `TOOL_RESULT_MAX_CHARS` back to `8_192`.

---

### Fix 3 — per-tool execution timeout (`src/cli/repl.rs`)

**Previous behaviour:** `dispatch()` had no timeout. A stalled bash command or unresponsive MCP server would block the entire turn indefinitely.

**New behaviour:** `execute_tool()` wraps `dispatch()` in `tokio::time::timeout(120s)`. On expiry, returns a `ToolResult { is_error: true, output: "Tool '…' timed out after 120s" }` and the turn continues normally.

**Rollback:** Remove the `tokio::time::timeout` wrapper and restore `let mut result = dispatch(...).await;`.

---

### Fix 4 — batch tool dispatch (`src/cli/repl.rs`)

**Previous behaviour:** When the LLM returned N tool calls in one response, they executed and sent results one-by-one, each triggering a separate LLM round-trip. For N=3 tools this meant 3 LLM calls instead of 1.

**New behaviour:** All tools execute sequentially (approval prompts preserved), results are collected, then sent to the server in rapid succession. The server's `pending_tool_results` guard holds the LLM call until every expected result has arrived — only the final send triggers the LLM. Result: 1 LLM call per batch of tool responses, N-1 fewer round-trips.

**Rollback:** Restore the original `for (call_id, tool_name, args) in tool_calls { ... stream_turn ... dispatch_tool_calls ... }` loop.

---

### Fix 5 — auto-wire tools on agent creation (`src/server/api/agents.rs`)

**Previous behaviour:** Agents created via the REST API with no tool attachment fell back to receiving ALL registered tools on every turn (backwards-compatible default in `messages.rs`). This sent unnecessary token-heavy schemas.

**New behaviour:** `create_agent()` now auto-attaches: if `tool_ids` are supplied in the request body, those are wired; otherwise all currently registered tools are attached immediately. The backwards-compatible fallback in `messages.rs` remains as protection for legacy agents.

**Rollback:** Remove the auto-wire block (the 15 lines between `sqlite::create_agent` and `// Handle memory blocks`).

---

## 2026-03-08 UTC — Fix MCP tool errors: stale schema sync + double-prefix message

**Summary:** (1) Detach all previously attached tools at every CADE startup and re-register from scratch so stale MCP tool schemas from removed/disconnected servers never reach the LLM. (2) Fix double-prefix in MCP error messages.

**Files modified:** `src/main.rs`, `src/tools/manager.rs`

**Root cause (Fix 1):** `attach_agent_tools()` uses `INSERT OR IGNORE` — it only adds entries, never removes them. MCP tools from previous sessions (removed servers, changed config) accumulated in `agent_tools` indefinitely. On the next session, `build_context()` still included stale schemas → LLM called them → `find_tool_idx()` returned `None` (server not in runtime McpManager) → `"Unknown tool: '...'"`.

**Root cause (Fix 2):** rmcp formats JSON-RPC errors as `"Mcp error: -32XXX: ..."`. The dispatch wrapper unconditionally prepended `"MCP error: "`, producing `"MCP error: Mcp error: -32XXX: ..."`.

**Fix 1 — startup tool sync (`src/main.rs`):** Before the MCP registration block, snapshot current non-MCP tool IDs (those without `__` in name), detach all, re-attach non-MCP IDs immediately, then let the MCP block re-attach only the current session's live MCP tools. MCP tools always carry a `server__tool` prefix; native and meta tools never do — so the `name.contains("__")` heuristic cleanly separates them without needing a tags endpoint.

**Fix 2 — clean error message (`src/tools/manager.rs`):** In `dispatch()` MCP `Err` arm, check if message already starts with `"Mcp error:"` / `"MCP error:"` before prepending the prefix.

**Previous behaviour:** Stale MCP tool schemas caused `"Unknown tool: 'server__tool'"` errors; protocol errors showed double-prefix `"MCP error: Mcp error: -32602: ..."`.

**New behaviour:** Every startup: non-MCP tools (native + meta) are preserved, stale MCP tools are dropped, only live MCP tools are re-attached. Protocol errors display cleanly.

**Rollback Fix 1:** Remove the `{ let non_mcp_ids ... }` sync block added before `if !mcp.is_empty()` in `main.rs`.
**Rollback Fix 2:** Restore `Some(Err(e)) => (format!("MCP error: {e}"), true),` in `manager.rs`.

---

## 2026-03-08 UTC — Fix EMPTY_YIELD_REPROMPT: DB pollution + cancel bypass

**Summary:** Two fixes to the auto-reprompt mechanism in `dispatch_tool_calls()`.

**Files modified:** `src/cli/repl.rs`, `src/agent/client.rs`, `src/server/api/messages.rs`

### Fix 1 — Re-prompt no longer persisted as user message

**Root cause:** `stream_turn(EMPTY_YIELD_REPROMPT, is_tool_return=false, ...)` sent the injection as a regular user message. The server persisted it as `role="user"` → future `build_context()` loads it from DB → synthetic "Tool execution complete..." pollutes conversation history and wastes context window on every subsequent turn.

**Fix:**
- Added `ephemeral: bool` parameter to `stream_turn()` (positioned after `tool_output`)
- Added `ephemeral: bool` to `client.stream_message_cancellable()` and `client.send_message()`
- Client includes `"ephemeral": true` in request body when set
- Both server handlers (`stream_message` SSE and `send_message` blocking) check `body["ephemeral"]` and skip `persist(...)` when true
- Re-prompt call: `stream_turn(..., EMPTY_YIELD_REPROMPT, ..., true, ...)` — ephemeral=true
- All other `stream_turn` call sites: ephemeral=false (no behaviour change)

**Rollback:** Remove `ephemeral: bool` from `stream_turn`, `stream_message_cancellable`, `send_message`; remove `if ephemeral { body["ephemeral"] = true }` from client; remove `is_ephemeral` guards from both server handlers.

---

### Fix 2 — Cancel during Phase 2 no longer triggers re-prompt

**Root cause:** If Esc/Ctrl+C fired during Phase 2 (tool result sending), `stream_turn` returned `vec![]` (cancelled). `dispatch_tool_calls` received empty messages, evaluated the re-prompt condition as true, cleared `cancel_turn`, and sent an LLM call despite user intent to cancel.

**Fix:** Added `cancel_turn` check at the very top of `dispatch_tool_calls()` before any condition evaluation. If `cancel_turn` is already set when entering, return immediately.

**Previous behaviour:** Cancel during Phase 2 → re-prompt fires → LLM call sent.
**New behaviour:** Cancel during Phase 2 → `dispatch_tool_calls` returns `Ok(())` immediately, turn ends cleanly.

**Rollback:** Remove the 5-line `cancel_turn` check block at the top of `dispatch_tool_calls()`.

---

## 2026-03-08 UTC — Ctrl+C cancels running agent turn

**Summary:** Added `(KeyCode::Char('c'), KeyModifiers::CONTROL)` arm to the tick task's key event match in the TUI event loop so Ctrl+C unconditionally cancels an in-progress LLM turn.

**Files modified:** `src/cli/repl.rs`

**Root cause:** The tick task's match block during a running turn had arms for Enter (steering), Esc (conditional cancel), and character input, but no arm for Ctrl+C. The key fell through to `_ => {}` and was silently dropped. The `app.rs:1083` handler (clear input, return empty string) was never reached because the tick task intercepts events before forwarding to the app.

**Previous behaviour:** Ctrl+C during a running turn was silently discarded. Only Esc (with empty input and ≥200ms elapsed) could cancel a turn.

**New behaviour:** Ctrl+C during a running turn clears typed input and sets `cancel_turn=true` → `stream_turn()` sees `__cancelled__` error → "Turn interrupted" is shown. Same 200ms grace period as Esc prevents stale Ctrl+C from a modal confirm from cancelling the next turn. Outside a turn, Ctrl+C still clears the input buffer (unchanged behaviour via `app.rs:1083`).

**Rollback:** Remove the `(KeyCode::Char('c'), KeyModifiers::CONTROL)` arm added after the Esc handler in the tick task's match block in `src/cli/repl.rs`.

---

## 2026-03-08 UTC — Queue multiple messages during agent turn

**Summary:** Plain Enter during a running turn now queues messages as follow-ups instead of cancelling. Multiple messages can be queued (VecDeque). Visual badge shows queue depth.

**Files modified:** `src/cli/repl.rs`, `src/ui/app.rs`

**Previous behaviour:**
- Plain Enter during turn: cancelled the turn + ran new message ("steering")
- Alt/Shift+Enter: queued ONE follow-up (Option<String>)
- Queue was single-slot; second message overwrote first

**New behaviour:**
- Plain Enter during turn: queues as follow-up (no cancel) — messages run in order
- Ctrl+Enter: steering — cancels turn + redirects immediately
- Alt/Shift+Enter: also queues as follow-up (same as plain Enter now)
- Queue is VecDeque<String> — unlimited depth, FIFO
- Status bar shows `· N queued` badge while messages are waiting
- Input placeholder shows `N queued — type another or Ctrl+Enter to redirect`

**Changes:**
- `queued_followup` type: `Arc<Mutex<Option<String>>>` → `Arc<Mutex<VecDeque<String>>>`
- Tick task Enter arms: 2 → 3 (Ctrl=steering, None=queue, Alt/Shift=queue)
- Post-turn drain: `.take()` → `.pop_front()`; updates `app.queued_count`
- `TuiApp.queued_count: usize` field; threaded through `render_frame`
- Status badge and placeholder added to `render_frame`

**Rollback:** Restore field type to `Option<String>`, restore 2-arm Enter match, restore `.take()` drain, remove `queued_count` from TuiApp + render_frame.
