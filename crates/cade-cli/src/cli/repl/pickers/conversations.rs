use crossterm::event::KeyCode;
use crate::Result;
use super::super::Repl;

impl Repl {
    /// `/resume` conversation picker — full-screen on TuiApp terminal.
    ///
    /// Keys: ↑/↓ move · Enter select · d delete · Esc/q cancel.
    /// Returns the picked conversation JSON, or None if cancelled.
    pub(crate) async fn conversation_picker(
        &self,
        app_arc: std::sync::Arc<parking_lot::Mutex<crate::ui::TuiApp>>,
        convs: &[serde_json::Value],
        agent_id: &str,
    ) -> Result<Option<serde_json::Value>> {
        use crossterm::event::{self, Event, KeyEventKind, KeyModifiers};
        use ratatui::{
            style::{Color as RC, Modifier, Style},
            text::{Line, Span},
            widgets::{Block, Borders, List, ListItem, ListState},
        };

        if convs.is_empty() {
            return Ok(None);
        }

        let mut sel: usize = 0;
        let mut result: Option<serde_json::Value> = None;

        let build_items = |sel: usize| -> Vec<ListItem<'static>> {
            convs
                .iter()
                .enumerate()
                .map(|(i, c)| {
                    let title = c["title"].as_str().unwrap_or("(untitled)").to_string();
                    let cnt = c["message_count"].as_i64().unwrap_or(0);
                    let ts = c["updated_at"].as_i64().unwrap_or(0);
                    let date = if ts > 0 {
                        let dt = chrono::DateTime::from_timestamp(ts, 0)
                            .unwrap_or_default()
                            .with_timezone(&chrono::Local);
                        dt.format("%m/%d %H:%M").to_string()
                    } else {
                        String::new()
                    };
                    let label = format!("  {title}  ({cnt} msgs)  {date}");
                    let style = if i == sel {
                        Style::default()
                            .fg(RC::Black)
                            .bg(RC::Cyan)
                            .add_modifier(Modifier::BOLD)
                    } else {
                        Style::default().fg(RC::White)
                    };
                    ListItem::new(Line::from(vec![Span::styled(label, style)]))
                })
                .collect()
        };

        // Initial draw
        {
            let mut app = app_arc.lock();
            let items = build_items(sel);
            let n = convs.len();
            let mut ls = ListState::default().with_selected(Some(sel));
            app.terminal.draw(|f| {
                let area  = f.area();
                let block = Block::default()
                    .borders(Borders::ALL)
                    .title(format!(" Conversations [{}/{}]  ↑↓ navigate · Enter select · d delete · Esc cancel ", sel + 1, n))
                    .border_style(Style::default().fg(RC::Cyan));
                let list = List::new(items).block(block);
                f.render_stateful_widget(list, area, &mut ls);
            })?;
        }

        loop {
            if !event::poll(std::time::Duration::from_millis(200))? {
                continue;
            }
            if let Event::Key(k) = event::read()? {
                if k.kind != KeyEventKind::Press {
                    continue;
                }
                match (k.code, k.modifiers) {
                    (KeyCode::Char('q') | KeyCode::Esc, _) => break,
                    (KeyCode::Up | KeyCode::Char('k'), _) => {
                        sel = sel.saturating_sub(1);
                    }
                    (KeyCode::Down | KeyCode::Char('j'), _) => {
                        if sel + 1 < convs.len() {
                            sel += 1;
                        }
                    }
                    (KeyCode::Enter, _) => {
                        result = convs.get(sel).cloned();
                        break;
                    }
                    (KeyCode::Char('d') | KeyCode::Delete, _) => {
                        let conv_id = convs[sel]["id"].as_str().unwrap_or("").to_string();
                        let title = convs[sel]["title"]
                            .as_str()
                            .unwrap_or("(untitled)")
                            .to_string();
                        // Use QuestionWidget for confirmation
                        use crate::ui::question::{Question, QuestionOption};
                        let opts = vec![
                            QuestionOption {
                                label: "Yes — delete".to_string(),
                                description: String::new(),
                            },
                            QuestionOption {
                                label: "No — keep".to_string(),
                                description: String::new(),
                            },
                        ];
                        let q = Question {
                            header: "Delete?".to_string(),
                            text: format!("Delete conversation \"{title}\"?"),
                            options: opts.clone(),
                            multi_select: false,
                            allow_other: false,
                            progress: None,
                        };
                        let ans = {
                            let mut app = app_arc.lock();
                            app.ask_question(&q)?
                        };
                        if matches!(&ans, Some(a) if a.as_str().starts_with("Yes")) {
                            let _ = self.client.delete_conversation(agent_id, &conv_id).await;
                        }
                        return Ok(None);
                    }
                    (KeyCode::Char('c'), KeyModifiers::CONTROL) => break,
                    _ => {}
                }
            }
            // Redraw after state change
            let mut app = app_arc.lock();
            let items = build_items(sel);
            let n = convs.len();
            let mut ls = ListState::default().with_selected(Some(sel));
            app.terminal.draw(|f| {
                let area  = f.area();
                let block = Block::default()
                    .borders(Borders::ALL)
                    .title(format!(" Conversations [{}/{}]  ↑↓ navigate · Enter select · d delete · Esc cancel ", sel + 1, n))
                    .border_style(Style::default().fg(RC::Cyan));
                let list = List::new(items).block(block);
                f.render_stateful_widget(list, area, &mut ls);
            })?;
        }

        Ok(result)
    }
}
