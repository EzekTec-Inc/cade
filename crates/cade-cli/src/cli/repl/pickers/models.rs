use crate::Result;
use super::super::Repl;

impl Repl {
    /// Interactive model picker — full-screen on TuiApp terminal.
    /// Returns the selected model string or None if cancelled.
    pub(crate) async fn interactive_model_picker(
        &self,
        app_arc: std::sync::Arc<parking_lot::Mutex<crate::ui::TuiApp>>,
    ) -> Result<Option<String>> {
        use crossterm::event::{self, Event, KeyCode, KeyEventKind};
        use ratatui::{
            layout::{Constraint, Direction, Layout},
            style::{Color as RC, Modifier, Style},
            text::{Line, Span},
            widgets::{
                Block, Borders, List, ListItem, ListState, Scrollbar, ScrollbarOrientation,
                ScrollbarState,
            },
        };

        {
            let mut app = app_arc.lock();
            let _ = app.push(crate::ui::RenderLine::DimMsg(
                "  Fetching models…".to_string(),
            ));
        }

        let current = self.model();

        // -- Fetch model list
        // (provider, display_name, model_id, toolset, is_dynamic)
        let mut models: Vec<(String, String, String, String, bool)> = Vec::new();
        let mut custom_providers: Vec<String> = Vec::new();

        match self.client.list_models().await {
            Ok(body) => {
                if let Some(arr) = body["supported"].as_array() {
                    for m in arr {
                        models.push((
                            m["provider"].as_str().unwrap_or("?").to_string(),
                            m["display_name"].as_str().unwrap_or("?").to_string(),
                            m["id"].as_str().unwrap_or("?").to_string(),
                            m["toolset"].as_str().unwrap_or("default").to_string(),
                            false,
                        ));
                    }
                }
                if let Some(arr) = body["dynamic"].as_array() {
                    for m in arr {
                        let id = m["id"].as_str().unwrap_or("?").to_string();
                        let provider = m["provider"].as_str().unwrap_or("?").to_string();
                        if !models.iter().any(|(_, _, mid, _, _)| mid == &id) {
                            models.push((
                                provider,
                                m["display_name"].as_str().unwrap_or(&id).to_string(),
                                id,
                                m["toolset"].as_str().unwrap_or("default").to_string(),
                                true,
                            ));
                        }
                    }
                }
                if let Some(arr) = body["custom_providers"].as_array() {
                    custom_providers = arr
                        .iter()
                        .filter_map(|v| v.as_str().map(String::from))
                        .collect();
                }
            }
            Err(_) => {
                let mut app = app_arc.lock();
                let _ = app.push(crate::ui::RenderLine::ErrorMsg(
                    "Could not fetch models. Specify directly: /model provider/model-name"
                        .to_string(),
                ));
                return Ok(None);
            }
        }

        for cp in &custom_providers {
            models.push((
                cp.clone(),
                format!("Enter model for {cp}…"),
                format!("{cp}/"),
                "default".to_string(),
                false,
            ));
        }
        // Sentinel: always-last "Enter custom model ID" entry
        models.push((
            "__custom__".to_string(),
            "Enter custom model ID…".to_string(),
            String::new(),
            String::new(),
            false,
        ));

        if models.len() == 1 {
            let mut app = app_arc.lock();
            let _ = app.push(crate::ui::RenderLine::DimMsg(
                "  No models available. Connect a provider: /connect".to_string(),
            ));
            return Ok(None);
        }

        let n_models = models.len();

        // -- Flat display-item list (provider headers + model rows)
        #[derive(Clone)]
        enum DisplayItem {
            Header(String, bool),
            ModelRow(usize),
        }

        let display_items: Vec<DisplayItem> = {
            let mut items = Vec::new();
            let mut last_p = String::new();
            for (i, (provider, _, _, _, dynamic)) in models.iter().enumerate() {
                if *provider != last_p {
                    items.push(DisplayItem::Header(provider.clone(), *dynamic));
                    last_p = provider.clone();
                }
                items.push(DisplayItem::ModelRow(i));
            }
            items
        };
        let disp_len = display_items.len();

        // list_pos = position in display_items (never on a Header)
        let initial_list_pos = display_items
            .iter()
            .position(|d| matches!(d, DisplayItem::ModelRow(i) if models[*i].2 == current))
            .or_else(|| {
                display_items
                    .iter()
                    .position(|d| matches!(d, DisplayItem::ModelRow(_)))
            })
            .unwrap_or(0);
        let mut list_pos = initial_list_pos;

        // Navigate display_items, skipping Header items
        let next_pos = |mut p: usize| -> usize {
            loop {
                p = (p + 1) % disp_len;
                if !matches!(display_items.get(p), Some(DisplayItem::Header(..))) {
                    return p;
                }
            }
        };
        let prev_pos = |mut p: usize| -> usize {
            loop {
                p = if p == 0 { disp_len - 1 } else { p - 1 };
                if !matches!(display_items.get(p), Some(DisplayItem::Header(..))) {
                    return p;
                }
            }
        };
        // Derive selected model index from list_pos
        let model_at = |p: usize| -> usize {
            if let Some(DisplayItem::ModelRow(i)) = display_items.get(p) {
                *i
            } else {
                0
            }
        };

        // -- Build ratatui ListItems
        let build_items = |list_pos: usize, current: &str| -> Vec<ListItem<'static>> {
            display_items
                .iter()
                .map(|item| match item {
                    DisplayItem::Header(provider, dynamic) => {
                        if provider == "__custom__" {
                            ListItem::new(Line::from(Span::styled(
                                "  ─────────────────────────────────────────".to_string(),
                                Style::default().fg(RC::DarkGray),
                            )))
                        } else {
                            let suffix = if *dynamic {
                                if provider == "ollama" {
                                    " (local)"
                                } else {
                                    " (live)"
                                }
                            } else {
                                ""
                            };
                            ListItem::new(Line::from(Span::styled(
                                format!("  {}{}", provider.to_uppercase(), suffix),
                                Style::default().fg(RC::Yellow).add_modifier(Modifier::BOLD),
                            )))
                        }
                    }
                    DisplayItem::ModelRow(i) => {
                        let (provider, name, id, toolset, _) = &models[*i];
                        let is_sel = *i == model_at(list_pos);
                        let is_current = !id.is_empty() && id == current;

                        if provider == "__custom__" {
                            ListItem::new(Line::from(vec![
                                Span::styled(
                                    if is_sel { "  ▶ " } else { "    " }.to_string(),
                                    Style::default().fg(RC::Cyan),
                                ),
                                Span::styled(
                                    name.clone(),
                                    Style::default().fg(if is_sel {
                                        RC::Cyan
                                    } else {
                                        RC::DarkGray
                                    }),
                                ),
                            ]))
                        } else {
                            let name_trunc = if name.len() > 44 {
                                format!("{}…", &name[..43])
                            } else {
                                format!("{:<44}", name)
                            };
                            let toolset_tag = if toolset.is_empty() {
                                String::new()
                            } else {
                                format!(" [{toolset}]")
                            };
                            let current_tag = if is_current {
                                " ← current".to_string()
                            } else {
                                String::new()
                            };
                            ListItem::new(Line::from(vec![
                                Span::styled(
                                    if is_sel { "  ▶ " } else { "    " }.to_string(),
                                    Style::default().fg(if is_sel {
                                        RC::Green
                                    } else {
                                        RC::DarkGray
                                    }),
                                ),
                                Span::styled(
                                    name_trunc,
                                    Style::default()
                                        .fg(if is_sel { RC::White } else { RC::DarkGray })
                                        .add_modifier(if is_sel {
                                            Modifier::BOLD
                                        } else {
                                            Modifier::empty()
                                        }),
                                ),
                                Span::styled(toolset_tag, Style::default().fg(RC::DarkGray)),
                                Span::styled(current_tag, Style::default().fg(RC::Cyan)),
                            ]))
                        }
                    }
                })
                .collect()
        };

