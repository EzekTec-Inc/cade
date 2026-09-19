use crate::colors::ThemeColorsExt;
use crate::subagent_tracker::{SubagentStatus, SubagentTracker};
use crossterm::event::{KeyCode, KeyEvent};
use ratatui::Frame;
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, List, ListItem, ListState, Paragraph, Wrap};

use serde::{Deserialize, Serialize};

/// Actions triggered from the Subagent Control Tray.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum SubagentTrayAction {
    #[default]
    None,
    Steer {
        subagent_id: String,
        message: String,
    },
    HotSwapModel {
        subagent_id: String,
        model: String,
    },
    PauseResume {
        subagent_id: String,
    },
    Kill {
        subagent_id: String,
    },
}

/// Persistent state for the dockable Subagent Control Tray in `cade-tui`.
#[derive(Debug, Clone, Default)]
pub struct SubagentTrayState {
    pub is_visible: bool,
    pub is_focused: bool,
    pub selected: usize,
    pub transcript_scroll: usize,
    pub viewing_transcript: bool,
    pub steer_input: Option<String>,
    pub model_input: Option<String>,
    pub pending_action: SubagentTrayAction,
}

impl SubagentTrayState {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn toggle_visible(&mut self) {
        self.is_visible = !self.is_visible;
        if self.is_visible {
            self.is_focused = true;
        } else {
            self.is_focused = false;
            self.steer_input = None;
            self.model_input = None;
            self.viewing_transcript = false;
        }
    }

    pub fn toggle_focus(&mut self) {
        if self.is_visible {
            self.is_focused = !self.is_focused;
        }
    }

    pub fn take_pending_action(&mut self) -> SubagentTrayAction {
        std::mem::replace(&mut self.pending_action, SubagentTrayAction::None)
    }

    pub fn handle_key(&mut self, k: KeyEvent, trackers: &[SubagentTracker]) -> bool {
        if !self.is_visible || !self.is_focused {
            return false;
        }

        // 1. If currently typing steering guidance
        if let Some(ref mut input) = self.steer_input {
            match k.code {
                KeyCode::Esc => {
                    self.steer_input = None;
                    return true;
                }
                KeyCode::Enter => {
                    let msg = input.trim().to_string();
                    self.steer_input = None;
                    if !msg.is_empty()
                        && let Some(t) = trackers.get(self.selected)
                    {
                        self.pending_action = SubagentTrayAction::Steer {
                            subagent_id: t.task_id.clone(),
                            message: msg,
                        };
                    }
                    return true;
                }
                KeyCode::Backspace => {
                    input.pop();
                    return true;
                }
                KeyCode::Char(c) => {
                    input.push(c);
                    return true;
                }
                _ => return true,
            }
        }

        // 2. If currently typing model hot-swap
        if let Some(ref mut input) = self.model_input {
            match k.code {
                KeyCode::Esc => {
                    self.model_input = None;
                    return true;
                }
                KeyCode::Enter => {
                    let m = input.trim().to_string();
                    self.model_input = None;
                    if !m.is_empty()
                        && let Some(t) = trackers.get(self.selected)
                    {
                        self.pending_action = SubagentTrayAction::HotSwapModel {
                            subagent_id: t.task_id.clone(),
                            model: m,
                        };
                    }
                    return true;
                }
                KeyCode::Backspace => {
                    input.pop();
                    return true;
                }
                KeyCode::Char(c) => {
                    input.push(c);
                    return true;
                }
                _ => return true,
            }
        }

        // 3. Normal navigation when viewing transcript
        if self.viewing_transcript {
            match k.code {
                KeyCode::Esc | KeyCode::Enter => {
                    self.viewing_transcript = false;
                    return true;
                }
                KeyCode::Up | KeyCode::Char('k') => {
                    self.transcript_scroll = self.transcript_scroll.saturating_add(1);
                    return true;
                }
                KeyCode::Down | KeyCode::Char('j') => {
                    self.transcript_scroll = self.transcript_scroll.saturating_sub(1);
                    return true;
                }
                KeyCode::PageUp => {
                    self.transcript_scroll = self.transcript_scroll.saturating_add(20);
                    return true;
                }
                KeyCode::PageDown => {
                    self.transcript_scroll = self.transcript_scroll.saturating_sub(20);
                    return true;
                }
                _ => return false,
            }
        }

        // 4. List view navigation & actions
        match k.code {
            KeyCode::Esc => {
                self.is_visible = false;
                self.is_focused = false;
                true
            }
            KeyCode::Down | KeyCode::Char('j') => {
                if !trackers.is_empty() {
                    self.selected = (self.selected + 1).min(trackers.len().saturating_sub(1));
                }
                true
            }
            KeyCode::Up | KeyCode::Char('k') => {
                self.selected = self.selected.saturating_sub(1);
                true
            }
            KeyCode::Enter => {
                if !trackers.is_empty() {
                    self.viewing_transcript = true;
                    self.transcript_scroll = 0;
                }
                true
            }
            KeyCode::Char('s') | KeyCode::Char('i') => {
                if !trackers.is_empty() {
                    self.steer_input = Some(String::new());
                }
                true
            }
            KeyCode::Char('m') => {
                if let Some(_t) = trackers.get(self.selected) {
                    self.model_input = Some("gemini/gemini-2.0-flash".to_string());
                }
                true
            }
            KeyCode::Char(' ') => {
                if let Some(t) = trackers.get(self.selected) {
                    self.pending_action = SubagentTrayAction::PauseResume {
                        subagent_id: t.task_id.clone(),
                    };
                }
                true
            }
            KeyCode::Char('x') => {
                if let Some(t) = trackers.get(self.selected) {
                    self.pending_action = SubagentTrayAction::Kill {
                        subagent_id: t.task_id.clone(),
                    };
                }
                true
            }
            _ => false,
        }
    }

