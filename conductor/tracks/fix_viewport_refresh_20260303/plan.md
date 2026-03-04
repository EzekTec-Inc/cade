# Implementation Plan: Fix viewport refresh issue

## Phase 1: Bugfix
- [x] Task: Investigate the root cause of the UI refresh failure.
    - [x] Analyze `src/cli/repl.rs` and `src/ui/app.rs`.
    - [x] Trace the execution flow from `handle_ask_user_question` to `stream_turn`.
- [x] Task: Implement a fix to force a UI redraw.
    - [x] Modify `handle_ask_user_question` in `src/cli/repl.rs` to explicitly call `app.draw()` after pushing the `ToolResult`.

## Phase 2: Verification
- [ ] Task: Manually verify the fix by reproducing the bug.
- [ ] Task: Run `cargo test` to ensure no regressions.

## Phase 3: Documentation
- [ ] Task: Add a comment to the code explaining the need for the explicit `draw()` call.