        // -- Draw helper
        let do_draw_model = |app_arc: &std::sync::Arc<parking_lot::Mutex<crate::ui::TuiApp>>,
                             list_pos: usize|
         -> Result<()> {
            let sel_model = model_at(list_pos);
            let title = format!(
                " Models  ↑↓/jk/PgUp/PgDn  Enter select  q cancel  [{}/{}] ",
                sel_model + 1,
                n_models
            );
            let items = build_items(list_pos, &current);
            let list = List::new(items).block(
                Block::default()
                    .borders(Borders::ALL)
                    .title(title)
                    .border_style(Style::default().fg(RC::Cyan)),
            );
            let mut ls = ListState::default().with_selected(Some(list_pos));
            let mut sb = ScrollbarState::new(disp_len).position(list_pos);
            let mut app = app_arc.lock();
            app.terminal.draw(|f| {
                let area = f.area();
                let chunks = Layout::default()
                    .direction(Direction::Horizontal)
                    .constraints([Constraint::Fill(1), Constraint::Length(1)])
                    .split(area);
                f.render_stateful_widget(list, chunks[0], &mut ls);
                f.render_stateful_widget(
                    Scrollbar::new(ScrollbarOrientation::VerticalRight),
                    chunks[1],
                    &mut sb,
                );
            })?;
            Ok(())
        };
        do_draw_model(&app_arc, list_pos)?;

