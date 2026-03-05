# Plan: Refactor viewport for accurate Markdown and Table rendering

## Phase 1: Investigation & Scaffolding
- [x] Task: Identify the current state of viewport rendering.
- [x] Task: Locate the Markdown parser (`src/ui/markdown.rs`).

## Phase 2: Implementation
- [x] Task: Enhance `src/ui/markdown.rs` to support Markdown tables.
- [x] Task: Update `src/ui/app.rs` to use `parse_markdown_lines` for `AssistantText` and `UserMessage`.

## Phase 3: Verification & Cleanup
- [x] Task: Verify the changes with `cargo check`.
- [x] Task: Ensure the UI correctly renders tables and markdown formatting as requested.
- [x] Task: Perform manual verification by running the app and inspecting agent responses.
