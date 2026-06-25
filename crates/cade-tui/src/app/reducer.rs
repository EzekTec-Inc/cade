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
                // 1. OSC 52 Universal Sequence
                use base64::Engine;
                let b64 = base64::prelude::BASE64_STANDARD.encode(&text);
                print!("\x1b]52;c;{}\x07", b64);
                use std::io::Write;
                let _ = std::io::stdout().flush();

                // 2. Native clipboard fallback (arboard)
                if let Ok(mut cb) = arboard::Clipboard::new() {
                    let _ = cb.set_text(&text);
                }

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
