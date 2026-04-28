//! [`EditorComponent`] trait — the abstraction that any input editor in
//! the CADE TUI must implement.
//!
//! ## Why
//!
//! Today [`crate::editor::Editor`] is concrete and tightly coupled to
//! [`tui_textarea::TextArea`]. To enable Vim-mode, completion-aware,
//! or remote/IDE-driven editors, the `TuiApp` event loop should depend
//! on a trait surface, not on the textarea concretely.
//!
//! This module declares the contract.  A concrete `impl
//! EditorComponent for Editor<'_>` is provided in
//! [`crate::editor`] so existing call sites continue to work; new
//! editor implementations only need to implement this trait.
//!
//! ## Contract
//!
//! - **`render`** draws into a [`ratatui::Frame`] at the given area.
//! - **`handle_input`** receives a key event and returns an
//!   [`EditorAction`] describing whether the event was consumed and
//!   what high-level intent it produced.
//! - **`text` / `set_text`** read and replace the buffer contents.
//! - **`cursor_position`** returns the absolute terminal cell of the
//!   visible cursor — used by the IME hardware-cursor sync path in
//!   `TuiApp::draw`.

use crossterm::event::KeyEvent;
use ratatui::{Frame, layout::Rect};

use crate::colors::ThemeColors;

/// High-level result of [`EditorComponent::handle_input`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EditorAction {
    /// The event was consumed and produced no high-level intent.
    /// The caller should not propagate it further.
    Consumed,
    /// The user asked to submit the current buffer (typically Enter).
    /// Carries the full text at submission time.
    Submit(String),
    /// The user cancelled the buffer (typically Esc).
    Cancel,
    /// The event was not handled by the editor and should bubble up
    /// (e.g. global shortcuts like Ctrl+L, Ctrl+P).
    Unhandled(KeyEvent),
}

/// The pluggable editor interface.
///
/// Any input editor used by `TuiApp` must implement this trait so the
/// host event loop, render path, and IME cursor sync all stay
/// editor-agnostic.
pub trait EditorComponent {
    /// Draw the editor into `area` of `frame`.
    ///
    /// Implementations should respect the active [`ThemeColors`] for
    /// background, cursor, and placeholder styling.
    fn render(&mut self, frame: &mut Frame, area: Rect, colors: &ThemeColors);

    /// Process a single key event.  See [`EditorAction`] for the
    /// possible outcomes.
    fn handle_input(&mut self, key: KeyEvent) -> EditorAction;

    /// Return the current buffer text as a single `String` with `\n`
    /// joining lines.
    fn text(&self) -> String;

    /// Replace the buffer with `text`.
    fn set_text(&mut self, text: String);

    /// Absolute terminal cell `(x, y)` of the visible cursor, or
    /// `None` if no cursor should be shown (e.g. when rendering an
    /// inline question instead of the editor).
    ///
    /// Used by `TuiApp::draw` to issue
    /// `crossterm::cursor::MoveTo(x, y); Show` after each frame so OS
    /// IMEs spawn their candidate window at the correct position.
    fn cursor_position(&self) -> Option<(u16, u16)>;
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::editor::Editor;
    use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

    /// Compile-time guard: `Editor` must implement `EditorComponent`.
    /// If this stops compiling, the trait surface drifted from the
    /// concrete editor's capabilities.
    #[test]
    fn editor_implements_editor_component() {
        fn assert_impl<T: EditorComponent>() {}
        assert_impl::<Editor<'static>>();
    }

    #[test]
    fn editor_action_variants_are_distinguishable() {
        let key = KeyEvent::new(KeyCode::Char('a'), KeyModifiers::NONE);
        assert_ne!(EditorAction::Consumed, EditorAction::Cancel);
        assert_ne!(
            EditorAction::Submit("hi".into()),
            EditorAction::Submit("bye".into())
        );
        assert_eq!(EditorAction::Unhandled(key), EditorAction::Unhandled(key));
    }

    #[test]
    fn editor_text_roundtrip_via_trait() {
        let mut e = Editor::new();
        EditorComponent::set_text(&mut e, "hello\nworld".into());
        assert_eq!(EditorComponent::text(&e), "hello\nworld");
    }

    #[test]
    fn editor_handle_input_inserts_char_via_trait() {
        let mut e = Editor::new();
        let key = KeyEvent::new(KeyCode::Char('x'), KeyModifiers::NONE);
        let action = EditorComponent::handle_input(&mut e, key);
        assert_eq!(action, EditorAction::Consumed);
        assert_eq!(EditorComponent::text(&e), "x");
    }

    #[test]
    fn editor_handle_enter_returns_submit_with_text() {
        let mut e = Editor::new();
        EditorComponent::set_text(&mut e, "ship it".into());
        let key = KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE);
        let action = EditorComponent::handle_input(&mut e, key);
        assert_eq!(action, EditorAction::Submit("ship it".into()));
    }

    #[test]
    fn editor_handle_esc_returns_cancel() {
        let mut e = Editor::new();
        EditorComponent::set_text(&mut e, "draft".into());
        let key = KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE);
        let action = EditorComponent::handle_input(&mut e, key);
        assert_eq!(action, EditorAction::Cancel);
    }

    #[test]
    fn editor_cursor_position_is_none_before_render() {
        let e = Editor::new();
        // The editor doesn't know its terminal area until it has been
        // rendered at least once.
        assert!(EditorComponent::cursor_position(&e).is_none());
    }
}
