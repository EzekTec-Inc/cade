//! Component trait — the building block of the CADE TUI.
//!
//! Every visual element that participates in the render cycle implements
//! [`Component`].  The trait mirrors the `pi-tui` design:
//!
//! - **`render(width)`** returns lines that fit within `width` columns.
//! - **`handle_input(key)`** processes keyboard events; returns `true`
//!    if the event was consumed.
//! - **`is_dirty()`** signals whether the component needs a redraw.
//!
//! Components are composed by embedding them in container structs (like
//! `TuiApp`).  The TUI host calls `render` top-down, passing the
//! available width, and routes key events to the focused component via
//! `handle_input`.

use crossterm::event::KeyEvent;

/// A single rendered line returned by [`Component::render`].
///
/// Each line **must not exceed `width`** columns (visible characters,
/// ignoring ANSI escape sequences).  The caller is responsible for
/// word-wrapping or truncating.
pub type RenderedLine = String;

/// Core trait for all TUI components.
///
/// Implementations must guarantee that every string returned by
/// `render()` fits within the requested `width`.
pub trait Component {
    /// Render the component into a list of terminal lines.
    ///
    /// `width` is the maximum number of visible columns available.
    /// Each returned string must not exceed this width.
    fn render(&self, width: u16) -> Vec<RenderedLine>;

    /// Process a key event while this component has focus.
    ///
    /// Returns `true` if the event was consumed (callers should not
    /// propagate it further).  Returns `false` if the event is
    /// unhandled and should bubble up.
    fn handle_input(&mut self, _key: KeyEvent) -> bool {
        false
    }

    /// Returns `true` when the component's visual state has changed
    /// since the last `render()` call and a redraw is needed.
    ///
    /// The default implementation always returns `true` (stateless —
    /// re-render every frame).  Components that cache their output
    /// should track a dirty flag and return `false` when unchanged.
    fn is_dirty(&self) -> bool {
        true
    }
}
