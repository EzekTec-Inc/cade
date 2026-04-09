use crate::cli::repl::MemoryPickerResult;
use crate::Result;
use super::super::Repl;

impl Repl {
    /// `/memory` interactive picker
    pub(crate) async fn memory_picker(
        &self,
        app_arc: std::sync::Arc<parking_lot::Mutex<crate::ui::TuiApp>>,
        blocks: &mut [cade_agent::agent::client::MemoryBlock],
    ) -> Result<Option<MemoryPickerResult>> {
        use crossterm::event::{self, Event, KeyCode, KeyEventKind, KeyModifiers};
        use ratatui::{
            layout::{Constraint, Direction, Layout},
            style::{Color as RC, Modifier, Style},
            text::Span,
            widgets::{Block, Borders, Cell, Paragraph, Row, Table, TableState, Wrap},
        };

        if blocks.is_empty() {
            return Ok(None);
        }

        let mut filter_query = String::new();
        let mut selected_filtered: usize = 0;

        let do_draw = |app_arc: &std::sync::Arc<parking_lot::Mutex<crate::ui::TuiApp>>,
                       blocks: &[cade_agent::agent::client::MemoryBlock],
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
            let hint = format!("{}{}", f_disp, " ↑↓ j k  Enter/e Edit  p Pin/Unpin  d Delete  Esc/q cancel ");

            let rows: Vec<Row> = filtered_indices
                .iter()
                .enumerate()
                .map(|(i, &orig_idx)| {
                    let b = &blocks[orig_idx];
                    let is_sel = i == sel;

                    let style = if is_sel {
                        Style::default()
                            .bg(RC::DarkGray)
                            .add_modifier(Modifier::BOLD)
                    } else {
                        Style::default()
                    };

                    let tier_str = b.tier.as_deref().unwrap_or("short");
                    let (tier_icon, tier_color) = match tier_str {
                        "pinned" => ("📌 Pinned", RC::Magenta),
                        "long" => ("○ Long", RC::DarkGray),
                        _ => ("● Short", RC::Green),
                    };

                    let size = format!("{} chars", b.value.chars().count());
                    let desc = b.description.as_deref().unwrap_or("");

                    Row::new(vec![
                        Cell::from(Span::styled(tier_icon, Style::default().fg(tier_color))),
                        Cell::from(Span::styled(
                            b.label.clone(),
                            Style::default().fg(if is_sel { RC::White } else { RC::Cyan }),
                        )),
                        Cell::from(Span::styled(size, Style::default().fg(RC::DarkGray))),
                        Cell::from(Span::styled(
                            desc.to_string(),
                            Style::default().fg(RC::Gray),
                        )),
                    ])
                    .style(style)
                })
                .collect();

            let table = Table::new(
                rows,
                [
                    Constraint::Length(12),
                    Constraint::Length(25),
                    Constraint::Length(12),
                    Constraint::Min(20),
                ],
            )
            .header(
                Row::new(vec!["Tier", "Label", "Size", "Description"])
                    .style(Style::default().fg(RC::Cyan).add_modifier(Modifier::BOLD)),
            )
            .block(
                Block::default()
                    .borders(Borders::ALL)
                    .title(format!(" Memory Blocks {hint}"))
                    .border_style(Style::default().fg(RC::Cyan)),
            );

            let mut ts = TableState::default().with_selected(Some(sel));

            let preview_text = if !blocks.is_empty() && sel < blocks.len() {
                let b = &blocks[sel];
                b.value.clone()
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
            let filtered_indices: Vec<usize> = blocks
                .iter()
                .enumerate()
                .filter(|(_, b)| {
                    let d = b.description.as_deref().unwrap_or("");
                    q.is_empty()
                        || b.label.to_lowercase().contains(&q)
                        || d.to_lowercase().contains(&q)
                })
                .map(|(i, _)| i)
                .collect();

            if selected_filtered >= filtered_indices.len() {
                selected_filtered = filtered_indices.len().saturating_sub(1);
            }

            do_draw(
                &app_arc,
                blocks,
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
                            break Some(MemoryPickerResult::Edit(blocks[orig_idx].clone()));
                        }
                    }

                    (KeyCode::Char('e'), KeyModifiers::NONE) if filter_query.is_empty() => {
                        if !filtered_indices.is_empty() {
                            let orig_idx = filtered_indices[selected_filtered];
                            break Some(MemoryPickerResult::Edit(blocks[orig_idx].clone()));
                        }
                    }

                    (KeyCode::Char('p'), KeyModifiers::NONE) if filter_query.is_empty() => {
                        if !filtered_indices.is_empty() {
                            let orig_idx = filtered_indices[selected_filtered];
                            break Some(MemoryPickerResult::TogglePin(blocks[orig_idx].clone()));
                        }
                    }

                    (KeyCode::Delete, _) | (KeyCode::Char('d'), KeyModifiers::NONE) => {
                        let is_d = matches!(key.code, KeyCode::Char('d'));
                        if is_d && !filter_query.is_empty() {
                            filter_query.push('d');
                            continue;
                        }

                        if !filtered_indices.is_empty() {
                            let orig_idx = filtered_indices[selected_filtered];
                            let b = &blocks[orig_idx];

                            use crate::ui::question::{Question, QuestionOption};
                            let q = Question {
                                header: "Confirm".to_string(),
                                text: format!("Delete memory block '{}'?", b.label),
                                options: vec![
                                    QuestionOption {
                                        label: "Yes — delete".to_string(),
                                        description: String::new(),
                                    },
                                    QuestionOption {
                                        label: "No — cancel".to_string(),
                                        description: String::new(),
                                    },
                                ],
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
                                break Some(MemoryPickerResult::Delete(b.clone()));
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
