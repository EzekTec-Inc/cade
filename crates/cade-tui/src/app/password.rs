//! Blocking password modal for the askpass integration.
//!
//! When `sudo -A` / `ssh` / `git` trigger the askpass binary, the IPC
//! server calls back into the TUI which opens this modal.  The user
//! types a password (characters masked with `*`) and presses Enter to
//! submit or Esc to cancel.

use crossterm::event::{self, Event, KeyCode, KeyModifiers};

use crate::Result;

use super::{RenderLine, TuiApp};

/// State for the active password prompt overlay.
#[derive(Debug, Clone)]
pub struct PasswordPromptState {
    /// The prompt text shown to the user (e.g. "[sudo] password for alice:").
    pub prompt: String,
    /// The password being typed (never displayed in cleartext).
    pub input: String,
}

impl TuiApp {
    /// Blocking password modal.  Reads terminal events directly
    /// (same pattern as `ask_question`).
    ///
    /// Returns `Some(password)` on Enter, `None` on Esc.
    pub fn ask_password(&mut self, prompt: &str) -> Result<Option<String>> {
        let mut state = PasswordPromptState {
            prompt: prompt.to_string(),
            input: String::new(),
        };

        self.active_password = Some(state.clone());
        self.draw()?;

        let answer = loop {
            if let Event::Key(key) = event::read()? {
                match (key.code, key.modifiers) {
                    (KeyCode::Enter, _) => {
                        break Some(state.input.clone());
                    }
                    (KeyCode::Esc, _) => {
                        break None;
                    }
                    (KeyCode::Backspace, _) => {
                        state.input.pop();
                        self.active_password = Some(state.clone());
                        self.draw()?;
                    }
                    (KeyCode::Char('c'), KeyModifiers::CONTROL) => {
                        break None;
                    }
                    (KeyCode::Char('u'), KeyModifiers::CONTROL) => {
                        state.input.clear();
                        self.active_password = Some(state.clone());
                        self.draw()?;
                    }
                    (KeyCode::Char(c), m)
                        if m == KeyModifiers::NONE || m == KeyModifiers::SHIFT =>
                    {
                        state.input.push(c);
                        self.active_password = Some(state.clone());
                        self.draw()?;
                    }
                    _ => {}
                }
            }
        };

        self.active_password = None;

        if answer.is_some() {
            self.push(RenderLine::ToolResult {
                is_error: false,
                content: format!("🔑 Password provided for: {prompt}"),
            })?;
        } else {
            self.push(RenderLine::ToolResult {
                is_error: true,
                content: format!("❌ Password prompt cancelled: {prompt}"),
            })?;
        }

        Ok(answer)
    }

    /// Blocking password modal driven by forwarded key events (like
    /// `ask_question_blocking`).  Safe to call from `tokio::task::spawn_blocking`.
    pub fn ask_password_blocking(
        &mut self,
        prompt: &str,
        key_rx: std::sync::mpsc::Receiver<crossterm::event::KeyEvent>,
    ) -> Result<Option<String>> {
        let mut state = PasswordPromptState {
            prompt: prompt.to_string(),
            input: String::new(),
        };

        self.active_password = Some(state.clone());
        self.draw()?;

        let answer = loop {
            let Ok(key) = key_rx.recv() else {
                break None; // channel closed
            };
            match (key.code, key.modifiers) {
                (KeyCode::Enter, _) => {
                    break Some(state.input.clone());
                }
                (KeyCode::Esc, _) => {
                    break None;
                }
                (KeyCode::Backspace, _) => {
                    state.input.pop();
                    self.active_password = Some(state.clone());
                    self.draw()?;
                }
                (KeyCode::Char('c'), KeyModifiers::CONTROL) => {
                    break None;
                }
                (KeyCode::Char('u'), KeyModifiers::CONTROL) => {
                    state.input.clear();
                    self.active_password = Some(state.clone());
                    self.draw()?;
                }
                (KeyCode::Char(c), m)
                    if m == KeyModifiers::NONE || m == KeyModifiers::SHIFT =>
                {
                    state.input.push(c);
                    self.active_password = Some(state.clone());
                    self.draw()?;
                }
                _ => {}
            }
        };

        self.active_password = None;

        if answer.is_some() {
            self.push(RenderLine::ToolResult {
                is_error: false,
                content: format!("🔑 Password provided for: {prompt}"),
            })?;
        } else {
            self.push(RenderLine::ToolResult {
                is_error: true,
                content: format!("❌ Password prompt cancelled: {prompt}"),
            })?;
        }

        Ok(answer)
    }
}
