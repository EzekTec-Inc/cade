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
