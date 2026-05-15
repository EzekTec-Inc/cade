//! Blocking password modal for the askpass integration.
//!
//! When `sudo -A` / `ssh` / `git` trigger the askpass binary, the IPC
//! server calls back into the TUI which opens this modal.  The user
//! types a password (characters masked with `*`) and presses Enter to
//! submit or Esc to cancel.

use crossterm::event::{self, Event, KeyCode, KeyModifiers};

use crate::Result;
use crate::colors::ThemeColorsExt;

use super::{RenderLine, TuiApp};
use crate::colors::ThemeColors;
use crate::overlay_component::{OverlayComponent, OverlayInputResult};
use ratatui::Frame;
use ratatui::layout::Rect;
use std::any::Any;

/// State for the active password prompt overlay.
pub struct PasswordPromptState {
    /// The prompt text shown to the user (e.g. "[sudo] password for alice:").
    pub prompt: String,
    /// The password being typed (never displayed in cleartext).
    pub input: String,
    /// Result when finished (used by blocking variant).
    pub result: Option<Option<String>>,
    /// Oneshot sender for the async variant — sends the password (or None)
    /// back to the caller without blocking or holding a lock.
    pub async_tx: Option<tokio::sync::oneshot::Sender<Option<String>>>,
}

impl OverlayComponent for PasswordPromptState {
    fn id(&self) -> &'static str {
        "password"
    }

    fn render_overlay(&mut self, frame: &mut Frame, _area: Rect, colors: &ThemeColors) {
        use ratatui::layout::{Constraint, Layout};
        use ratatui::style::{Modifier, Style};
        use ratatui::text::{Line, Span};
        use ratatui::widgets::{Block, Borders, Clear, Paragraph};
        let area = frame.area();
        let popup_w = 50u16.min(area.width.saturating_sub(4));
        let popup_h = 5u16;
        let x = area.x + (area.width.saturating_sub(popup_w)) / 2;
        let y = area.y + (area.height.saturating_sub(popup_h)) / 2;
        let popup_area = Rect::new(x, y, popup_w, popup_h);

        // Dim backdrop behind password popup
        crate::app::layout::helpers::render_backdrop(frame, area, colors);

        frame.render_widget(Clear, popup_area);
        let block = Block::default()
            .title(" 🔒 Password ")
            .title_style(
                Style::default()
                    .fg(colors.c_primary())
                    .add_modifier(Modifier::BOLD),
            )
            .borders(Borders::ALL)
            .border_type(colors.c_border_style())
            .border_style(Style::default().fg(colors.c_primary()))
            .style(
                Style::default()
                    .bg(colors.c_bg_surface1())
                    .fg(colors.c_text_primary()),
            );
        let inner = block.inner(popup_area);
        frame.render_widget(block, popup_area);
        let prompt_line = Line::from(Span::styled(
            &self.prompt,
            Style::default()
                .fg(colors.c_text_primary())
                .add_modifier(Modifier::BOLD),
        ));
        let mask: String = "*".repeat(self.input.len());
        let input_line = Line::from(vec![
            Span::raw("> "),
            Span::styled(mask, Style::default().fg(colors.c_primary())),
            Span::raw("█"),
        ]);
        let rows = Layout::vertical([Constraint::Length(1), Constraint::Length(1)]).split(inner);
        frame.render_widget(Paragraph::new(prompt_line), rows[0]);
        frame.render_widget(Paragraph::new(input_line), rows[1]);
    }

    fn handle_input(&mut self, key: crossterm::event::KeyEvent) -> OverlayInputResult {
        match (key.code, key.modifiers) {
            (KeyCode::Enter, _) => {
                self.result = Some(Some(self.input.clone()));
                OverlayInputResult::Dismiss
            }
            (KeyCode::Esc, _) | (KeyCode::Char('c'), KeyModifiers::CONTROL) => {
                self.result = Some(None);
                OverlayInputResult::Dismiss
            }
            (KeyCode::Backspace, _) => {
                self.input.pop();
                OverlayInputResult::Consumed
            }
            (KeyCode::Char('u'), KeyModifiers::CONTROL) => {
                self.input.clear();
                OverlayInputResult::Consumed
            }
            (KeyCode::Char(c), m) if m == KeyModifiers::NONE || m == KeyModifiers::SHIFT => {
                self.input.push(c);
                OverlayInputResult::Consumed
            }
            _ => OverlayInputResult::NotHandled,
        }
    }

    fn take_result(&mut self) -> Option<Box<dyn Any>> {
        let result = self.result.take();
        // If the async channel is active, send the result through it.
        if let Some(tx) = self.async_tx.take() {
            let _ = tx.send(result.clone().flatten());
        }
        result.map(|r| Box::new(r) as Box<dyn Any>)
    }
}

impl TuiApp {
    /// Blocking password modal.  Reads terminal events directly
    /// (same pattern as `ask_question`).
    ///
    /// Returns `Some(password)` on Enter, `None` on Esc.
    pub fn ask_password(&mut self, prompt: &str) -> Result<Option<String>> {
        let state = PasswordPromptState {
            prompt: prompt.to_string(),
            input: String::new(),
            result: None,
            async_tx: None,
        };

        self.overlays.push(Box::new(state));
        self.draw()?;

        let answer = loop {
            if let Event::Key(key) = event::read()? {
                if let Some(top) = self.overlays.last_mut()
                    && top.id() == "password"
                {
                    let res = top.handle_input(key);
                    if matches!(res, OverlayInputResult::Dismiss) {
                        let Some(mut pop) = self.overlays.pop() else {
                            continue;
                        };
                        let result = pop
                            .take_result()
                            .and_then(|any| any.downcast::<Option<String>>().ok().map(|b| *b))
                            .flatten();
                        break result;
                    }
                }
                self.draw()?;
            }
        };

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
        let state = PasswordPromptState {
            prompt: prompt.to_string(),
            input: String::new(),
            result: None,
            async_tx: None,
        };

        self.overlays.push(Box::new(state));
        self.draw()?;

        let answer = loop {
            let Ok(key) = key_rx.recv() else {
                break None; // channel closed
            };
            if let Some(top) = self.overlays.last_mut()
                && top.id() == "password"
            {
                let res = top.handle_input(key);
                if matches!(res, OverlayInputResult::Dismiss) {
                    let Some(mut pop) = self.overlays.pop() else {
                        continue;
                    };
                    let result = pop
                        .take_result()
                        .and_then(|any| any.downcast::<Option<String>>().ok().map(|b| *b))
                        .flatten();
                    break result;
                }
            }
            self.draw()?;
        };

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

    /// Non-blocking password modal.  Pushes the overlay and returns a
    /// `oneshot::Receiver` that resolves when the user submits or cancels.
    ///
    /// The TUI event loop handles key events for the overlay — no lock is
    /// held by the caller while waiting.  This eliminates the deadlock risk
    /// of the `spawn_blocking` + `parking_lot::Mutex` pattern.
    pub fn ask_password_async(
        &mut self,
        prompt: &str,
    ) -> Result<tokio::sync::oneshot::Receiver<Option<String>>> {
        let (tx, rx) = tokio::sync::oneshot::channel();
        let state = PasswordPromptState {
            prompt: prompt.to_string(),
            input: String::new(),
            result: None,
            async_tx: Some(tx),
        };
        self.overlays.push(Box::new(state));
        self.draw()?;
        Ok(rx)
    }
}
