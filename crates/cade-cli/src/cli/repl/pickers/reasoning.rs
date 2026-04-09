use crate::Result;
use super::super::Repl;

impl Repl {
    /// Interactive reasoning tier picker — full-screen on TuiApp terminal.
    /// Returns the selected reasoning tier string or None if cancelled.
    pub(crate) async fn interactive_reasoning_picker(
        &self,
        app_arc: std::sync::Arc<parking_lot::Mutex<crate::ui::TuiApp>>,
    ) -> Result<Option<String>> {
        use crossterm::event::{self, Event, KeyCode, KeyEventKind};
        use ratatui::{
            layout::{Constraint, Direction, Layout},
            style::{Color as RC, Modifier, Style},
            text::{Line, Span},
            widgets::{Block, Borders, List, ListItem, ListState},
        };

        let current_effort = self
            .reasoning_effort
            .lock()
            .clone()
            .unwrap_or_else(|| "none".to_string());

        let tiers = [
            ("none", "No explicit reasoning budget (default)"),
            ("low", "Low reasoning effort"),
            ("medium", "Medium reasoning effort"),
            ("high", "High reasoning effort"),
            ("xhigh", "Maximum reasoning effort"),
        ];

        let mut list_pos = tiers
            .iter()
            .position(|&(t, _)| t == current_effort)
            .unwrap_or(0);

        let do_draw_tier = |app_arc: &std::sync::Arc<parking_lot::Mutex<crate::ui::TuiApp>>,
                            list_pos: usize|
         -> Result<()> {
            let title = format!(
                " Reasoning Tiers  ↑↓/jk  Enter select  q cancel  [{}/{}] ",
                list_pos + 1,
                tiers.len()
            );

            let items: Vec<ListItem<'static>> = tiers
                .iter()
                .enumerate()
                .map(|(i, (tier, desc))| {
                    let is_sel = i == list_pos;
                    let is_current = *tier == current_effort;
                    let current_tag = if is_current { " ← current" } else { "" };

                    ListItem::new(Line::from(vec![
                        Span::styled(
                            if is_sel { "  ▶ " } else { "    " }.to_string(),
                            Style::default().fg(if is_sel { RC::Green } else { RC::DarkGray }),
                        ),
                        Span::styled(
                            format!("{:<10}", tier),
                            Style::default()
                                .fg(if is_sel { RC::White } else { RC::DarkGray })
                                .add_modifier(if is_sel {
                                    Modifier::BOLD
                                } else {
                                    Modifier::empty()
                                }),
                        ),
                        Span::styled(desc.to_string(), Style::default().fg(RC::DarkGray)),
                        Span::styled(current_tag, Style::default().fg(RC::Cyan)),
                    ]))
                })
                .collect();

            let list = List::new(items).block(
                Block::default()
                    .borders(Borders::ALL)
                    .title(title)
                    .border_style(Style::default().fg(RC::DarkGray)),
            );

            let mut app = app_arc.lock();
            app.terminal.draw(|f| {
                let area = f.area();
                let center = Layout::default()
                    .direction(Direction::Vertical)
                    .constraints([
                        Constraint::Min(0),
                        Constraint::Length(tiers.len() as u16 + 2),
                        Constraint::Min(0),
                    ])
                    .split(area)[1];

                let h_center = Layout::default()
                    .direction(Direction::Horizontal)
                    .constraints([
                        Constraint::Percentage(10),
                        Constraint::Percentage(80),
                        Constraint::Percentage(10),
                    ])
                    .split(center)[1];

                let mut ls = ListState::default();
                ls.select(Some(list_pos));
                f.render_stateful_widget(list, h_center, &mut ls);
            })?;
            Ok(())
        };

        do_draw_tier(&app_arc, list_pos)?;

        let result = loop {
            if !event::poll(std::time::Duration::from_millis(200))? {
                continue;
            }
            if let Ok(Event::Key(key)) = event::read() {
                if key.kind != KeyEventKind::Press {
                    continue;
                }
                match (key.code, key.modifiers) {
                    (KeyCode::Esc, _) | (KeyCode::Char('q'), _) => break None,
                    (KeyCode::Enter, _) => {
                        break Some(tiers[list_pos].0.to_string());
                    }
                    (KeyCode::Up, _) | (KeyCode::Char('k'), _) => {
                        list_pos = if list_pos == 0 {
                            tiers.len() - 1
                        } else {
                            list_pos - 1
                        };
                        let _ = do_draw_tier(&app_arc, list_pos);
                    }
                    (KeyCode::Down, _) | (KeyCode::Char('j'), _) => {
                        list_pos = (list_pos + 1) % tiers.len();
                        let _ = do_draw_tier(&app_arc, list_pos);
                    }
                    _ => {}
                }
            }
        };

        Ok(result)
    }
}
