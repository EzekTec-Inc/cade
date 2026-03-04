# Specification: Fix viewport refresh issue after question modal

## Overview
This track addresses a bug where the REPL viewport does not update to display the results of a tool call that is executed immediately after a user answers a question from the `ask_user_question` tool.

## Objectives
1.  Ensure that the UI is consistently redrawn after any user interaction that triggers a state change, including answering a question from a modal.
2.  Prevent race conditions or state mismatches between the UI thread and the agent's execution logic.

## Requirements
-   The fix must be applied to `src/cli/repl.rs`.
-   The solution should force a redraw of the `TuiApp` after the `QuestionWidget` returns and the answer has been processed.
-   The fix should not introduce any new UI flickering or performance regressions.