//! Leader-Key / "Which-Key" Chord System (`Ctrl+X` Chords).
//!
//! Provides fast single-key navigation without typing full slash commands.

// region:    --- Imports

use std::time::{Duration, Instant};
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Clear, Paragraph};
use ratatui::Frame;

use crate::colors::{ThemeColors, ThemeColorsExt};

// endregion: --- Imports

// region:    --- Types

/// Actions triggered by completing a leader chord sequence.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LeaderAction {
    /// Open the LLM model picker ('m').
    ModelPicker,
    /// Open the session / branch picker ('s').
    SessionPicker,
    /// Cycle or open the theme selector ('t').
    ThemePicker,
    /// Undo the last checkpoint ('u').
    UndoCheckpoint,
    /// Redo the last reverted checkpoint ('r').
    RedoCheckpoint,
    /// Toggle permission mode ('p').
    TogglePermissions,
    /// Show the help overlay ('?').
    HelpOverlay,
}

/// Result of processing a key event while in leader chord mode.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LeaderOutcome {
    /// Leader key was just pressed, waiting for second key.
    Pending,
    /// Action completed.
    Action(LeaderAction),
    /// Chord cancelled via Esc or unmapped key.
    Dismissed,
}

// endregion: --- Types

// region:    --- LeaderKeyEngine

/// Deep state machine managing leader chord sequence and floating hint UI.
#[derive(Debug, Clone)]
pub struct LeaderKeyEngine {
    pub is_active: bool,
    pub activated_at: Option<Instant>,
    pub timeout: Duration,
}

impl Default for LeaderKeyEngine {
    fn default() -> Self {
        Self {
            is_active: false,
            activated_at: None,
            timeout: Duration::from_secs(4),
        }
    }
}

impl LeaderKeyEngine {
    pub fn new() -> Self {
        Self::default()
    }

    /// Activate leader chord mode.
    pub fn activate(&mut self) {
        self.is_active = true;
        self.activated_at = Some(Instant::now());
    }

    /// Dismiss leader chord mode.
    pub fn dismiss(&mut self) {
        self.is_active = false;
        self.activated_at = None;
    }

    /// Process a key event when leader chord mode is active.
    pub fn handle_key(&mut self, key: KeyEvent) -> LeaderOutcome {
        if !self.is_active {
            if key.modifiers.contains(KeyModifiers::CONTROL) && key.code == KeyCode::Char('x') {
                self.activate();
                return LeaderOutcome::Pending;
            }
            return LeaderOutcome::Dismissed;
        }

        // Check timeout
        if let Some(t) = self.activated_at
            && t.elapsed() > self.timeout
        {
            self.dismiss();
            return LeaderOutcome::Dismissed;
        }

        match key.code {
            KeyCode::Esc => {
                self.dismiss();
                LeaderOutcome::Dismissed
            }
            KeyCode::Char('m') | KeyCode::Char('M') => {
                self.dismiss();
                LeaderOutcome::Action(LeaderAction::ModelPicker)
            }
            KeyCode::Char('s') | KeyCode::Char('S') => {
                self.dismiss();
                LeaderOutcome::Action(LeaderAction::SessionPicker)
            }
            KeyCode::Char('t') | KeyCode::Char('T') => {
                self.dismiss();
                LeaderOutcome::Action(LeaderAction::ThemePicker)
            }
            KeyCode::Char('u') | KeyCode::Char('U') => {
                self.dismiss();
                LeaderOutcome::Action(LeaderAction::UndoCheckpoint)
            }
            KeyCode::Char('r') | KeyCode::Char('R') => {
                self.dismiss();
                LeaderOutcome::Action(LeaderAction::RedoCheckpoint)
            }
            KeyCode::Char('p') | KeyCode::Char('P') => {
                self.dismiss();
                LeaderOutcome::Action(LeaderAction::TogglePermissions)
            }
            KeyCode::Char('?') | KeyCode::Char('h') | KeyCode::Char('H') => {
                self.dismiss();
                LeaderOutcome::Action(LeaderAction::HelpOverlay)
            }
            _ => {
                self.dismiss();
                LeaderOutcome::Dismissed
            }
        }
    }

    /// Render the floating bottom which-key hint bar.
    pub fn render_hint_bar(&self, frame: &mut Frame, area: Rect, colors: &ThemeColors) {
        if !self.is_active || area.height < 3 {
            return;
        }

        let bar_h = 3u16;
        let bar_w = area.width.saturating_sub(4).min(90);
        let bar_x = area.x + (area.width.saturating_sub(bar_w)) / 2;
        let bar_y = area.y + area.height.saturating_sub(bar_h).saturating_sub(2);

        let render_area = Rect::new(bar_x, bar_y, bar_w, bar_h);

        frame.render_widget(Clear, render_area);

        let shortcuts = [
            ("m", "Model"),
            ("s", "Session"),
            ("t", "Theme"),
            ("p", "Perms"),
            ("u", "Undo"),
            ("r", "Redo"),
            ("?", "Help"),
            ("Esc", "Close"),
        ];

        let mut spans = vec![
            Span::styled(" Ctrl+X: ", Style::default().fg(colors.c_primary()).add_modifier(Modifier::BOLD)),
        ];

        for (key, label) in shortcuts {
            spans.push(Span::styled(format!("[{key}] "), Style::default().fg(colors.c_success()).add_modifier(Modifier::BOLD)));
            spans.push(Span::styled(format!("{label}  "), Style::default().fg(colors.c_text_primary())));
        }

        let widget = Paragraph::new(Line::from(spans))
            .block(
                Block::default()
                    .borders(Borders::ALL)
                    .border_style(Style::default().fg(colors.c_primary()))
                    .title(" Quick Actions "),
            );

        frame.render_widget(widget, render_area);
    }
}

// endregion: --- LeaderKeyEngine

// region:    --- Tests

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_leader_activation_and_actions() {
        let mut engine = LeaderKeyEngine::new();
        assert!(!engine.is_active);

        // Ctrl+X activates chord mode
        let ctrl_x = KeyEvent::new(KeyCode::Char('x'), KeyModifiers::CONTROL);
        let outcome = engine.handle_key(ctrl_x);
        assert_eq!(outcome, LeaderOutcome::Pending);
        assert!(engine.is_active);

        // Press 'm' triggers ModelPicker
        let m_key = KeyEvent::new(KeyCode::Char('m'), KeyModifiers::NONE);
        let outcome = engine.handle_key(m_key);
        assert_eq!(outcome, LeaderOutcome::Action(LeaderAction::ModelPicker));
        assert!(!engine.is_active);
    }

    #[test]
    fn test_leader_dismissal_on_esc() {
        let mut engine = LeaderKeyEngine::new();
        engine.activate();

        let esc = KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE);
        let outcome = engine.handle_key(esc);
        assert_eq!(outcome, LeaderOutcome::Dismissed);
        assert!(!engine.is_active);
    }
}

// endregion: --- Tests
