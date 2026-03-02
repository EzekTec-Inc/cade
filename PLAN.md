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
