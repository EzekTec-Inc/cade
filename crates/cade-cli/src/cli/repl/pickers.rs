use super::AgentPickerResult;
use super::Repl;
use crate::Result;
use cade_agent::agent::client::AgentState;
use crossterm::event::KeyCode;

impl Repl {
    /// `/resume` conversation picker — full-screen on TuiApp terminal.
    ///
    /// Keys: ↑/↓ move · Enter select · d delete · Esc/q cancel.
    /// Returns the picked conversation JSON, or None if cancelled.
    pub(crate) async fn conversation_picker(
        &self,
        app_arc: std::sync::Arc<std::sync::Mutex<crate::ui::TuiApp>>,
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
            let mut app = app_arc.lock().expect("lock poisoned");
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
                            let mut app = app_arc.lock().expect("lock poisoned");
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
            let mut app = app_arc.lock().expect("lock poisoned");
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
        app_arc: std::sync::Arc<std::sync::Mutex<crate::ui::TuiApp>>,
        agents: &mut [AgentState],
    ) -> Result<Option<AgentPickerResult>> {
        use crossterm::event::{self, Event, KeyCode, KeyEventKind};
        use ratatui::{
            style::{Color as RC, Modifier, Style},
            text::{Line, Span},
            widgets::{Block, Borders, List, ListItem, ListState},
        };
        use std::collections::HashSet;

        if agents.is_empty() {
            return Ok(None);
        }

        let current = self.agent_id();
        let total = agents.len();
        let mut selected: usize = agents.iter().position(|a| a.id == current).unwrap_or(0);
        let mut marked: HashSet<usize> = HashSet::new();

        let build_items = |agents: &[AgentState],
                           sel: usize,
                           marked: &HashSet<usize>,
                           current: &str|
         -> Vec<ListItem<'static>> {
            agents
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
                    ListItem::new(Line::from(vec![
                        Span::styled(
                            if is_sel { " ▶ " } else { "   " }.to_string(),
                            Style::default().fg(if is_sel { RC::Green } else { RC::DarkGray }),
                        ),
                        Span::styled(
                            if is_marked { "☑ " } else { "☐ " }.to_string(),
                            Style::default().fg(if is_marked { RC::Yellow } else { RC::DarkGray }),
                        ),
                        Span::styled(
                            format!("{:<32}", a.name),
                            Style::default()
                                .fg(if is_sel { RC::White } else { RC::DarkGray })
                                .add_modifier(if is_sel {
                                    Modifier::BOLD
                                } else {
                                    Modifier::empty()
                                }),
                        ),
                        Span::styled(short_id, Style::default().fg(RC::DarkGray)),
                        Span::styled(
                            if is_active {
                                "  ← active".to_string()
                            } else {
                                String::new()
                            },
                            Style::default().fg(RC::Cyan),
                        ),
                    ]))
                })
                .collect()
        };

        let do_draw = |app_arc: &std::sync::Arc<std::sync::Mutex<crate::ui::TuiApp>>,
                       agents: &[AgentState],
                       sel: usize,
                       marked: &HashSet<usize>,
                       current: &str|
         -> Result<()> {
            let mut app = app_arc.lock().expect("lock poisoned");
            let items = build_items(agents, sel, marked, current);
            let n = marked.len();
            let hint = if n == 0 {
                " ↑↓/jk  Space mark  r rename  d delete  Enter switch  q cancel ".to_string()
            } else {
                format!(" [{n} marked]  d delete all  q cancel ")
            };
            let mut ls = ListState::default().with_selected(Some(sel));
            app.terminal.draw(|f| {
                let area = f.area();
                let block = Block::default()
                    .borders(Borders::ALL)
                    .title(format!(" Agents {hint}"))
                    .border_style(Style::default().fg(RC::Cyan));
                let list = List::new(items).block(block);
                f.render_stateful_widget(list, area, &mut ls);
            })?;
            Ok(())
        };

        do_draw(&app_arc, agents, selected, &marked, &current)?;

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
                        if marked.is_empty() {
                            let a = agents[selected].clone();
                            if a.id != current {
                                break Some(AgentPickerResult::Switch(a));
                            }
                        }
                    }

                    (KeyCode::Char(' '), _) => {
                        if marked.contains(&selected) {
                            marked.remove(&selected);
                        } else {
                            marked.insert(selected);
                        }
                    }

                    (KeyCode::Char('d'), _) | (KeyCode::Delete, _) => {
                        let targets: Vec<usize> = if marked.is_empty() {
                            vec![selected]
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
                            let mut app = app_arc.lock().expect("lock poisoned");
                            let r = app.ask_question(&q)?;
                            app.scroll = 0;
                            let _ = app.draw();
                            matches!(&r, Some(a) if a.as_str().starts_with("Yes"))
                        };
                        if confirmed {
                            let to_delete: Vec<AgentState> =
                                targets.iter().map(|&i| agents[i].clone()).collect();
                            break Some(AgentPickerResult::DeleteMany(to_delete));
                        }
                    }

                    (KeyCode::Char('r'), _) => {
                        let a = agents[selected].clone();
                        // Collect new name via QuestionWidget (allow_other = freetext)
                        use crate::ui::question::{Question, QuestionOption};
                        let opts = vec![QuestionOption {
                            label: "Keep current name".to_string(),
                            description: String::new(),
                        }];
                        let q = Question {
                            header: "Rename agent".to_string(),
                            text: format!("New name for '{}':", a.name),
                            options: opts.clone(),
                            multi_select: false,
                            allow_other: true,
                            progress: None,
                        };
                        let ans = {
                            let mut app = app_arc.lock().expect("lock poisoned");
                            app.ask_question(&q)?
                        };
                        if let Some(answer) = &ans {
                            let new_name = answer.as_str();
                            if !new_name.is_empty() && new_name != "Keep current name" {
                                break Some(AgentPickerResult::Rename { agent: a, new_name });
                            }
                        }
                    }

                    (KeyCode::Up, _) | (KeyCode::Char('k'), _) => {
                        selected = if selected == 0 {
                            total - 1
                        } else {
                            selected - 1
                        };
                    }
                    (KeyCode::Down, _) | (KeyCode::Char('j'), _) => {
                        selected = (selected + 1) % total;
                    }
                    _ => {}
                }
                do_draw(&app_arc, agents, selected, &marked, &current)?;
            }
        };

        Ok(result)
    }

    /// Interactive model picker — full-screen on TuiApp terminal.
    /// Returns the selected model string or None if cancelled.
    pub(crate) async fn interactive_model_picker(
        &self,
        app_arc: std::sync::Arc<std::sync::Mutex<crate::ui::TuiApp>>,
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
            let mut app = app_arc.lock().expect("lock poisoned");
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
                let mut app = app_arc.lock().expect("lock poisoned");
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
            let mut app = app_arc.lock().expect("lock poisoned");
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
        let do_draw_model = |app_arc: &std::sync::Arc<std::sync::Mutex<crate::ui::TuiApp>>,
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
            let mut app = app_arc.lock().expect("lock poisoned");
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
                                let mut app = app_arc.lock().expect("lock poisoned");
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

    /// Interactive reasoning tier picker — full-screen on TuiApp terminal.
    /// Returns the selected reasoning tier string or None if cancelled.
    pub(crate) async fn interactive_reasoning_picker(
        &self,
        app_arc: std::sync::Arc<std::sync::Mutex<crate::ui::TuiApp>>,
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
            .expect("lock poisoned")
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

        let do_draw_tier = |app_arc: &std::sync::Arc<std::sync::Mutex<crate::ui::TuiApp>>,
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

            let mut app = app_arc.lock().expect("lock poisoned");
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
