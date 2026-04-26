# CADE TUI Modernization & Refactor Plan

This document outlines a structural refactor for CADE's TUI (`crates/cade-tui`), taking inspiration from the architecture of `@mariozechner/pi-tui` (the UI framework powering `pi-coding-agent`). 

The goal is to move CADE from a rigid, monolithic, state-driven rendering loop into a more extensible, component-based architecture capable of supporting true Input Method Editor (IME) synchronization, pluggable editors (e.g., Vim mode), and dynamic overlays without polluting the core state machine.

---

## 1. True IME Support via Hardware Cursor Sync

### Current State
CADE currently uses `tui-textarea` for input. While `tui-textarea` provides a visual cursor (usually via reversed colors), it does not explicitly synchronize the terminal's **hardware cursor**. This breaks Input Method Editors (IMEs) for languages like Chinese, Japanese, and Korean, as the OS candidate window spawns in the wrong location on the screen.

### Proposed Architecture
`pi` solves this by defining a `Focusable` interface. When a component has focus, the TUI looks for its cursor coordinates and moves the physical hardware cursor to that exact spot.

**Implementation Steps:**
1. In `crates/cade-tui/src/app/render.rs`, after rendering the `tui-textarea` (or new custom Editor), extract its cursor coordinates (`x, y`) relative to the viewport.
2. In `crates/cade-cli/src/cli/repl.rs` or inside the `draw()` loop of `TuiApp`, immediately after `frame.render_widget(...)` or `terminal.draw(...)`, manually flush a `crossterm::cursor::MoveTo(x, y)` and `crossterm::cursor::Show` command.
3. If no input element is focused, emit `crossterm::cursor::Hide`.

---

## 2. Dynamic Overlay Stack Abstraction

### Current State
`TuiApp` explicitly tracks every potential overlay as an `Option<State>`.
```rust
pub picker: Option<PickerState>,
pub theme_picker: Option<ThemePickerState>,
pub command_palette: Option<CommandPaletteState>,
pub active_question: Option<ActiveQuestionState>,
```
This forces `render_frame()` and `handle_key_input()` to contain massive `if let Some(...)` blocks. Adding a new feature (like an MCP context picker) requires modifying the core app loop and layout engine.

### Proposed Architecture
Adopt a stack-based `OverlayManager`. `pi` achieves this via `ctx.ui.custom(component, { overlay: true })`.

**Implementation Steps:**
1. Expand the existing `Component` trait in `crates/cade-tui/src/component.rs` to allow rendering on top of an existing frame.
   ```rust
   pub trait OverlayComponent {
       fn render_overlay(&self, frame: &mut Frame, area: Rect, colors: &ThemeColors);
       fn handle_input(&mut self, key: KeyEvent) -> bool;
       fn is_dismissed(&self) -> bool;
   }
   ```
2. Add an overlay stack to `TuiApp`:
   ```rust
   pub overlays: Vec<Box<dyn OverlayComponent>>,
   ```
3. Update the render loop:
   * First, render the base UI (Timeline, Footer, Input).
   * Then, iterate through `self.overlays` and call `render_overlay()`.
4. Update the input loop:
   * When a key event occurs, pass it to `self.overlays.last_mut()`.
   * If the overlay consumes the event (`returns true`), stop. 
   * If the overlay was dismissed, pop it from the stack.

---

## 3. Pluggable Input Editor Interface

### Current State
CADE tightly couples its event loop to `tui_textarea::TextArea`. If a user wants Vim-style navigation (Normal vs. Insert mode) or advanced custom autocomplete triggers, it requires hacking the main `handle_key_input` loop in `app/input.rs`.

### Proposed Architecture
`pi` defines a `CustomEditor` class. Extensions can swap out the default editor for a Vim-style editor that intercepts keystrokes locally and only passes the final `String` to the application on submit.

**Implementation Steps:**
1. Create an `EditorComponent` trait in CADE:
   ```rust
   pub trait EditorComponent {
       fn render(&mut self, frame: &mut Frame, area: Rect, colors: &ThemeColors);
       fn handle_input(&mut self, key: KeyEvent) -> EditorAction;
       fn text(&self) -> String;
       fn set_text(&mut self, text: String);
       fn cursor_position(&self) -> (u16, u16);
   }
   
   pub enum EditorAction {
       Consumed,
       Submit(String),
       Cancel,
       Unhandled(KeyEvent),
   }
   ```
2. Wrap `tui_textarea::TextArea` into a `DefaultEditor` struct that implements this trait.
3. Update `TuiApp` to hold `Box<dyn EditorComponent>`. This paves the way for a future `VimEditor` struct that users can enable via config.

---

## 4. UI Extension Slots

### Current State
CADE allows limited extensibility through `header_lines: Vec<RenderLine>` and `footer_extra: Option<String>`.

### Proposed Architecture
`pi` provides robust API hooks (`setWidget`, `setStatus`, `setFooter`) to let plugins inject entire components into designated regions of the screen.

**Implementation Steps:**
1. Define formal "Slots" in the `ratatui` layout (e.g., `TopBar`, `RightSidebar`, `BottomWidget`, `FooterReplacement`).
2. Allow these slots to hold `Box<dyn Component>`.
3. If CADE decides to support WebAssembly (WASM) or external plugins in the future, these slots will be the primary visual entry points for those extensions.

---

## Summary of Benefits
By implementing these four architectural patterns:
1. **Accessibility:** CJK developers can use their native IMEs without visual glitches.
2. **Maintainability:** The core `render_frame` function shrinks from 500+ lines of monolithic logic to a clean, slot-based layout manager.
3. **Extensibility:** Adding new pickers, themes, or tools requires zero changes to the core event loop.
4. **Customizability:** Users gain the ability to drop in alternative input methods (like Vim mode) seamlessly.