//! Navigable child-session viewport.
//!
//! Lists every tracked (background) subagent with its live status and lets
//! the user drill into a single subagent's captured transcript.  Opened from
//! the prompt editor with `F5`; closed with `Esc`.

use crossterm::event::{KeyCode, KeyEvent};
use ratatui::{
    layout::Rect,
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::{List, ListItem, Paragraph},
    Frame,
};

use crate::colors::{ThemeColors, ThemeColorsExt};
use crate::overlay::{
    overlay_selected_style, render_overlay_hint, render_overlay_shell, split_overlay_body,
};
use crate::overlay_component::{OverlayComponent, OverlayInputResult};
use crate::subagent_tracker::{SubagentStatus, SubagentTracker};

pub struct SubagentInspectorOverlay {
    trackers: Vec<SubagentTracker>,
    selected: usize,
    transcript_scroll: usize,
    viewing_transcript: bool,
    dismissed: bool,
}

impl SubagentInspectorOverlay {
    pub fn new(trackers: Vec<SubagentTracker>) -> Self {
        Self {
            trackers,
            selected: 0,
            transcript_scroll: 0,
            viewing_transcript: false,
            dismissed: false,
        }
    }
}

impl OverlayComponent for SubagentInspectorOverlay {
    fn id(&self) -> &'static str {
        "subagent_inspector"
    }

    fn render_overlay(&mut self, frame: &mut Frame, area: Rect, colors: &ThemeColors) {
        if self.viewing_transcript {
            self.render_transcript(frame, area, colors);
        } else {
            self.render_list(frame, area, colors);
        }
    }

    fn handle_input(&mut self, key: KeyEvent) -> OverlayInputResult {
        let n = self.trackers.len();
        if n == 0 {
            self.dismissed = true;
            return OverlayInputResult::Dismiss;
        }

        match (key.code, key.modifiers) {
            (KeyCode::Esc, _) => {
                if self.viewing_transcript {
                    self.viewing_transcript = false;
                    self.transcript_scroll = 0;
                } else {
                    self.dismissed = true;
                    return OverlayInputResult::Dismiss;
                }
            }
            (KeyCode::Enter, _) if !self.viewing_transcript => {
                self.viewing_transcript = true;
                self.transcript_scroll = 0;
            }
            (KeyCode::Up, _) | (KeyCode::Char('k'), _) if !self.viewing_transcript => {
                if self.selected > 0 {
                    self.selected -= 1;
                }
            }
            (KeyCode::Down, _) | (KeyCode::Char('j'), _) if !self.viewing_transcript => {
                if self.selected + 1 < n {
                    self.selected += 1;
                }
            }
            (KeyCode::Up, _) | (KeyCode::Char('k'), _) => {
                self.transcript_scroll = self.transcript_scroll.saturating_add(1);
            }
            (KeyCode::Down, _) | (KeyCode::Char('j'), _) => {
                self.transcript_scroll = self.transcript_scroll.saturating_sub(1);
            }
            (KeyCode::PageUp, _) => {
                self.transcript_scroll = self.transcript_scroll.saturating_add(20);
            }
            (KeyCode::PageDown, _) => {
                self.transcript_scroll = self.transcript_scroll.saturating_sub(20);
            }
            _ => {}
        }
        OverlayInputResult::Consumed
    }

    fn is_dismissed(&self) -> bool {
        self.dismissed
    }
}

