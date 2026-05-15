use super::super::Repl;
use crate::Result;
use cade_plugin::marketplace::RegistryPluginInfo;
use cade_tui::colors::ThemeColorsExt;
use crossterm::event::{self, Event, KeyCode, KeyEventKind, KeyModifiers};
use ratatui::{
    layout::{Constraint, Direction, Layout},
    style::{Modifier, Style},
    text::Span,
    widgets::{Block, Borders, Cell, Paragraph, Row, Table, TableState, Wrap},
};

pub enum MarketplaceActionResult {
    Install(String, String), // (url, plugin_id)
}

impl Repl {
    /// `/marketplace` interactive picker
    pub(crate) async fn marketplace_picker(
        &self,
        app_arc: std::sync::Arc<parking_lot::Mutex<crate::ui::TuiApp>>,
        plugins: &[RegistryPluginInfo],
    ) -> Result<Option<MarketplaceActionResult>> {
        if plugins.is_empty() {
            let mut app = app_arc.lock();
            app.show_toast(
                "Marketplace is currently empty or unreachable.",
                crate::ui::ToastLevel::Warning,
            );
            return Ok(None);
        }

        let mut filter_query = String::new();
        let mut selected_filtered: usize = 0;

        let do_draw = |app_arc: &std::sync::Arc<parking_lot::Mutex<crate::ui::TuiApp>>,
                       plugins: &[RegistryPluginInfo],
                       filtered_indices: &[usize],
                       sel: usize,
                       filter_query: &str|
         -> Result<()> {
            let mut app = app_arc.lock();
            let colors = app.colors.clone();

            let f_disp = if filter_query.is_empty() {
                String::new()
            } else {
                format!(" [Filter: {}] ", filter_query)
            };
            let hint = format!("{}{}", f_disp, " ↑↓ Navigate  Enter Install  Esc/q Cancel ");

            let rows: Vec<Row> = filtered_indices
                .iter()
                .enumerate()
                .map(|(i, &original_idx)| {
                    let p = &plugins[original_idx];
                    let is_sel = i == sel;

                    let style = if is_sel {
                        Style::default()
                            .bg(colors.c_bg_surface1())
                            .add_modifier(Modifier::BOLD)
                    } else {
                        Style::default()
                    };

                    Row::new(vec![
                        Cell::from(Span::styled(
                            p.id.clone(),
                            Style::default().fg(if is_sel {
                                colors.c_text_primary()
                            } else {
                                colors.c_primary()
                            }),
                        )),
                        Cell::from(Span::styled(
                            p.author.clone(),
                            Style::default().fg(if is_sel {
                                colors.c_text_primary()
                            } else {
                                colors.c_text_muted()
                            }),
                        )),
                        Cell::from(Span::styled(
                            p.version.clone(),
                            Style::default().fg(colors.c_text_muted()),
                        )),
                    ])
                    .style(style)
                })
                .collect();

            let table = Table::new(
                rows,
                [
                    Constraint::Percentage(40),
                    Constraint::Percentage(40),
                    Constraint::Percentage(20),
                ],
            )
            .header(
                Row::new(vec!["ID", "Author", "Version"]).style(
                    Style::default()
                        .fg(colors.c_primary())
                        .add_modifier(Modifier::BOLD),
                ),
            )
            .block(
                Block::default()
                    .borders(Borders::ALL)
                    .title(format!(" Plugin Marketplace {hint}"))
                    .border_type(colors.c_border_style())
                    .border_style(Style::default().fg(colors.c_border_accent())),
            );

            let mut ts = TableState::default().with_selected(Some(sel));

            let preview_text = if !filtered_indices.is_empty() && sel < filtered_indices.len() {
                let p = &plugins[filtered_indices[sel]];
                format!(
                    "ID: {}\nAuthor: {}\nVersion: {}\nTags: {}\n\n{}\n\nURL: {}",
                    p.id,
                    p.author,
                    p.version,
                    p.tags.join(", "),
                    p.description,
                    p.url
                )
            } else {
                String::new()
            };

            let preview = Paragraph::new(preview_text)
                .wrap(Wrap { trim: false })
                .block(
                    Block::default()
                        .borders(Borders::ALL)
                        .title(" Preview ")
                        .border_type(colors.c_border_style())
                        .border_style(Style::default().fg(colors.c_border_muted())),
                );

            app.terminal.draw(|f| {
                let area = f.area();

                let top_chunks = Layout::default()
                    .direction(Direction::Horizontal)
                    .constraints([Constraint::Percentage(50), Constraint::Percentage(50)].as_ref())
                    .split(area);

                f.render_stateful_widget(table, top_chunks[0], &mut ts);
                f.render_widget(preview, top_chunks[1]);
            })?;
            Ok(())
        };

        let result = loop {
            let q = filter_query.to_lowercase();
            let filtered_indices: Vec<usize> = plugins
                .iter()
                .enumerate()
                .filter(|(_, p)| {
                    q.is_empty()
                        || p.id.to_lowercase().contains(&q)
                        || p.description.to_lowercase().contains(&q)
                        || p.author.to_lowercase().contains(&q)
                        || p.tags.iter().any(|t| t.to_lowercase().contains(&q))
                })
                .map(|(i, _)| i)
                .collect();

            if selected_filtered >= filtered_indices.len() {
                selected_filtered = filtered_indices.len().saturating_sub(1);
            }

            do_draw(
                &app_arc,
                plugins,
                &filtered_indices,
                selected_filtered,
                &filter_query,
            )?;

            if !event::poll(std::time::Duration::from_millis(100))? {
                continue;
            }

            if let Ok(Event::Key(key)) = event::read() {
                if key.kind != KeyEventKind::Press {
                    continue;
                }
                match key.code {
                    KeyCode::Esc => break None,
                    KeyCode::Char('q') if key.modifiers.is_empty() && filter_query.is_empty() => {
                        break None;
                    }
                    KeyCode::Up | KeyCode::Char('k')
                        if key.modifiers.is_empty() && filter_query.is_empty() =>
                    {
                        selected_filtered = selected_filtered.saturating_sub(1);
                    }
                    KeyCode::Down | KeyCode::Char('j')
                        if key.modifiers.is_empty() && filter_query.is_empty() =>
                    {
                        if selected_filtered + 1 < filtered_indices.len() {
                            selected_filtered += 1;
                        }
                    }
                    KeyCode::Enter => {
                        if !filtered_indices.is_empty()
                            && selected_filtered < filtered_indices.len()
                        {
                            let p = &plugins[filtered_indices[selected_filtered]];
                            break Some(MarketplaceActionResult::Install(
                                p.url.clone(),
                                p.id.clone(),
                            ));
                        }
                    }
                    KeyCode::Char('w') if key.modifiers == KeyModifiers::CONTROL => {
                        if let Some(pos) = filter_query.rfind(' ') {
                            filter_query.truncate(pos);
                        } else {
                            filter_query.clear();
                        }
                    }
                    KeyCode::Backspace => {
                        filter_query.pop();
                    }
                    KeyCode::Char(c) if !c.is_control() => {
                        filter_query.push(c);
                    }
                    _ => {}
                }
            }
        };

        // Redraw underlying UI to clear the overlay
        app_arc.lock().draw()?;
        Ok(result)
    }
}
