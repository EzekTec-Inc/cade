//! Centralized state action reducer for CADE TUI.
#![cfg_attr(test, allow(unused_imports))]

use crate::app::{TuiApp, ToastLevel};

// region:    --- Types

/// Type-safe, centralized TUI actions that mutate the workspace state.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TuiAction {
    /// Submit a message to the active agent.
    SendMessage(String),
    /// Copy a specific block of text to the OS/terminal clipboard.
    CopyBlock(String),
    /// Connect/register a custom model provider at runtime.
    ConnectProvider(String),
}

// endregion: --- Types

// region:    --- Implementations

impl TuiApp {
    /// Centralized dispatcher that processes the given TuiAction and mutates TuiApp state.
    pub fn dispatch(&mut self, action: TuiAction) {
        match action {
            TuiAction::SendMessage(text) => {
                self.editor.set_text(text);
                self.editor.expand_pastes();
                self.draw_dirty = true;
            }
            TuiAction::CopyBlock(text) => {
                crate::app::clipboard::write_to_clipboard(&text);
                self.show_toast("Content copied to clipboard", ToastLevel::Success);
                self.draw_dirty = true;
            }
            TuiAction::ConnectProvider(name) => {
                self.show_toast(format!("Connecting provider: {name}"), ToastLevel::Info);
                self.draw_dirty = true;
            }
        }
    }
}

// endregion: --- Implementations
