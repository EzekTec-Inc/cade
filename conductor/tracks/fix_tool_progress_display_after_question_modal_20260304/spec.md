# Specification: Fix: Tool run progress and results not displayed after question modal

## Problem Description
After a user makes a selection in the question modal (e.g., for tool permission approval), the system does not display the progress and results of subsequent tool runs, nor the agent's analysis response. The UI only shows the tool call header and a brief summary of the permission response, but no further updates for the actual tool execution or the agent's continuing thought process.

## Expected Behavior
When a tool is executed, its progress (if streaming) and its final result (including any error states) should be displayed in the main REPL viewport. Following a permission prompt, the actual tool's execution details and the agent's analysis should be visible to the user, providing full transparency on the agent's actions.

## Scope
- Identify the root cause in `src/cli/repl.rs` and `src/ui/app.rs` where tool results are not being pushed to the `TuiApp`'s `RenderLine` stream for display.
- Ensure that all tool results, especially those following an `ask_user_question` (or permission prompt) interaction, are correctly rendered in the `TuiApp` viewport.
- The fix should maintain consistent UI behavior and avoid reintroducing previous issues with viewport refresh or modal overlays.

## Out of Scope
- Extensive refactoring of the `QuestionWidget` or `TuiApp` beyond what is necessary to resolve the rendering issue.
- Changes to the underlying tool execution logic or server-side message handling, unless directly related to the UI rendering problem.
