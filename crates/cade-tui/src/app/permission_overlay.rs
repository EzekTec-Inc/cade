//! Permission request overlay — an interactive modal that prompts the
//! user to approve or deny a mid-execution tool permission request.
//!
//! When a running tool calls `ask_permission`, the execution future is
//! halted and this overlay is pushed onto the TUI stack.  The user sees
//! a floating modal with the permission description and can approve (Y),
//! deny (N), or always-allow-for-session (A).

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::{
    Frame,
    layout::{Constraint, Layout, Rect},
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Clear, Paragraph, Wrap},
};

use crate::colors::ThemeColors;
use crate::colors::ThemeColorsExt;
use crate::overlay_component::{OverlayComponent, OverlayInputResult};

/// Result produced by the permission overlay.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PermissionVerdict {
    /// User explicitly approved this specific operation.
    Allow,
    /// User denied this specific operation.
    Deny,
    /// User approved and requested to remember the choice for this session.
    AllowSession,
}

/// State for the floating permission request modal.
#[derive(Debug)]
pub struct PermissionOverlay {
    /// The permission being requested (e.g. "bash.exec", "file.write").
    pub permission: String,
    /// The specific target pattern (e.g. a file path or command).
    pub pattern: String,
    /// One-shot channel to send the verdict back to the waiting tool.
    pub tx: Option<tokio::sync::oneshot::Sender<PermissionVerdict>>,
    /// Stored verdict after user input.
    verdict: Option<PermissionVerdict>,
}

impl PermissionOverlay {
    /// Create a new permission overlay for the given operation.
    pub fn new(
        permission: impl Into<String>,
        pattern: impl Into<String>,
        tx: tokio::sync::oneshot::Sender<PermissionVerdict>,
    ) -> Self {
        Self {
            permission: permission.into(),
            pattern: pattern.into(),
            tx: Some(tx),
            verdict: None,
        }
    }

    fn permission_label(&self) -> String {
        match self.permission.as_str() {
            "bash.exec" => "Shell Execution".to_string(),
            "file.write" => "Write File".to_string(),
            "file.edit" => "Edit File".to_string(),
            "file.delete" => "Delete File".to_string(),
            "network.connect" => "Network Access".to_string(),
            "desktop.screenshot" => "Desktop Screenshot".to_string(),
            "clipboard.read" => "Clipboard Read".to_string(),
            other => other.to_string(),
        }
    }
}

