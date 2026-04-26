# CADE TUI Refactor: Implementation Plan

## Phase 1: IME Support (Hardware Cursor Sync)
1. **Goal:** Synchronize terminal hardware cursor with the input editor's visual cursor.
2. **Tasks:**
   - Update `Editor` to expose absolute cursor coordinates relative to the input area.
   - Modify `TuiApp::draw` (in `crates/cade-cli/src/cli/repl.rs` or `crates/cade-tui/src/app/mod.rs`) to retrieve these coordinates.
   - After `terminal.draw(...)`, emit `crossterm::cursor::MoveTo` and `crossterm::cursor::Show`.
   - On editor blur/hide, emit `crossterm::cursor::Hide`.
3. **Verification:** Test with CJK IME to ensure candidate window spawns at the text cursor.

## Phase 2: Pluggable Editor Interface
1. **Goal:** Decouple `tui-textarea` from `TuiApp`.
2. **Tasks:**
   - Define `EditorComponent` trait (`render`, `handle_input`, `text`, `set_text`, `cursor_position`).
   - Define `EditorAction` enum for input results (Consumed, Submit, Cancel, Unhandled).
   - Wrap `tui_textarea::TextArea` in `DefaultEditor : EditorComponent`.
   - Update `TuiApp.editor` to `Box<dyn EditorComponent>`.
   - Update `app/input.rs` to route keystrokes through `EditorComponent::handle_input`.
3. **Verification:** Standard input, history navigation, and submission work identically to the current state.

## Phase 3: Overlay Stack Abstraction
1. **Goal:** Replace hardcoded `Option<State>` overlays with a dynamic stack.
2. **Tasks:**
   - Define `OverlayComponent` trait (`render_overlay`, `handle_input`, `is_dismissed`).
   - Migrate `PickerState`, `ThemePickerState`, `CommandPaletteState`, and `ActiveQuestionState` to implement `OverlayComponent`.
   - Add `pub overlays: Vec<Box<dyn OverlayComponent>>` to `TuiApp`.
   - Remove hardcoded `Option` fields from `TuiApp`.
   - Update `render_frame` to iterate and render the `overlays` stack.
   - Update `handle_key_input` to route events to `overlays.last_mut()`.
3. **Verification:** Trigger the file picker, theme picker, and command palette. Ensure they render on top and intercept keys correctly.

## Phase 4: UI Extension Slots
1. **Goal:** Allow components to be injected into designated layout regions.
2. **Tasks:**
   - Define a `SlotManager` or standard `Box<dyn Component>` fields for `header`, `footer`, and `sidebar`.
   - Update `render_frame` layout constraints to dynamically size and render active slots.
3. **Verification:** Inject a test widget into the header and footer slots and confirm rendering and dynamic layout adjustments.
