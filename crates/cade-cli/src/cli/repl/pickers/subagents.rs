use super::super::Repl;
use crate::Result;
use crate::cli::repl::SubagentPickerResult;

impl Repl {
    /// `/subagents` interactive picker
    pub(crate) async fn subagent_picker(
        &self,
        app_arc: std::sync::Arc<parking_lot::Mutex<crate::ui::TuiApp>>,
        subagents: &[cade_agent::subagents::SubagentDef],
    ) -> Result<Option<SubagentPickerResult>> {
        use crossterm::event::{self, Event, KeyCode, KeyEventKind, KeyModifiers};
        use ratatui::{
            layout::{Constraint, Direction, Layout},
            style::{Color as RC, Modifier, Style},
            text::Span,
            widgets::{Block, Borders, Cell, Paragraph, Row, Table, TableState, Wrap},
        };

        if subagents.is_empty() {
            return Ok(None);
        }

        let mut filter_query = String::new();
        let mut selected_filtered: usize = 0;

        let do_draw = |app_arc: &std::sync::Arc<parking_lot::Mutex<crate::ui::TuiApp>>,
                       subagents: &[cade_agent::subagents::SubagentDef],
                       filtered_indices: &[usize],
                       sel: usize,
                       filter_query: &str|
         -> Result<()> {
            let mut app = app_arc.lock();

            let f_disp = if filter_query.is_empty() {
                String::new()
            } else {
                format!(" [Filter: {}] ", filter_query)
            };
            let hint = format!(
                "{}{}",
                f_disp, " ↑↓ Navigate  Enter Select  e Edit  Esc/q Cancel "
            );

            let rows: Vec<Row> = filtered_indices
                .iter()
                .enumerate()
                .map(|(i, &original_idx)| {
                    let s = &subagents[original_idx];
                    let is_sel = i == sel;

                    let style = if is_sel {
                        Style::default()
                            .bg(RC::DarkGray)
                            .add_modifier(Modifier::BOLD)
                    } else {
                        Style::default()
                    };

                    let scope_str = format!("{:?}", s.scope).to_lowercase();
                    let scope_color = match s.scope {
                        cade_agent::subagents::SubagentScope::Builtin => RC::Magenta,
                        cade_agent::subagents::SubagentScope::Global => RC::Green,
                        cade_agent::subagents::SubagentScope::Project => RC::Cyan,
                    };

                    let model_str = s.model.as_deref().unwrap_or("inherited");

                    Row::new(vec![
                        Cell::from(Span::styled(
                            scope_str,
                            Style::default().fg(if is_sel { RC::White } else { scope_color }),
                        )),
                        Cell::from(Span::styled(
                            s.name.clone(),
                            Style::default().fg(if is_sel { RC::White } else { RC::Cyan }),
                        )),
                        Cell::from(Span::styled(model_str, Style::default().fg(RC::DarkGray))),
                    ])
                    .style(style)
                })
                .collect();

            let table = Table::new(
                rows,
                [
                    Constraint::Length(12),
                    Constraint::Length(25),
                    Constraint::Min(20),
                ],
            )
            .header(
                Row::new(vec!["Scope", "Name", "Model"])
                    .style(Style::default().fg(RC::Cyan).add_modifier(Modifier::BOLD)),
            )
            .block(
                Block::default()
                    .borders(Borders::ALL)
                    .title(format!(" Subagents {hint}"))
                    .border_style(Style::default().fg(RC::Cyan)),
            );

            let mut ts = TableState::default().with_selected(Some(sel));

            let preview_text = if !filtered_indices.is_empty() && sel < filtered_indices.len() {
                let s = &subagents[filtered_indices[sel]];
                let meta = format!(
                    "Description: {}
Tools: {}
Model: {}
Skills: {}

-- System Prompt --

",
                    s.description,
                    s.tools,
                    s.model.as_deref().unwrap_or("inherited"),
                    s.skills.join(", ")
                );
                format!("{}{}", meta, s.system_prompt)
            } else {
                String::new()
            };

            let preview = Paragraph::new(preview_text)
                .wrap(Wrap { trim: false })
                .block(
                    Block::default()
                        .borders(Borders::ALL)
                        .title(" Preview ")
                        .border_style(Style::default().fg(RC::DarkGray)),
                );

            app.terminal.draw(|f| {
                let top_chunks = Layout::default()
                    .direction(Direction::Horizontal)
                    .constraints([Constraint::Percentage(40), Constraint::Percentage(60)].as_ref())
                    .split(f.area());

                f.render_stateful_widget(table, top_chunks[0], &mut ts);
                f.render_widget(preview, top_chunks[1]);
            })?;
            Ok(())
        };

        let result = loop {
            let q = filter_query.to_lowercase();
            let filtered_indices: Vec<usize> = subagents
                .iter()
                .enumerate()
                .filter(|(_, s)| {
                    q.is_empty()
                        || s.name.to_lowercase().contains(&q)
                        || s.description.to_lowercase().contains(&q)
                        || format!("{:?}", s.scope).to_lowercase().contains(&q)
                })
                .map(|(i, _)| i)
                .collect();

            if selected_filtered >= filtered_indices.len() {
                selected_filtered = filtered_indices.len().saturating_sub(1);
            }

            do_draw(
                &app_arc,
                subagents,
                &filtered_indices,
                selected_filtered,
                &filter_query,
            )?;

            if !event::poll(std::time::Duration::from_millis(200))? {
                continue;
            }
            if let Ok(Event::Key(key)) = event::read() {
                if key.kind != KeyEventKind::Press {
                    continue;
                }
                match (key.code, key.modifiers) {
                    (KeyCode::Esc, _) => break None,
                    (KeyCode::Char('c'), KeyModifiers::CONTROL) => break None,

                    (KeyCode::Up, _) | (KeyCode::BackTab, _) => {
                        selected_filtered = selected_filtered.saturating_sub(1);
                    }
                    (KeyCode::Down, _) | (KeyCode::Tab, _) => {
                        if selected_filtered + 1 < filtered_indices.len() {
                            selected_filtered += 1;
                        }
                    }

                    (KeyCode::Enter, _) => {
                        if !filtered_indices.is_empty() {
                            let orig_idx = filtered_indices[selected_filtered];
                            break Some(SubagentPickerResult::Run(
                                subagents[orig_idx].name.clone(),
                            ));
                        }
                    }

                    (KeyCode::Char('e'), KeyModifiers::NONE) if filter_query.is_empty() => {
                        if !filtered_indices.is_empty() {
                            let orig_idx = filtered_indices[selected_filtered];
                            let s = &subagents[orig_idx];
                            if let Some(path) = &s.path {
                                break Some(SubagentPickerResult::Edit(path.clone()));
                            } else {
                                app_arc.lock().show_toast(
                                    "Built-in subagents cannot be edited",
                                    crate::ui::ToastLevel::Error,
                                );
                            }
                        }
                    }

                    (KeyCode::Char(c), m)
                        if m == KeyModifiers::NONE || m == KeyModifiers::SHIFT =>
                    {
                        filter_query.push(c);
                    }
                    (KeyCode::Backspace, _) => {
                        filter_query.pop();
                    }

                    _ => {}
                }
            }
        };

        Ok(result)
    }
}