impl OverlayComponent for PermissionOverlay {
    fn id(&self) -> &'static str {
        "permission"
    }

    fn render_overlay(&mut self, frame: &mut Frame, area: Rect, colors: &ThemeColors) {
        // Dim background
        frame.render_widget(Clear, area);

        // Calculate centered modal area (60% width, auto height)
        let modal_w = (area.width as f64 * 0.6) as u16;
        let modal_h = 9u16;
        let x = area.x + (area.width.saturating_sub(modal_w)) / 2;
        let y = area.y + area.height.saturating_sub(modal_h) / 2;
        let modal_area = Rect::new(x, y, modal_w, modal_h);

        // Draw the modal shell
        let block = Block::default()
            .borders(Borders::ALL)
            .border_type(colors.c_border_style())
            .style(Style::default().bg(colors.c_bg_surface2()))
            .border_style(colors.border_accent())
            .title(Line::from(vec![
                Span::raw(" "),
                Span::styled(
                    "🔒 Permission Request",
                    Style::default()
                        .fg(colors.c_warning())
                        .add_modifier(Modifier::BOLD),
                ),
                Span::raw(" "),
            ]));

        let inner = block.inner(modal_area);
        frame.render_widget(Clear, modal_area);
        frame.render_widget(block, modal_area);

        // Layout inner content
        let [body_area, hint_area] =
            Layout::vertical([Constraint::Min(1), Constraint::Length(1)]).areas(inner);

        // Body text
        let body_text = vec![
            Line::from(vec![
                Span::styled(
                    "Permission: ",
                    Style::default()
                        .fg(colors.c_primary())
                        .add_modifier(Modifier::BOLD),
                ),
                Span::styled(
                    self.permission_label(),
                    Style::default().fg(colors.c_text_primary()),
                ),
            ]),
            Line::from(vec![
                Span::styled(
                    "Target:     ",
                    Style::default()
                        .fg(colors.c_primary())
                        .add_modifier(Modifier::BOLD),
                ),
                Span::styled(
                    &self.pattern,
                    Style::default()
                        .fg(colors.c_warning())
                        .add_modifier(Modifier::DIM),
                ),
            ]),
            Line::from(Span::raw("")),
            Line::from(vec![Span::styled(
                "Allow this operation?",
                Style::default().fg(colors.c_text_primary()),
            )]),
        ];

        let body = Paragraph::new(body_text)
            .block(Block::default().style(Style::default()))
            .wrap(Wrap { trim: false });
        frame.render_widget(body, body_area);

        // Hint row
        let hint_text = Line::from(vec![
            Span::styled(" [Y]es  ", Style::default().fg(colors.c_success())),
            Span::styled("[N]o  ", Style::default().fg(colors.c_error())),
            Span::styled(
                "[A]lways for session  ",
                Style::default()
                    .fg(colors.c_primary())
                    .add_modifier(Modifier::DIM),
            ),
            Span::styled("[Esc] Cancel", colors.text_muted()),
        ]);
        frame.render_widget(Paragraph::new(hint_text).style(Style::default()), hint_area);
    }

    fn handle_input(&mut self, key: KeyEvent) -> OverlayInputResult {
        match (key.code, key.modifiers) {
            (KeyCode::Char('y') | KeyCode::Char('Y'), _) => {
                self.verdict = Some(PermissionVerdict::Allow);
                if let Some(tx) = self.tx.take() {
                    let _ = tx.send(PermissionVerdict::Allow);
                }
                OverlayInputResult::Dismiss
            }
            (KeyCode::Char('n') | KeyCode::Char('N'), _) | (KeyCode::Esc, _) => {
                self.verdict = Some(PermissionVerdict::Deny);
                if let Some(tx) = self.tx.take() {
                    let _ = tx.send(PermissionVerdict::Deny);
                }
                OverlayInputResult::Dismiss
            }
            (KeyCode::Char('a') | KeyCode::Char('A'), _) => {
                self.verdict = Some(PermissionVerdict::AllowSession);
                if let Some(tx) = self.tx.take() {
                    let _ = tx.send(PermissionVerdict::AllowSession);
                }
                OverlayInputResult::Dismiss
            }
            (KeyCode::Char('c'), KeyModifiers::CONTROL) => {
                self.verdict = Some(PermissionVerdict::Deny);
                if let Some(tx) = self.tx.take() {
                    let _ = tx.send(PermissionVerdict::Deny);
                }
                OverlayInputResult::Dismiss
            }
            _ => OverlayInputResult::Consumed,
        }
    }

    fn take_result(&mut self) -> Option<Box<dyn std::any::Any>> {
        self.verdict
            .take()
            .map(|v| Box::new(v) as Box<dyn std::any::Any>)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crossterm::event::{KeyEvent, KeyModifiers};

    fn make_key(code: KeyCode) -> KeyEvent {
        KeyEvent::new(code, KeyModifiers::NONE)
    }

    #[test]
    fn permission_overlay_approve() {
        let (tx, _rx) = tokio::sync::oneshot::channel();
        let mut overlay = PermissionOverlay::new("file.write", "/tmp/test.txt", tx);

        let result = overlay.handle_input(make_key(KeyCode::Char('y')));
        assert_eq!(result, OverlayInputResult::Dismiss);
        assert_eq!(overlay.verdict, Some(PermissionVerdict::Allow));
    }

    #[test]
    fn permission_overlay_deny() {
        let (tx, _rx) = tokio::sync::oneshot::channel();
        let mut overlay = PermissionOverlay::new("bash.exec", "rm -rf /", tx);

        let result = overlay.handle_input(make_key(KeyCode::Char('n')));
        assert_eq!(result, OverlayInputResult::Dismiss);
        assert_eq!(overlay.verdict, Some(PermissionVerdict::Deny));
    }

    #[test]
    fn permission_overlay_allow_session() {
        let (tx, _rx) = tokio::sync::oneshot::channel();
        let mut overlay = PermissionOverlay::new("network.connect", "example.com:443", tx);

        let result = overlay.handle_input(make_key(KeyCode::Char('A')));
        assert_eq!(result, OverlayInputResult::Dismiss);
        assert_eq!(overlay.verdict, Some(PermissionVerdict::AllowSession));
    }

    #[test]
    fn permission_overlay_esc_denies() {
        let (tx, _rx) = tokio::sync::oneshot::channel();
        let mut overlay = PermissionOverlay::new("file.delete", "/data/db", tx);

        let result = overlay.handle_input(make_key(KeyCode::Esc));
        assert_eq!(result, OverlayInputResult::Dismiss);
        assert_eq!(overlay.verdict, Some(PermissionVerdict::Deny));
    }
}
