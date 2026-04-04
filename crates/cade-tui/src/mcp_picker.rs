use crate::{Result, colors::ThemeColors, overlay};
use cade_core::settings::McpServerConfig;
use crossterm::event::{self, Event, KeyCode, KeyEventKind, KeyModifiers};
use ratatui::{
    DefaultTerminal,
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color as RC, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Cell, Paragraph, Row, Table, TableState, Wrap},
};

pub enum McpAction {
    Edit(String),
    Toggle(String),
    Delete(String),
    New,
}

pub struct McpEntry {
    pub key: String,
    pub config: McpServerConfig,
    pub tool_count: Option<usize>, // None if disconnected
}

pub fn show_mcp_manager(
    terminal: &mut DefaultTerminal,
    servers: Vec<McpEntry>,
    colors: &ThemeColors,
) -> Result<Option<McpAction>> {
    if servers.is_empty() {
        // Fallback for empty state
        loop {
            terminal.draw(|f| {
                let area = f.area();
                let inner_shell = overlay::render_overlay_shell(f, area, "MCP Servers", colors);
                let inner = Rect {
                    x: inner_shell.x,
                    y: inner_shell.y,
                    width: inner_shell.width,
                    height: inner_shell.height.saturating_sub(1),
                };
                let hint_area = Rect {
                    x: inner_shell.x,
                    y: inner_shell.y + inner_shell.height.saturating_sub(1),
                    width: inner_shell.width,
                    height: 1,
                };
                f.render_widget(
                    Paragraph::new(vec![
                        Line::from(Span::styled(
                            "  No MCP servers configured.",
                            overlay::overlay_muted_style(colors),
                        )),
                        Line::from(""),
                        Line::from(Span::styled(
                            "  Press 'n' to scaffold your first MCP server.",
                            overlay::overlay_muted_style(colors),
                        )),
                    ]),
                    Rect {
                        x: inner.x,
                        y: inner.y + 2,
                        width: inner.width,
                        height: 3,
                    },
                );
                overlay::render_overlay_hint(f, hint_area, "n New  Esc close", colors);
            })?;
            if !event::poll(std::time::Duration::from_millis(200))? {
                continue;
            }
            if let Ok(Event::Key(key)) = event::read() {
                if key.kind != KeyEventKind::Press {
                    continue;
                }
                match key.code {
                    KeyCode::Esc | KeyCode::Char('q') => return Ok(None),
                    KeyCode::Char('n') => return Ok(Some(McpAction::New)),
                    _ => {}
                }
            }
        }
    }

    let mut filter_query = String::new();
    let mut selected_filtered: usize = 0;

    loop {
        let q = filter_query.to_lowercase();
        let filtered_indices: Vec<usize> = servers
            .iter()
            .enumerate()
            .filter(|(_, s)| {
                q.is_empty()
                    || s.key.to_lowercase().contains(&q)
                    || s.config.command.to_lowercase().contains(&q)
                    || s.config.url.as_deref().unwrap_or("").to_lowercase().contains(&q)
            })
            .map(|(i, _)| i)
            .collect();

        if selected_filtered >= filtered_indices.len() {
            selected_filtered = filtered_indices.len().saturating_sub(1);
        }

        terminal.draw(|f| {
            let area = f.area();
            let inner_shell = overlay::render_overlay_shell(f, area, "MCP Servers", colors);
            
            let main_chunks = Layout::default()
                .direction(Direction::Vertical)
                .constraints([Constraint::Min(5), Constraint::Length(3)].as_ref())
                .split(Rect {
                    x: inner_shell.x,
                    y: inner_shell.y,
                    width: inner_shell.width,
                    height: inner_shell.height.saturating_sub(1), // leave room for footer
                });

            let top_chunks = Layout::default()
                .direction(Direction::Horizontal)
                .constraints([Constraint::Percentage(40), Constraint::Percentage(60)].as_ref())
                .split(main_chunks[0]);

            let hint = " ↑↓ Navigate  e Edit  Space Toggle  n New  d Delete  Esc/q Close ";

            // -- Left Pane (Table)
            let rows: Vec<Row> = filtered_indices.iter().enumerate().map(|(i, &idx)| {
                let s = &servers[idx];
                let is_sel = i == selected_filtered;
                
                let style = if is_sel {
                    Style::default().bg(colors.overlay_selected_bg).add_modifier(Modifier::BOLD)
                } else {
                    Style::default()
                };

                let status_str = if s.config.disabled {
                    "Disabled"
                } else if s.tool_count.is_some() {
                    "Connected"
                } else {
                    "Error"
                };

                let status_color = if s.config.disabled {
                    colors.overlay_hint
                } else if s.tool_count.is_some() {
                    colors.success
                } else {
                    colors.error
                };

                let kind_str = if s.config.url.is_some() { "HTTP" } else { "Stdio" };

                Row::new(vec![
                    Cell::from(Span::styled(if is_sel { "▶ " } else { "  " }, Style::default().fg(if is_sel { colors.overlay_selected_fg } else { colors.overlay_hint }))),
                    Cell::from(Span::styled(s.key.clone(), Style::default().fg(if is_sel { RC::White } else { colors.text }))),
                    Cell::from(Span::styled(kind_str, Style::default().fg(colors.overlay_hint))),
                    Cell::from(Span::styled(status_str, Style::default().fg(status_color))),
                ]).style(style)
            }).collect();

            let table = Table::new(
                rows,
                [
                    Constraint::Length(3),
                    Constraint::Min(15),
                    Constraint::Length(8),
                    Constraint::Length(10),
                ],
            )
            .header(Row::new(vec!["", "Server", "Type", "Status"]).style(Style::default().fg(colors.overlay_title).add_modifier(Modifier::BOLD)))
            .block(Block::default().borders(Borders::ALL).title(format!(" MCP Servers {hint}")).border_style(Style::default().fg(colors.overlay_border)));

            let mut ts = TableState::default().with_selected(Some(selected_filtered));
            f.render_stateful_widget(table, top_chunks[0], &mut ts);

            // -- Right Pane (Preview)
            let preview_text = if !filtered_indices.is_empty() && selected_filtered < filtered_indices.len() {
                let s = &servers[filtered_indices[selected_filtered]];
                
                let mut meta = String::new();
                meta.push_str(&format!("ID: {}\n", s.key));
                
                if s.config.disabled {
                    meta.push_str("Status: Disabled\n");
                } else if let Some(tc) = s.tool_count {
                    meta.push_str(&format!("Status: Connected ({} tools)\n", tc));
                } else {
                    meta.push_str("Status: Connection Failed / Retrying\n");
                }

                if let Some(url) = &s.config.url {
                    meta.push_str(&format!("Transport: HTTP\nURL: {}\n", url));
                    if s.config.auth_token.is_some() {
                        meta.push_str("Auth: Bearer Token (set)\n");
                    }
                    if let Some(headers) = &s.config.headers {
                        if !headers.is_empty() {
                            meta.push_str("Headers:\n");
                            for (k, v) in headers {
                                meta.push_str(&format!("  {}: {}\n", k, v));
                            }
                        }
                    }
                } else {
                    meta.push_str(&format!("Transport: Stdio\nCommand: {}\n", s.config.command));
                    if !s.config.args.is_empty() {
                        meta.push_str(&format!("Args: [{}]\n", s.config.args.join(", ")));
                    }
                }

                if !s.config.env.is_empty() {
                    meta.push_str("Env:\n");
                    for (k, v) in &s.config.env {
                        meta.push_str(&format!("  {}={}\n", k, v));
                    }
                }

                if !s.config.write_tools.is_empty() {
                    meta.push_str(&format!("Write Tools: [{}]\n", s.config.write_tools.join(", ")));
                }

                meta
            } else {
                String::new()
            };

            let preview = Paragraph::new(preview_text)
                .wrap(Wrap { trim: false })
                .block(Block::default().borders(Borders::ALL).title(" Configuration ").border_style(Style::default().fg(colors.overlay_border)));
            f.render_widget(preview, top_chunks[1]);

            // -- Bottom Pane (Filter)
            let filter_block = Block::default().borders(Borders::ALL).title(" Filter (Type to search) ").border_style(Style::default().fg(colors.overlay_border));
            let filter_text = Paragraph::new(format!("> {}█", filter_query)).block(filter_block).style(Style::default().fg(colors.text));
            f.render_widget(filter_text, main_chunks[1]);

            // Footer hint
            let hint_area = Rect {
                x: inner_shell.x,
                y: inner_shell.y + inner_shell.height.saturating_sub(1),
                width: inner_shell.width,
                height: 1,
            };
            overlay::render_overlay_hint(f, hint_area, "Esc/q close", colors);
        })?;

        if !event::poll(std::time::Duration::from_millis(200))? {
            continue;
        }

        if let Ok(Event::Key(key)) = event::read() {
            if key.kind != KeyEventKind::Press {
                continue;
            }

            match (key.code, key.modifiers) {
                (KeyCode::Esc, _) | (KeyCode::Char('q'), KeyModifiers::NONE) => return Ok(None),
                (KeyCode::Char('c'), KeyModifiers::CONTROL) => return Ok(None),

                (KeyCode::Up, _) | (KeyCode::BackTab, _) => {
                    selected_filtered = selected_filtered.saturating_sub(1);
                }
                (KeyCode::Down, _) | (KeyCode::Tab, _) => {
                    if selected_filtered + 1 < filtered_indices.len() {
                        selected_filtered += 1;
                    }
                }

                (KeyCode::Char('e'), KeyModifiers::NONE) => {
                    if !filtered_indices.is_empty() {
                        let idx = filtered_indices[selected_filtered];
                        return Ok(Some(McpAction::Edit(servers[idx].key.clone())));
                    }
                }
                (KeyCode::Char('d'), KeyModifiers::NONE) => {
                    if !filtered_indices.is_empty() {
                        let idx = filtered_indices[selected_filtered];
                        return Ok(Some(McpAction::Delete(servers[idx].key.clone())));
                    }
                }
                (KeyCode::Char('n'), KeyModifiers::NONE) => {
                    return Ok(Some(McpAction::New));
                }
                (KeyCode::Char(' '), KeyModifiers::NONE) => {
                    if !filtered_indices.is_empty() {
                        let idx = filtered_indices[selected_filtered];
                        return Ok(Some(McpAction::Toggle(servers[idx].key.clone())));
                    }
                }
                (KeyCode::Char(c), m) if m == KeyModifiers::NONE || m == KeyModifiers::SHIFT => {
                    filter_query.push(c);
                }
                (KeyCode::Backspace, _) => {
                    filter_query.pop();
                }
                _ => {}
            }
        }
    }
}