    pub fn render(
        &self,
        frame: &mut Frame,
        area: Rect,
        trackers: &[SubagentTracker],
        colors: &crate::colors::ThemeColors,
    ) {
        if !self.is_visible || area.width < 10 || area.height < 5 {
            return;
        }

        let border_style = if self.is_focused {
            colors.border_focus()
        } else {
            colors.border_muted()
        };

        let active_count = trackers
            .iter()
            .filter(|t| matches!(t.status, SubagentStatus::Running))
            .count();

        let title = if active_count > 0 {
            format!(" Subagent Control Tray · {active_count} active (Ctrl+W) ")
        } else {
            " Subagent Control Tray · Idle (Ctrl+W) ".to_string()
        };

        let block = Block::default()
            .borders(Borders::ALL)
            .title(Span::styled(
                title,
                if self.is_focused {
                    colors.primary_bold()
                } else {
                    colors.text_muted()
                },
            ))
            .border_style(border_style)
            .style(colors.style_base());

        let inner = block.inner(area);
        frame.render_widget(block, area);

        // A. Idle State (No subagents currently spawned)
        if trackers.is_empty() {
            let lines = vec![
                Line::from(vec![
                    Span::styled("○ ", colors.text_dim()),
                    Span::styled("Swarm Status: ", colors.text_muted()),
                    Span::styled("Idle", colors.primary_bold()),
                ]),
                Line::from(""),
                Line::from(Span::styled(
                    "No background subagents running.",
                    colors.text_dim(),
                )),
                Line::from(""),
                Line::from(Span::styled("Spawn workflows via:", colors.text_muted())),
                Line::from(vec![
                    Span::styled("  @worker ", colors.primary()),
                    Span::styled("<task>", colors.text_dim()),
                ]),
                Line::from(vec![
                    Span::styled("  run_subagent", colors.primary()),
                    Span::styled("(mode=\"build\")", colors.text_dim()),
                ]),
                Line::from(vec![
                    Span::styled("  subagent", colors.primary()),
                    Span::styled("(action=\"tasks\")", colors.text_dim()),
                ]),
                Line::from(""),
                Line::from(Span::styled("Hotkeys:", colors.text_muted())),
                Line::from(vec![
                    Span::styled("  F5      ", colors.primary_bold()),
                    Span::styled("toggle tray open/close", colors.text_dim()),
                ]),
                Line::from(vec![
                    Span::styled("  Ctrl+W  ", colors.primary_bold()),
                    Span::styled("switch focus editor ⇄ tray", colors.text_dim()),
                ]),
                Line::from(vec![
                    Span::styled("  Esc     ", colors.primary_bold()),
                    Span::styled("close tray", colors.text_dim()),
                ]),
            ];
            let p = Paragraph::new(lines).style(colors.style_base());
            frame.render_widget(p, inner);
            return;
        }

        // B. Transcript Drilldown View
        if self.viewing_transcript {
            if let Some(t) = trackers.get(self.selected) {
                let chunks = Layout::default()
                    .direction(Direction::Vertical)
                    .constraints([Constraint::Min(2), Constraint::Length(1)])
                    .split(inner);

                let header_line = Line::from(vec![
                    Span::styled("Task ID: ", colors.text_muted()),
                    Span::styled(&t.task_id, colors.primary_bold()),
                    Span::styled(format!(" [{}]", t.mode), colors.text_dim()),
                ]);

                let total_lines = t.transcript.len();
                let visible_height = chunks[0].height.saturating_sub(1) as usize;
                let max_scroll = total_lines.saturating_sub(visible_height);
                let effective_scroll = self.transcript_scroll.min(max_scroll);
                let start_idx = total_lines
                    .saturating_sub(visible_height)
                    .saturating_sub(effective_scroll);

                let mut transcript_lines = vec![header_line];
                for line_text in t.transcript.iter().skip(start_idx).take(visible_height) {
                    transcript_lines
                        .push(Line::from(Span::styled(line_text, colors.text_primary())));
                }

                let p = Paragraph::new(transcript_lines)
                    .style(colors.style_base())
                    .wrap(Wrap { trim: false });
                frame.render_widget(p, chunks[0]);

                let footer = Line::from(vec![
                    Span::styled("Enter/Esc: ", colors.primary_bold()),
                    Span::styled("back to list  ·  ", colors.text_dim()),
                    Span::styled("↑/↓: ", colors.primary_bold()),
                    Span::styled("scroll transcript", colors.text_dim()),
                ]);
                frame.render_widget(Paragraph::new(footer), chunks[1]);
            }
            return;
        }

        // C. Normal List View with Action Inputs
        let show_input_row = self.steer_input.is_some() || self.model_input.is_some();
        let constraints = if show_input_row {
            vec![
                Constraint::Min(3),
                Constraint::Length(3),
                Constraint::Length(1),
            ]
        } else {
            vec![Constraint::Min(3), Constraint::Length(1)]
        };

        let chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints(constraints)
            .split(inner);

        let items: Vec<ListItem> = trackers
            .iter()
            .enumerate()
            .map(|(i, t)| {
                let elapsed = t.started.elapsed().as_secs();
                let (status_str, status_style) = match &t.status {
                    SubagentStatus::Running => ("● run", colors.warning()),
                    SubagentStatus::Completed { .. } => ("✓ done", colors.success()),
                    SubagentStatus::Failed { .. } => ("✗ fail", colors.error()),
                };

                let is_selected = i == self.selected;
                let approx_tokens = (t.output_lines * 40) / 1000 + 1;

                let row1 = Line::from(vec![
                    Span::styled(format!("[{status_str}] "), status_style),
                    Span::styled(
                        &t.task_id,
                        if is_selected {
                            colors.primary_bold()
                        } else {
                            colors.text_primary_bold()
                        },
                    ),
                    Span::styled(format!(" [{}]", t.mode), colors.text_dim()),
                ]);

                let tool_str = t
                    .current_tool
                    .as_deref()
                    .map(|tool| format!(" · tool: {tool}"))
                    .unwrap_or_default();

                let row2 = Line::from(vec![Span::styled(
                    format!(
                        "    {elapsed}s · {} tools · ~{approx_tokens}k tok{tool_str}",
                        t.tool_calls
                    ),
                    colors.text_muted(),
                )]);

                ListItem::new(vec![row1, row2]).style(if is_selected {
                    colors.selected_bg_style()
                } else {
                    colors.style_base()
                })
            })
            .collect();

        let list = List::new(items);
        frame.render_stateful_widget(
            list,
            chunks[0],
            &mut ListState::default().with_selected(Some(self.selected)),
        );

        // D. Interactive Action Input Row (if active)
        if let Some(ref steer_text) = self.steer_input {
            let p_block = Block::default()
                .borders(Borders::ALL)
                .border_style(colors.border_focus())
                .title(" Steer Guidance (Enter: send · Esc: cancel) ");
            let p = Paragraph::new(format!("> {steer_text}█"))
                .block(p_block)
                .style(colors.text_primary());
            frame.render_widget(p, chunks[1]);
        } else if let Some(ref model_text) = self.model_input {
            let p_block = Block::default()
                .borders(Borders::ALL)
                .border_style(colors.warning())
                .title(" Hot-Swap Model (Enter: apply next turn · Esc: cancel) ");
            let p = Paragraph::new(format!("> {model_text}█"))
                .block(p_block)
                .style(colors.text_primary());
            frame.render_widget(p, chunks[1]);
        }

        // E. Footer Action Shortcuts
        let footer_idx = if show_input_row { 2 } else { 1 };
        let footer_spans = vec![
            Span::styled("↑/↓ ", colors.primary_bold()),
            Span::styled("sel · ", colors.text_dim()),
            Span::styled("Enter ", colors.primary_bold()),
            Span::styled("logs · ", colors.text_dim()),
            Span::styled("s ", colors.primary_bold()),
            Span::styled("steer · ", colors.text_dim()),
            Span::styled("m ", colors.primary_bold()),
            Span::styled("model · ", colors.text_dim()),
            Span::styled("Space ", colors.primary_bold()),
            Span::styled("pause · ", colors.text_dim()),
            Span::styled("x ", colors.error()),
            Span::styled("kill", colors.text_dim()),
        ];
        frame.render_widget(Paragraph::new(Line::from(footer_spans)), chunks[footer_idx]);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dummy_tracker(id: &str) -> SubagentTracker {
        SubagentTracker {
            task_id: id.to_string(),
            mode: "build".to_string(),
            status: SubagentStatus::Running,
            started: std::time::Instant::now(),
            output_lines: 10,
            tool_calls: 3,
            current_tool: Some("bash".to_string()),
            transcript: vec!["line 1".to_string(), "line 2".to_string()],
        }
    }

    #[test]
    fn test_tray_toggle_and_focus() {
        let mut state = SubagentTrayState::new();
        assert!(!state.is_visible);
        assert!(!state.is_focused);

        state.toggle_visible();
        assert!(state.is_visible);
        assert!(state.is_focused);

        state.toggle_focus();
        assert!(!state.is_focused);

        state.toggle_visible();
        assert!(!state.is_visible);
    }

    #[test]
    fn test_tray_hotkeys_and_steer_input() {
        let mut state = SubagentTrayState::new();
        state.is_visible = true;
        state.is_focused = true;

        let trackers = vec![dummy_tracker("worker-1"), dummy_tracker("worker-2")];

        // Navigate down
        state.handle_key(KeyEvent::from(KeyCode::Char('j')), &trackers);
        assert_eq!(state.selected, 1);

        // Press 's' to start steering input
        state.handle_key(KeyEvent::from(KeyCode::Char('s')), &trackers);
        assert_eq!(state.steer_input, Some(String::new()));

        // Type "focus on tests"
        for c in "focus".chars() {
            state.handle_key(KeyEvent::from(KeyCode::Char(c)), &trackers);
        }
        assert_eq!(state.steer_input, Some("focus".to_string()));

        // Press Enter to submit steering guidance
        state.handle_key(KeyEvent::from(KeyCode::Enter), &trackers);
        assert_eq!(state.steer_input, None);
        assert_eq!(
            state.take_pending_action(),
            SubagentTrayAction::Steer {
                subagent_id: "worker-2".to_string(),
                message: "focus".to_string(),
            }
        );
    }

    #[test]
    fn test_tray_hotswap_pause_kill_actions() {
        let mut state = SubagentTrayState::new();
        state.is_visible = true;
        state.is_focused = true;

        let trackers = vec![dummy_tracker("worker-1"), dummy_tracker("worker-2")];

        // Press 'x' to kill selected subagent
        state.handle_key(KeyEvent::from(KeyCode::Char('x')), &trackers);
        assert_eq!(
            state.take_pending_action(),
            SubagentTrayAction::Kill {
                subagent_id: "worker-1".to_string(),
            }
        );

        // Press Space to pause/resume
        state.handle_key(KeyEvent::from(KeyCode::Char(' ')), &trackers);
        assert_eq!(
            state.take_pending_action(),
            SubagentTrayAction::PauseResume {
                subagent_id: "worker-1".to_string(),
            }
        );

        // Press 'm' to start model hot-swap
        state.handle_key(KeyEvent::from(KeyCode::Char('m')), &trackers);
        assert!(state.model_input.is_some());
        state.handle_key(KeyEvent::from(KeyCode::Enter), &trackers);
        assert_eq!(state.model_input, None);
        assert_eq!(
            state.take_pending_action(),
            SubagentTrayAction::HotSwapModel {
                subagent_id: "worker-1".to_string(),
                model: "gemini/gemini-2.0-flash".to_string(),
            }
        );
    }

    #[test]
    fn test_subagent_tray_action_serde_roundtrip() {
        let actions = vec![
            SubagentTrayAction::None,
            SubagentTrayAction::Kill {
                subagent_id: "worker-1".to_string(),
            },
            SubagentTrayAction::PauseResume {
                subagent_id: "worker-2".to_string(),
            },
            SubagentTrayAction::Steer {
                subagent_id: "worker-3".to_string(),
                message: "look at tests".to_string(),
            },
            SubagentTrayAction::HotSwapModel {
                subagent_id: "worker-4".to_string(),
                model: "claude-3-5-sonnet".to_string(),
            },
        ];

        for action in actions {
            let json = serde_json::to_string(&action).expect("should serialize");
            let decoded: SubagentTrayAction =
                serde_json::from_str(&json).expect("should deserialize");
            assert_eq!(action, decoded);
        }
    }
}