        // -- Event loop
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
                        let sel = model_at(list_pos);
                        let (provider, _, id, _, _) = &models[sel];
                        if provider == "__custom__" || id.ends_with('/') {
                            // Freetext input via QuestionWidget
                            let prefix = if id.ends_with('/') && id.len() > 1 {
                                id.as_str()
                            } else {
                                ""
                            };
                            use crate::ui::question::{Question, QuestionOption};
                            let opts = vec![QuestionOption {
                                label: "Cancel".to_string(),
                                description: String::new(),
                            }];
                            let prompt = if prefix.is_empty() {
                                "Enter model ID (e.g. provider/model-name):".to_string()
                            } else {
                                format!("Enter model for {prefix}")
                            };
                            let q = Question {
                                header: "Custom model".to_string(),
                                text: prompt.clone(),
                                options: opts.clone(),
                                multi_select: false,
                                allow_other: true,
                                progress: None,
                            };
                            let ans = {
                                let mut app = app_arc.lock();
                                app.ask_question(&q)?
                            };
                            if let Some(a) = &ans {
                                let typed = a.as_str();
                                if !typed.is_empty() && typed != "Cancel" {
                                    let full = if prefix.is_empty() || typed.starts_with(prefix) {
                                        typed
                                    } else {
                                        format!("{prefix}{typed}")
                                    };
                                    break Some(full);
                                }
                            }
                            break None;
                        } else {
                            break Some(id.clone());
                        }
                    }

                    (KeyCode::Up, _) | (KeyCode::Char('k'), _) => {
                        list_pos = prev_pos(list_pos);
                        do_draw_model(&app_arc, list_pos)?;
                    }
                    (KeyCode::Down, _) | (KeyCode::Char('j'), _) => {
                        list_pos = next_pos(list_pos);
                        do_draw_model(&app_arc, list_pos)?;
                    }
                    (KeyCode::PageDown, _) => {
                        for _ in 0..10 {
                            list_pos = next_pos(list_pos);
                        }
                        do_draw_model(&app_arc, list_pos)?;
                    }
                    (KeyCode::PageUp, _) => {
                        for _ in 0..10 {
                            list_pos = prev_pos(list_pos);
                        }
                        do_draw_model(&app_arc, list_pos)?;
                    }
                    _ => {}
                }
            }
        };

        Ok(result)
    }
}