impl SubagentInspectorOverlay {
    fn render_list(&self, frame: &mut Frame, area: Rect, colors: &ThemeColors) {
        let inner = render_overlay_shell(
            frame,
            area,
            " Subagents (Enter: view output · Esc: close) ",
            colors,
        );

        let items: Vec<ListItem> = self
            .trackers
            .iter()
            .enumerate()
            .map(|(i, t)| {
                let elapsed = t.started.elapsed().as_secs();
                let status = match &t.status {
                    SubagentStatus::Running => "running",
                    SubagentStatus::Completed { .. } => "done",
                    SubagentStatus::Failed { .. } => "failed",
                };
                let tool = t
                    .current_tool
                    .as_deref()
                    .map(|t| format!(" · {t}"))
                    .unwrap_or_default();
                let is_selected = i == self.selected;
                let style = if is_selected {
                    overlay_selected_style(colors)
                } else {
                    Style::default().fg(colors.c_text_primary())
                };
                ListItem::new(Line::from(vec![
                    Span::styled(
                        format!("[{status}] "),
                        if is_selected {
                            style
                        } else {
                            colors.text_muted()
                        },
                    ),
                    Span::styled(
                        t.mode.to_string(),
                        Style::default()
                            .add_modifier(Modifier::BOLD)
                            .fg(if is_selected {
                                colors.c_bg_base()
                            } else {
                                colors.c_primary()
                            }),
                    ),
                    Span::styled(
                        format!(
                            "  · {elapsed}s · {} tools · {} lines{tool}",
                            t.tool_calls, t.output_lines
                        ),
                        colors.text_muted(),
                    ),
                ]))
                .style(style)
            })
            .collect();

        let (body, footer) = split_overlay_body(inner, 1);
        let list = List::new(items).highlight_style(Style::default().add_modifier(Modifier::BOLD));
        frame.render_stateful_widget(
            list,
            body,
            &mut ratatui::widgets::ListState::default().with_selected(Some(self.selected)),
        );
        render_overlay_hint(
            frame,
            footer,
            &format!(
                "{n} subagents — ↑/↓ select · Enter open · Esc close",
                n = self.trackers.len()
            ),
            colors,
        );
    }

    fn render_transcript(&self, frame: &mut Frame, area: Rect, colors: &ThemeColors) {
        let t = &self.trackers[self.selected];
        let inner = render_overlay_shell(
            frame,
            area,
            &format!(" Subagent [{}] · output (Esc: back) ", t.mode),
            colors,
        );

        let (body, footer) = split_overlay_body(inner, 1);

        // Transcript is stored newest-first (see push_output); display oldest→newest.
        let lines: Vec<String> = t.transcript.iter().rev().cloned().collect();
        let content = if lines.is_empty() {
            "  (no output yet — subagent still starting)".to_string()
        } else {
            lines.join("\n")
        };

        frame.render_widget(
            Paragraph::new(content)
                .wrap(ratatui::widgets::Wrap { trim: false })
                .scroll((self.transcript_scroll as u16, 0)),
            body,
        );

        render_overlay_hint(
            frame,
            footer,
            "↑/↓ or j/k scroll · PgUp/PgDn page · Esc back",
            colors,
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

    fn key(code: KeyCode) -> KeyEvent {
        KeyEvent::new(code, KeyModifiers::NONE)
    }

    fn tracker(task_id: &str, mode: &str, lines: &[&str]) -> SubagentTracker {
        let mut t = SubagentTracker::new(task_id.into(), mode.into());
        for l in lines {
            t.push_output((*l).into());
        }
        t
    }

    #[test]
    fn esc_dismisses_from_list() {
        let mut o = SubagentInspectorOverlay::new(vec![tracker("a", "worker", &[])]);
        assert_eq!(
            o.handle_input(key(KeyCode::Esc)),
            OverlayInputResult::Dismiss
        );
        assert!(o.is_dismissed());
    }

    #[test]
    fn enter_opens_transcript_then_esc_returns() {
        let mut o = SubagentInspectorOverlay::new(vec![tracker("a", "build", &["hello", "world"])]);
        assert_eq!(
            o.handle_input(key(KeyCode::Enter)),
            OverlayInputResult::Consumed
        );
        assert!(o.viewing_transcript);
        // First Esc backs out to the list, does not dismiss.
        assert_eq!(
            o.handle_input(key(KeyCode::Esc)),
            OverlayInputResult::Consumed
        );
        assert!(!o.viewing_transcript);
        assert!(!o.is_dismissed());
        // Second Esc dismisses.
        assert_eq!(
            o.handle_input(key(KeyCode::Esc)),
            OverlayInputResult::Dismiss
        );
    }

    #[test]
    fn down_moves_selection_and_wraps_at_end() {
        let mut o = SubagentInspectorOverlay::new(vec![
            tracker("a", "worker", &[]),
            tracker("b", "build", &[]),
        ]);
        o.handle_input(key(KeyCode::Down));
        assert_eq!(o.selected, 1);
        // At the last entry, Down is a no-op.
        o.handle_input(key(KeyCode::Down));
        assert_eq!(o.selected, 1);
        o.handle_input(key(KeyCode::Up));
        assert_eq!(o.selected, 0);
    }

    #[test]
    fn empty_tracker_list_dismisses_immediately() {
        let mut o = SubagentInspectorOverlay::new(vec![]);
        assert_eq!(
            o.handle_input(key(KeyCode::Esc)),
            OverlayInputResult::Dismiss
        );
    }
}
