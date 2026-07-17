use super::super::Repl;
use crate::Result;
use crossterm::event::KeyCode;

impl Repl {
    /// `/artifacts` interactive split-pane browser — full-screen on TuiApp terminal.
    ///
    /// Keys:
    ///   - ↑ / ↓ / k / j : Move selection in the left artifact list
    ///   - Shift+↑ / Shift+↓ / Shift+k / Shift+j : Scroll the right preview pane
    ///   - Enter / v / s : View / fetch full content details
    ///   - d / Delete : Delete the selected artifact (with confirmation)
    ///   - f : Toggle filter/search input field
    ///   - Esc / q : Close the browser
    pub(crate) async fn artifacts_picker(
        &self,
        app_arc: std::sync::Arc<parking_lot::Mutex<crate::ui::TuiApp>>,
        initial_arts: Vec<serde_json::Value>,
        agent_id: &str,
    ) -> Result<()> {
        use crossterm::event::{self, Event, KeyEventKind, KeyModifiers};
        use ratatui::{
            layout::{Constraint, Direction, Layout},
            style::{Color as RC, Modifier, Style},
            text::{Line, Span},
            widgets::{Block, Borders, List, ListItem, ListState, Paragraph, Wrap},
        };
        use std::collections::HashMap;

        if initial_arts.is_empty() {
            let mut app = app_arc.lock();
            app.show_toast("No artifacts stored yet.", crate::ui::ToastLevel::Info);
            return Ok(());
        }

        let mut arts = initial_arts;
        let mut sel: usize = 0;
        let mut filter_query = String::new();
        let mut filtering_active = false;
        let mut preview_scroll: u16 = 0;

        // Memoized data_text cache to prevent redundant roundtrips as user moves cursor
        let mut content_cache: HashMap<String, String> = HashMap::new();

        let build_list_items = |arts_list: &[serde_json::Value], sel: usize| -> Vec<ListItem<'static>> {
            arts_list
                .iter()
                .enumerate()
                .map(|(i, a)| {
                    let id = a["id"].as_str().unwrap_or("?");
                    let kind = a["kind"].as_str().unwrap_or("other");
                    let size = a["size_bytes"].as_i64().unwrap_or(0);
                    let ts = a["created_at"].as_i64().unwrap_or(0);
                    
                    let date = if ts > 0 {
                        chrono::DateTime::from_timestamp(ts, 0)
                            .unwrap_or_default()
                            .with_timezone(&chrono::Local)
                            .format("%m/%d %H:%M")
                            .to_string()
                    } else {
                        String::new()
                    };

                    let short_id = if id.len() > 10 { &id[..10] } else { id };
                    let label = format!("  {kind:<12}  {size:>6}B  {date}  {short_id}");
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

        let mut last_selected_id = String::new();

        loop {
            // Filter the artifacts based on query
            let q = filter_query.to_lowercase();
            let filtered_arts: Vec<serde_json::Value> = arts
                .iter()
                .filter(|a| {
                    if q.is_empty() {
                        true
                    } else {
                        let kind = a["kind"].as_str().unwrap_or("").to_lowercase();
                        let id = a["id"].as_str().unwrap_or("").to_lowercase();
                        kind.contains(&q) || id.contains(&q)
                    }
                })
                .cloned()
                .collect();

            if filtered_arts.is_empty() {
                sel = 0;
            } else if sel >= filtered_arts.len() {
                sel = filtered_arts.len().saturating_sub(1);
            }

            // Detect selection change and fetch content on-demand
            let current_id = if !filtered_arts.is_empty() {
                filtered_arts[sel]["id"].as_str().unwrap_or("").to_string()
            } else {
                String::new()
            };

            if current_id != last_selected_id {
                last_selected_id = current_id.clone();
                preview_scroll = 0; // Reset scroll offset on item change
            }

            // Lazy fetch of the selected artifact's text body if not in cache
            if !current_id.is_empty() && !content_cache.contains_key(&current_id) {
                // Render a temporary "loading" indicator
                {
                    let mut app = app_arc.lock();
                    app.terminal.draw(|f| {
                        let block = Block::default()
                            .borders(Borders::ALL)
                            .title(" Artifacts (Loading details...) ")
                            .border_style(Style::default().fg(RC::Cyan));
                        f.render_widget(block, f.area());
                    })?;
                }

                match self.client.get_artifact(agent_id, &current_id).await {
                    Ok(detail) => {
                        let text = detail["data_text"].as_str().unwrap_or("").to_string();
                        content_cache.insert(current_id.clone(), text);
                    }
                    Err(e) => {
                        content_cache.insert(current_id.clone(), format!("Failed to retrieve artifact content: {e}"));
                    }
                }
            }

            // Render
            {
                let mut app = app_arc.lock();
                let items = build_list_items(&filtered_arts, sel);
                let mut ls = ListState::default().with_selected(Some(sel));

                let preview_text = if !current_id.is_empty() {
                    content_cache.get(&current_id).cloned().unwrap_or_else(|| "Loading...".to_string())
                } else {
                    "No artifacts match filter.".to_string()
                };

                let filter_title = if filtering_active {
                    format!(" [Search Mode: {}█] ", filter_query)
                } else if !filter_query.is_empty() {
                    format!(" [Filter: {}] ", filter_query)
                } else {
                    String::new()
                };

                app.terminal.draw(|f| {
                    let total_area = f.area();
                    
                    let main_layout = Layout::default()
                        .direction(Direction::Vertical)
                        .constraints([Constraint::Min(2), Constraint::Length(3)])
                        .split(total_area);

                    let top_chunks = Layout::default()
                        .direction(Direction::Horizontal)
                        .constraints([Constraint::Percentage(45), Constraint::Percentage(55)])
                        .split(main_layout[0]);

                    let list_block = Block::default()
                        .borders(Borders::ALL)
                        .title(format!(" Artifacts ({}){} ", filtered_arts.len(), filter_title))
                        .border_style(Style::default().fg(RC::Cyan));

                    let list = List::new(items).block(list_block);
                    f.render_stateful_widget(list, top_chunks[0], &mut ls);

                    let preview_block = Block::default()
                        .borders(Borders::ALL)
                        .title(format!(" Content Preview (Scroll: {}) ", preview_scroll))
                        .border_style(Style::default().fg(RC::DarkGray));

                    let preview = Paragraph::new(preview_text)
                        .wrap(Wrap { trim: false })
                        .scroll((preview_scroll, 0))
                        .block(preview_block);
                    f.render_widget(preview, top_chunks[1]);

                    let footer_block = Block::default()
                        .borders(Borders::ALL)
                        .title(" Actions ")
                        .border_style(Style::default().fg(RC::DarkGray));
                    let footer = Paragraph::new(" ↑↓ Move  Shift+↑↓ Scroll Preview  f Toggle Search  d Delete  Esc Close ")
                        .block(footer_block);
                    f.render_widget(footer, main_layout[1]);
                })?;
            }

            // Input handling
            if !event::poll(std::time::Duration::from_millis(200))? {
                continue;
            }

            if let Event::Key(k) = event::read()? {
                if k.kind != KeyEventKind::Press {
                    continue;
                }

                // Search mode input interception
                if filtering_active {
                    match k.code {
                        KeyCode::Esc | KeyCode::Enter => {
                            filtering_active = false;
                            continue;
                        }
                        KeyCode::Backspace => {
                            filter_query.pop();
                            continue;
                        }
                        KeyCode::Char(c) if k.modifiers == KeyModifiers::NONE || k.modifiers == KeyModifiers::SHIFT => {
                            filter_query.push(c);
                            continue;
                        }
                        _ => {}
                    }
                }

                match (k.code, k.modifiers) {
                    (KeyCode::Char('q') | KeyCode::Esc, _) => break,
                    (KeyCode::Char('c'), KeyModifiers::CONTROL) => break,

                    // Normal list navigation
                    (KeyCode::Up, m) if m == KeyModifiers::NONE => {
                        sel = sel.saturating_sub(1);
                    }
                    (KeyCode::Down, m) if m == KeyModifiers::NONE => {
                        if sel + 1 < filtered_arts.len() {
                            sel += 1;
                        }
                    }
                    (KeyCode::Char('k'), m) if m == KeyModifiers::NONE => {
                        sel = sel.saturating_sub(1);
                    }
                    (KeyCode::Char('j'), m) if m == KeyModifiers::NONE => {
                        if sel + 1 < filtered_arts.len() {
                            sel += 1;
                        }
                    }

                    // Shift-navigation for scrolling the preview panel
                    (KeyCode::Up, KeyModifiers::SHIFT) | (KeyCode::Char('K'), _) => {
                        preview_scroll = preview_scroll.saturating_sub(1);
                    }
                    (KeyCode::Down, KeyModifiers::SHIFT) | (KeyCode::Char('J'), _) => {
                        preview_scroll = preview_scroll.saturating_add(1);
                    }

                    // Search toggle
                    (KeyCode::Char('f'), KeyModifiers::NONE) => {
                        filtering_active = true;
                    }

                    // Delete artifact
                    (KeyCode::Char('d') | KeyCode::Delete, _) => {
                        if filtered_arts.is_empty() {
                            continue;
                        }
                        let art_id = filtered_arts[sel]["id"].as_str().unwrap_or("").to_string();
                        let kind = filtered_arts[sel]["kind"].as_str().unwrap_or("other").to_string();

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
                            text: format!("Delete artifact \"{kind} ({})\"?", &art_id[..8.min(art_id.len())]),
                            options: opts,
                            multi_select: false,
                            allow_other: false,
                            progress: None,
                        };
                        let ans = {
                            let mut app = app_arc.lock();
                            app.ask_question(&q)?
                        };
                        if matches!(&ans, Some(a) if a.as_str().starts_with("Yes")) {
                            match self.client.delete_artifact(agent_id, &art_id).await {
                                Ok(true) => {
                                    arts.retain(|a| a["id"].as_str() != Some(&art_id));
                                    content_cache.remove(&art_id);
                                    let mut app = app_arc.lock();
                                    app.show_toast("Artifact deleted successfully.", crate::ui::ToastLevel::Info);
                                }
                                _ => {
                                    let mut app = app_arc.lock();
                                    app.show_toast("Failed to delete artifact.", crate::ui::ToastLevel::Error);
                                }
                            }
                        }
                    }
                    _ => {}
                }
            }
        }

        Ok(())
    }
}
