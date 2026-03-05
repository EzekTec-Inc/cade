# Spec: Refactor viewport for accurate Markdown and Table rendering

## Goal
Ensure the TUI viewport accurately renders Markdown contents (headers, bullets, horizontal rules, code fences, inline styling) and tables, matching the expected visual quality for a modern CLI.

## Requirements
1.  **Markdown Parsing**: Use a consistent Markdown parsing logic for both user and assistant messages.
2.  **Table Support**: Implement logic to detect and render Markdown tables with aligned columns and proper borders/colors.
3.  **Visual Consistency**: Use soft colors and clear spacing for a professional look.

## Implementation Details
-   `src/ui/markdown.rs`: Update `parse_markdown_lines` to handle table buffering and rendering.
-   `src/ui/app.rs`: Delegate `AssistantText` and `UserMessage` rendering to `parse_markdown_lines`.
