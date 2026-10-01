//! Permission decisions adapt the shared question presentation/input model.
use super::{ActiveQuestionDrawState, ActiveQuestionState};
use crate::colors::ThemeColors;
use crate::overlay_component::{OverlayComponent, OverlayInputResult};
use crate::question::{Question, QuestionAnswer, QuestionOption};
use crossterm::event::{Event, KeyCode, KeyEvent, KeyModifiers};
use ratatui::{Frame, layout::Rect};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PermissionVerdict {
    Allow,
    Deny,
    AllowSession,
}

#[derive(Debug)]
pub struct PermissionOverlay {
    pub permission: String,
    pub pattern: String,
    pub tx: Option<tokio::sync::oneshot::Sender<PermissionVerdict>>,
    verdict: Option<PermissionVerdict>,
    dialog: ActiveQuestionState,
}

impl PermissionOverlay {
    pub fn new(
        permission: impl Into<String>,
        pattern: impl Into<String>,
        tx: tokio::sync::oneshot::Sender<PermissionVerdict>,
    ) -> Self {
        let permission = permission.into();
        let pattern = pattern.into();
        let detail = if permission == "bash.exec" {
            format!("```bash\n{pattern}\n```")
        } else {
            format!("```text\n{pattern}\n```")
        };
        let question = Question {
            header: format!("Permission · {permission}"),
            text: format!("Allow this operation?\n\n{detail}"),
            options: vec![
                QuestionOption {
                    label: "Allow once".into(),
                    description: "Y · This operation only".into(),
                },
                QuestionOption {
                    label: "Deny".into(),
                    description: "N · Refuse this operation".into(),
                },
                QuestionOption {
                    label: "Allow for this session".into(),
                    description: "A · Remember for this session".into(),
                },
            ],
            multi_select: false,
            allow_other: false,
            progress: None,
        };
        Self {
            permission,
            pattern,
            tx: Some(tx),
            verdict: None,
            dialog: ActiveQuestionState {
                approval_id: None,
                draw_state: ActiveQuestionDrawState::new(question),
                tx: None,
                result: None,
            },
        }
    }

    fn finish(&mut self, result: OverlayInputResult) -> OverlayInputResult {
        if result == OverlayInputResult::Dismiss {
            // Authorization is an indexed, typed choice, never arbitrary text.
            let verdict = match self.dialog.result.take().flatten() {
                Some(QuestionAnswer::Single(_)) => match self.dialog.draw_state.cursor_pos {
                    0 => PermissionVerdict::Allow,
                    2 => PermissionVerdict::AllowSession,
                    _ => PermissionVerdict::Deny,
                },
                _ => PermissionVerdict::Deny,
            };
            if let Some(tx) = self.tx.take() {
                let _ = tx.send(verdict.clone());
            }
            self.verdict = Some(verdict);
        }
        result
    }
}

impl OverlayComponent for PermissionOverlay {
    fn id(&self) -> &'static str {
        "permission"
    }
    fn render_overlay(&mut self, frame: &mut Frame, area: Rect, colors: &ThemeColors) {
        self.dialog.render_overlay(frame, area, colors);
    }
    fn render_inline(&self, frame: &mut Frame, area: Rect, colors: &ThemeColors) {
        self.dialog.render_inline(frame, area, colors);
    }
    fn inline_height(&self, max: u16) -> u16 {
        self.dialog.inline_height(max)
    }
    fn inline_height_for(&self, area: Rect) -> u16 {
        self.dialog.inline_height_for(area)
    }
    fn inline_area(&self) -> Option<Rect> {
        self.dialog.inline_area()
    }
    fn handle_input(&mut self, key: KeyEvent) -> OverlayInputResult {
        let index = if key.modifiers == KeyModifiers::NONE || key.modifiers == KeyModifiers::SHIFT {
            match key.code {
                KeyCode::Char('y' | 'Y') => Some(0),
                KeyCode::Char('n' | 'N') => Some(1),
                KeyCode::Char('a' | 'A') => Some(2),
                _ => None,
            }
        } else {
            None
        };
        let result = if let Some(index) = index {
            if key.kind != crossterm::event::KeyEventKind::Press {
                return OverlayInputResult::Consumed;
            }
            self.dialog.draw_state.cursor_pos = index;
            self.dialog
                .handle_input(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE))
        } else {
            self.dialog.handle_input(key)
        };
        self.finish(result)
    }
    fn handle_event(&mut self, event: &Event) -> OverlayInputResult {
        if let Event::Key(key) = event {
            return self.handle_input(*key);
        }
        let result = self.dialog.handle_event(event);
        self.finish(result)
    }
    fn take_result(&mut self) -> Option<Box<dyn std::any::Any>> {
        self.verdict
            .take()
            .map(|v| Box::new(v) as Box<dyn std::any::Any>)
    }
    fn is_dismissed(&self) -> bool {
        self.tx.as_ref().is_some_and(|tx| tx.is_closed())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn candidate6_permission_keyboard_verdicts_are_typed() {
        for (key, expected) in [
            (KeyCode::Char('y'), PermissionVerdict::Allow),
            (KeyCode::Char('n'), PermissionVerdict::Deny),
            (KeyCode::Char('A'), PermissionVerdict::AllowSession),
            (KeyCode::Esc, PermissionVerdict::Deny),
            (KeyCode::Char('3'), PermissionVerdict::AllowSession),
        ] {
            let (tx, mut rx) = tokio::sync::oneshot::channel();
            let mut overlay = PermissionOverlay::new("bash.exec", "git status", tx);
            assert_eq!(
                overlay.handle_input(KeyEvent::new(key, KeyModifiers::NONE)),
                OverlayInputResult::Dismiss
            );
            assert_eq!(rx.try_recv().unwrap(), expected);
        }
    }
}
