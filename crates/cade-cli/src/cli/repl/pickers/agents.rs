use cade_agent::agent::client::AgentState;
use crate::cli::repl::AgentPickerResult;
use crate::Result;
use super::super::Repl;

impl Repl {
    /// `/agents` TUI picker — full-screen on TuiApp terminal.
    ///
    /// Keys:
    ///   ↑/↓  j/k  — move cursor
    ///   Space      — toggle mark for deletion
    ///   d / Delete — confirm delete of all marked (or current if none marked)
    ///   r          — rename highlighted agent
    ///   Enter      — switch to highlighted agent (only when no marks)
    ///   Esc / q    — cancel
    pub(crate) async fn agent_picker(
        &self,
        app_arc: std::sync::Arc<parking_lot::Mutex<crate::ui::TuiApp>>,
        agents: &mut [AgentState],
    ) -> Result<Option<AgentPickerResult>> {
        use crossterm::event::{self, Event, KeyCode, KeyEventKind, KeyModifiers};
        use ratatui::{
            layout::Constraint,
            style::{Color as RC, Modifier, Style},
            text::Span,
            widgets::{Block, Borders, Cell, Row, Table, TableState},
        };
        use std::collections::HashSet;

        if agents.is_empty() {
            return Ok(None);
        }

        let current = self.agent_id();
        let mut marked: HashSet<usize> = HashSet::new();
        let mut selected_idx: usize = 0;

        let do_draw = |app_arc: &std::sync::Arc<parking_lot::Mutex<crate::ui::TuiApp>>,
                       agents: &[AgentState],
                       sel: usize,
                       marked: &HashSet<usize>,
                       current: &str|
         -> Result<()> {
            let mut app = app_arc.lock();

            let n = marked.len();
            let hint = if n == 0 {
                " ↑↓  Space mark  r rename  d delete  Enter switch  Esc cancel ".to_string()
            } else {
                format!(" [{n} marked]  d delete all  Esc cancel ")
            };

            let rows: Vec<Row> = agents
                .iter()
                .enumerate()
                .map(|(i, a)| {
                    let is_sel = i == sel;
                    let is_marked = marked.contains(&i);
                    let is_active = a.id == current;
                    let short_id = if a.id.len() > 22 {
                        a.id[..22].to_string() + "…"
                    } else {
                        a.id.clone()
                    };
                    let model_str = a.model.clone().unwrap_or_else(|| "unknown".to_string());

                    let style = if is_sel {
                        Style::default()
                            .bg(RC::DarkGray)
                            .add_modifier(Modifier::BOLD)
                    } else {
                        Style::default()
                    };

                    let mark_span = Span::styled(
                        if is_marked { "☑" } else { "☐" },
                        Style::default().fg(if is_marked { RC::Yellow } else { RC::DarkGray }),
                    );

                    let active_span = Span::styled(
                        if is_active { "★ active" } else { "" },
                        Style::default().fg(RC::Cyan),
                    );

                    Row::new(vec![
                        Cell::from(mark_span),
                        Cell::from(Span::styled(
                            a.name.clone(),
                            Style::default().fg(if is_sel { RC::White } else { RC::Gray }),
                        )),
                        Cell::from(Span::styled(model_str, Style::default().fg(RC::DarkGray))),
                        Cell::from(Span::styled(short_id, Style::default().fg(RC::DarkGray))),
                        Cell::from(active_span),
                    ])
                    .style(style)
                })
                .collect();

            let table = Table::new(
                rows,
                [
                    Constraint::Length(3),
                    Constraint::Length(30),
                    Constraint::Length(30),
                    Constraint::Length(25),
                    Constraint::Min(10),
                ],
            )
            .header(
                Row::new(vec!["M", "Name", "Model", "ID", "Status"])
                    .style(Style::default().fg(RC::Cyan).add_modifier(Modifier::BOLD)),
            )
            .block(
                Block::default()
                    .borders(Borders::ALL)
                    .title(format!(" Agents {hint}"))
                    .border_style(Style::default().fg(RC::Cyan)),
            );

            let mut ts = TableState::default().with_selected(Some(sel));

            app.terminal.draw(|f| {
                f.render_stateful_widget(table, f.area(), &mut ts);
            })?;
            Ok(())
        };

        let result = loop {
            if selected_idx >= agents.len() {
                selected_idx = agents.len().saturating_sub(1);
            }

            do_draw(
                &app_arc,
                agents,
                selected_idx,
                &marked,
                &current,
            )?;

            if !event::poll(std::time::Duration::from_millis(200))? {
                continue;
            }
            if let Ok(Event::Key(key)) = event::read() {
                if key.kind != KeyEventKind::Press {
                    continue;
                }
                match (key.code, key.modifiers) {
                    (KeyCode::Esc, _) | (KeyCode::Char('q'), KeyModifiers::NONE) => break None,

                    // Allow Ctrl+C to also cancel
                    (KeyCode::Char('c'), KeyModifiers::CONTROL) => break None,

                    (KeyCode::Up, _) | (KeyCode::BackTab, _) | (KeyCode::Char('k'), _) => {
                        selected_idx = selected_idx.saturating_sub(1);
                    }
                    (KeyCode::Down, _) | (KeyCode::Tab, _) | (KeyCode::Char('j'), _) => {
                        if selected_idx + 1 < agents.len() {
                            selected_idx += 1;
                        }
                    }

                    (KeyCode::Enter, _) => {
                        if marked.is_empty() && !agents.is_empty() {
                            let a = agents[selected_idx].clone();
                            if a.id != current {
                                break Some(AgentPickerResult::Switch(a));
                            }
                        }
                    }

                    (KeyCode::Char(' '), _) => {
                        if !agents.is_empty() {
                            if marked.contains(&selected_idx) {
                                marked.remove(&selected_idx);
                            } else {
                                marked.insert(selected_idx);
                            }
                        }
                    }

                    (KeyCode::Delete, _) | (KeyCode::Char('d'), KeyModifiers::NONE) => {
                        // Deletion logic...
                        let targets: Vec<usize> = if marked.is_empty() {
                            if agents.is_empty() {
                                continue;
                            }
                            vec![selected_idx]
                        } else {
                            let mut v: Vec<usize> = marked.iter().copied().collect();
                            v.sort_unstable();
                            v
                        };
                        let names: Vec<String> =
                            targets.iter().map(|&i| agents[i].name.clone()).collect();
                        let label = if targets.len() == 1 {
                            format!("Delete '{}'?", names[0])
                        } else {
                            format!("Delete {} agents ({})?", targets.len(), names.join(", "))
                        };
                        use crate::ui::question::{Question, QuestionOption};
                        let opts = vec![
                            QuestionOption {
                                label: "Yes — delete".to_string(),
                                description: String::new(),
                            },
                            QuestionOption {
                                label: "No — cancel".to_string(),
                                description: String::new(),
                            },
                        ];
                        let q = Question {
                            header: "Confirm".to_string(),
                            text: label.clone(),
                            options: opts.clone(),
                            multi_select: false,
                            allow_other: false,
                            progress: None,
                        };
                        let confirmed = {
                            let mut app = app_arc.lock();
                            let r = app.ask_question(&q)?;
                            app.scroll = 0;
                            let _ = app.draw();
                            matches!(&r, Some(a) if a.as_str().starts_with("Yes"))
                        };
                        if confirmed {
                            if targets.len() == 1 {
                                let orig_idx = targets[0];
                                let a = agents[orig_idx].clone();
                                match self.client.delete_agent(&a.id).await {
                                    Ok(_) => {
                                        agents[orig_idx].name = format!("{} (deleted)", a.name);
                                        marked.remove(&orig_idx);
                                        if a.id == current {
                                            break Some(AgentPickerResult::Switch(a));
                                        }
                                    }
                                    Err(e) => {
                                        app_arc.lock().show_toast(
                                            e.to_string(),
                                            crate::ui::ToastLevel::Error,
                                        );
                                    }
                                }
                            } else {
                                break Some(AgentPickerResult::DeleteMany(
                                    targets.into_iter().map(|i| agents[i].clone()).collect(),
                                ));
                            }
                        }
                    }

                    (KeyCode::Char('r'), KeyModifiers::NONE) => {
                        // rename
                        if !agents.is_empty() {
                            let orig_idx = selected_idx;
                            let a = agents[orig_idx].clone();
                            use crate::ui::question::{Question, QuestionOption};
                            let q = Question {
                                header: "Rename".to_string(),
                                text: format!("Rename '{}':", a.name),
                                options: vec![QuestionOption {
                                    label: "Cancel".to_string(),
                                    description: String::new(),
                                }],
                                multi_select: false,
                                allow_other: true,
                                progress: None,
                            };
                            let name = {
                                let mut app = app_arc.lock();
                                let ans = app.ask_question(&q)?;
                                app.scroll = 0;
                                let _ = app.draw();
                                match ans {
                                    Some(n) if n.as_str() != "Cancel" && !n.as_str().is_empty() => {
                                        n.as_str().to_string()
                                    }
                                    _ => String::new(),
                                }
                            };
                            if !name.is_empty() {
                                match self.client.rename_agent(&a.id, &name).await {
                                    Ok(_) => {
                                        agents[orig_idx].name = name.clone();
                                        if a.id == current {
                                            break Some(AgentPickerResult::Rename {
                                                agent: a,
                                                new_name: name,
                                            });
                                        }
                                    }
                                    Err(e) => {
                                        app_arc.lock().show_toast(
                                            e.to_string(),
                                            crate::ui::ToastLevel::Error,
                                        );
                                    }
                                }
                            }
                        }
                    }

                    _ => {}
                }
            }
        };

        Ok(result)
    }
}
