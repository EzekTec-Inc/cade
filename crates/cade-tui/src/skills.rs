use crate::Result;
use cade_core::skills::Skill;
use crossterm::event::{self, Event, KeyCode, KeyModifiers};
use ratatui::{
    DefaultTerminal, Frame,
    layout::{Constraint, Layout, Rect},
    style::{Color as RC, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Paragraph},
};

pub enum SkillsAction {
    Reload,
}

#[derive(Debug, Clone, PartialEq)]
enum SkillsMode {
    List,
    Detail,
    Edit,
}

pub fn show_skills_manager(
    terminal: &mut DefaultTerminal,
    skills: Vec<Skill>,
) -> Result<Option<SkillsAction>> {
    let mut mode = SkillsMode::List;
    let mut cursor = 0;
    let mut list_scroll = 0;
    let mut detail_scroll = 0;

    let mut edit_fields = vec![String::new(); 6];
    let mut field_cursor = 0;
    let mut field_pos = 0;
    let mut dirty = false;
    let mut message: Option<String> = None;

    let load_edit_fields = |skills: &[Skill],
                            cursor: usize,
                            edit_fields: &mut Vec<String>,
                            field_cursor: &mut usize,
                            field_pos: &mut usize,
                            dirty: &mut bool| {
        if let Some(s) = skills.get(cursor) {
            *edit_fields = vec![
                s.name.clone(),
                s.description.clone(),
                s.category.clone().unwrap_or_default(),
                s.tags.join(", "),
                s.triggers.join(", "),
                s.body.clone(),
            ];
            *field_cursor = 0;
            *field_pos = 0;
            *dirty = false;
        }
    };

    loop {
        terminal.draw(|f| {
            let area = f.area();
            f.render_widget(
                Paragraph::new("").style(Style::default().bg(RC::Rgb(10, 10, 18))),
                area,
            );
            let inner = Rect {
                x: area.x + 1,
                y: area.y + 1,
                width: area.width.saturating_sub(2),
                height: area.height.saturating_sub(3),
            };
            let hint_area = Rect {
                x: area.x + 2,
                y: area.y + area.height.saturating_sub(2),
                width: area.width.saturating_sub(4),
                height: 1,
            };

            match mode {
                SkillsMode::List => {
                    f.render_widget(
                        Paragraph::new(Line::from(vec![
                            Span::styled(
                                "  ◆ Skills  ",
                                Style::default().fg(RC::Cyan).add_modifier(Modifier::BOLD),
                            ),
                            Span::styled(
                                format!("({} loaded)", skills.len()),
                                Style::default().fg(RC::DarkGray),
                            ),
                        ])),
                        Rect {
                            x: inner.x,
                            y: inner.y,
                            width: inner.width,
                            height: 1,
                        },
                    );

                    if skills.is_empty() {
                        f.render_widget(
                            Paragraph::new(vec![
                                Line::from(Span::styled(
                                    "  No skills found.",
                                    Style::default().fg(RC::DarkGray),
                                )),
                                Line::from(""),
                                Line::from(Span::styled(
                                    "  /skills create <name>  to scaffold your first skill",
                                    Style::default().fg(RC::DarkGray),
                                )),
                            ]),
                            Rect {
                                x: inner.x,
                                y: inner.y + 2,
                                width: inner.width,
                                height: 3,
                            },
                        );
                        render_hint(f, "Esc close", hint_area);
                        return;
                    }

                    let card_h: u16 = 5;
                    let cards_area = Rect {
                        x: inner.x,
                        y: inner.y + 1,
                        width: inner.width,
                        height: inner.height.saturating_sub(1),
                    };
                    let visible = (cards_area.height / card_h) as usize;
                    let end = (list_scroll + visible).min(skills.len());

                    let chunks =
                        Layout::vertical((list_scroll..end).map(|_| Constraint::Length(card_h)))
                            .split(cards_area);

                    for (i, idx) in (list_scroll..end).enumerate() {
                        if let Some(skill) = skills.get(idx) {
                            let is_sel = idx == cursor;
                            let bg = if is_sel {
                                RC::Rgb(30, 30, 45)
                            } else {
                                RC::Rgb(15, 15, 22)
                            };

                            let mut block = Block::default().style(Style::default().bg(bg));
                            if is_sel {
                                block = block
                                    .borders(Borders::LEFT)
                                    .border_style(Style::default().fg(RC::Green));
                            } else {
                                block = block.padding(ratatui::widgets::Padding::new(1, 0, 0, 0));
                            }

                            let cat = skill.category.as_deref().unwrap_or("general");
                            let p = Paragraph::new(vec![
                                Line::from(vec![
                                    Span::styled(
                                        format!("  {}  ", skill.name),
                                        Style::default()
                                            .fg(if is_sel { RC::White } else { RC::Gray })
                                            .add_modifier(Modifier::BOLD),
                                    ),
                                    Span::styled(
                                        format!("[{}] ", skill.id),
                                        Style::default().fg(RC::DarkGray),
                                    ),
                                    Span::styled(
                                        format!("({})", skill.scope),
                                        Style::default().fg(RC::Magenta),
                                    ),
                                ]),
                                Line::from(Span::styled(
                                    format!("  {}", skill.description),
                                    Style::default().fg(RC::Gray),
                                )),
                                Line::from(Span::styled(
                                    format!(
                                        "  Cat: {cat}  |  Tags: {}  |  Triggers: {}",
                                        skill.tags.join(", "),
                                        skill.triggers.join(", ")
                                    ),
                                    Style::default().fg(RC::DarkGray),
                                )),
                                Line::from(""), // spacer
                            ])
                            .block(block);
                            f.render_widget(p, chunks[i]);
                        }
                    }
                    render_hint(
                        f,
                        "Enter view · e edit · ↑↓ navigate · Esc close",
                        hint_area,
                    );
                }
                SkillsMode::Detail | SkillsMode::Edit => {
                    let is_edit = mode == SkillsMode::Edit;
                    let skill = if let Some(s) = skills.get(cursor) {
                        s
                    } else {
                        return;
                    };

                    let title_area = Rect {
                        x: inner.x,
                        y: inner.y,
                        width: inner.width,
                        height: 1,
                    };
                    let msg = if let Some(m) = &message {
                        format!("  {m}")
                    } else if is_edit {
                        if dirty { "  [Unsaved changes]" } else { "" }.to_string()
                    } else {
                        "".to_string()
                    };

                    let mode_str = if is_edit { "Edit" } else { "Detail" };
                    f.render_widget(
                        Paragraph::new(Line::from(vec![
                            Span::styled(
                                format!("  ◆ Skill {mode_str}: {}  ", skill.name),
                                Style::default().fg(RC::Cyan).add_modifier(Modifier::BOLD),
                            ),
                            Span::styled(
                                format!("[{}] ", skill.id),
                                Style::default().fg(RC::DarkGray),
                            ),
                            Span::styled(
                                format!("({})", skill.scope),
                                Style::default().fg(RC::Magenta),
                            ),
                            Span::styled(
                                msg,
                                Style::default().fg(if dirty { RC::Yellow } else { RC::Green }),
                            ),
                        ])),
                        title_area,
                    );

                    let meta_h = 4;
                    let meta_area = Rect {
                        x: inner.x,
                        y: inner.y + 2,
                        width: inner.width,
                        height: meta_h,
                    };
                    let body_area = Rect {
                        x: inner.x,
                        y: inner.y + 2 + meta_h + 1,
                        width: inner.width,
                        height: inner.height.saturating_sub(3 + meta_h),
                    };

                    // Render edit fields or detail view
                    let labels = ["Name:", "Desc:", "Category:", "Tags:", "Triggers:"];
                    let mut lines = Vec::new();
                    for (fi, label) in labels.iter().enumerate() {
                        let is_active = is_edit && field_cursor == fi;
                        let val = if is_edit {
                            &edit_fields[fi]
                        } else {
                            match fi {
                                0 => &skill.name,
                                1 => &skill.description,
                                2 => skill.category.as_deref().unwrap_or(""),
                                3 => &skill.tags.join(", "),
                                4 => &skill.triggers.join(", "),
                                _ => "",
                            }
                        };
                        let val_style = if is_active {
                            Style::default()
                                .fg(RC::White)
                                .add_modifier(Modifier::UNDERLINED)
                        } else {
                            Style::default().fg(RC::Gray)
                        };
                        lines.push(Line::from(vec![
                            Span::styled(
                                format!("  {:<10} ", label),
                                Style::default().fg(RC::DarkGray),
                            ),
                            if is_active {
                                let before = &val[..field_pos.min(val.len())];
                                let cursor_char = val[field_pos..].chars().next().unwrap_or(' ');
                                let after_start = field_pos
                                    + cursor_char
                                        .len_utf8()
                                        .min(val.len().saturating_sub(field_pos));
                                let after = &val[after_start..];
                                Span::styled(
                                    format!("{before}\x1b[7m{cursor_char}\x1b[27m{after}"),
                                    val_style,
                                )
                            } else {
                                Span::styled(val.to_string(), val_style)
                            },
                        ]));
                    }
                    f.render_widget(Paragraph::new(lines), meta_area);

                    let is_body_active = is_edit && field_cursor == 5;
                    f.render_widget(
                        Paragraph::new(Line::from(vec![Span::styled(
                            "  Body:",
                            Style::default().fg(RC::DarkGray),
                        )])),
                        Rect {
                            x: inner.x,
                            y: inner.y + 2 + meta_h,
                            width: inner.width,
                            height: 1,
                        },
                    );

                    let body_str = if is_edit {
                        &edit_fields[5]
                    } else {
                        &skill.body
                    };
                    let mut body_lines = Vec::new();
                    for (bi, line) in body_str.split('\n').enumerate() {
                        if is_body_active {
                            let cursor_line = body_str[..field_pos.min(body_str.len())]
                                .split('\n')
                                .count()
                                .saturating_sub(1);
                            if bi == cursor_line {
                                let line_start = body_str[..field_pos.min(body_str.len())]
                                    .rfind('\n')
                                    .map(|i| i + 1)
                                    .unwrap_or(0);
                                let col = field_pos.saturating_sub(line_start).min(line.len());
                                let before = &line[..col];
                                let cursor_char = line[col..].chars().next().unwrap_or(' ');
                                let after_start = col
                                    + cursor_char.len_utf8().min(line.len().saturating_sub(col));
                                let after = &line[after_start..];
                                body_lines.push(Line::from(vec![
                                    Span::raw("    "),
                                    Span::styled(
                                        format!("{before}\x1b[7m{cursor_char}\x1b[27m{after}"),
                                        Style::default().fg(RC::White),
                                    ),
                                ]));
                            } else {
                                body_lines.push(Line::from(format!("    {line}")));
                            }
                        } else {
                            body_lines.push(Line::from(format!("    {line}")));
                        }
                    }

                    let scroll_offset = if is_body_active {
                        let cursor_line = body_str[..field_pos.min(body_str.len())]
                            .split('\n')
                            .count()
                            .saturating_sub(1);
                        let max_visible = body_area.height as usize;
                        if cursor_line < detail_scroll {
                            detail_scroll = cursor_line;
                        } else if cursor_line >= detail_scroll + max_visible {
                            detail_scroll = cursor_line.saturating_sub(max_visible) + 1;
                        }
                        detail_scroll as u16
                    } else {
                        detail_scroll as u16
                    };

                    f.render_widget(
                        Paragraph::new(body_lines).scroll((scroll_offset, 0)),
                        body_area,
                    );

                    if is_edit {
                        render_hint(f, "Tab next field · Ctrl+S save · Esc cancel", hint_area);
                    } else {
                        render_hint(f, "e edit · ↑↓ scroll · Esc back", hint_area);
                    }
                }
            }
        })?;

        if !event::poll(std::time::Duration::from_millis(50))? {
            continue;
        }
        if let Event::Key(key) = event::read()? {
            message = None;
            match mode {
                SkillsMode::List => match (key.code, key.modifiers) {
                    (KeyCode::Char('j'), _) | (KeyCode::Down, _) => {
                        if cursor + 1 < skills.len() {
                            cursor += 1;
                        }
                        let visible = 8usize;
                        if cursor >= list_scroll + visible {
                            list_scroll += 1;
                        }
                    }
                    (KeyCode::Char('k'), _) | (KeyCode::Up, _) => {
                        if cursor > 0 {
                            cursor -= 1;
                        }
                        if cursor < list_scroll {
                            list_scroll = list_scroll.saturating_sub(1);
                        }
                    }
                    (KeyCode::Enter, _) => {
                        if !skills.is_empty() {
                            mode = SkillsMode::Detail;
                            detail_scroll = 0;
                        }
                    }
                    (KeyCode::Char('e'), _) => {
                        if !skills.is_empty() {
                            load_edit_fields(
                                &skills,
                                cursor,
                                &mut edit_fields,
                                &mut field_cursor,
                                &mut field_pos,
                                &mut dirty,
                            );
                            mode = SkillsMode::Edit;
                        }
                    }
                    (KeyCode::Esc, _) | (KeyCode::Char('q'), _) => return Ok(None),
                    _ => {}
                },
                SkillsMode::Detail => match (key.code, key.modifiers) {
                    (KeyCode::Char('j'), _) | (KeyCode::Down, _) => detail_scroll += 1,
                    (KeyCode::Char('k'), _) | (KeyCode::Up, _) => {
                        detail_scroll = detail_scroll.saturating_sub(1)
                    }
                    (KeyCode::Char('e'), _) => {
                        load_edit_fields(
                            &skills,
                            cursor,
                            &mut edit_fields,
                            &mut field_cursor,
                            &mut field_pos,
                            &mut dirty,
                        );
                        mode = SkillsMode::Edit;
                    }
                    (KeyCode::Esc, _) => mode = SkillsMode::List,
                    _ => {}
                },
                SkillsMode::Edit => match (key.code, key.modifiers) {
                    (KeyCode::Char('s'), KeyModifiers::CONTROL) => {
                        if let Some(s) = skills.get(cursor) {
                            if let Err(e) = cade_core::skills::write_skill_to_disk(s, &edit_fields)
                            {
                                message = Some(format!("Failed to save: {e}"));
                            } else {
                                return Ok(Some(SkillsAction::Reload));
                            }
                        }
                    }
                    (KeyCode::Esc, _) => {
                        dirty = false;
                        mode = SkillsMode::Detail;
                    }
                    (KeyCode::Tab, _) => {
                        field_cursor = (field_cursor + 1) % 6;
                        field_pos = edit_fields.get(field_cursor).map(|f| f.len()).unwrap_or(0);
                    }
                    (KeyCode::BackTab, _) => {
                        field_cursor = (field_cursor + 5) % 6;
                        field_pos = edit_fields.get(field_cursor).map(|f| f.len()).unwrap_or(0);
                    }
                    (KeyCode::Enter, _) => {
                        if field_cursor == 5 {
                            let pos = field_pos;
                            if let Some(f) = edit_fields.get_mut(5) {
                                let pos = pos.min(f.len());
                                f.insert(pos, '\n');
                                field_pos = pos + 1;
                                dirty = true;
                            }
                        } else {
                            field_cursor = (field_cursor + 1) % 6;
                            field_pos = edit_fields.get(field_cursor).map(|f| f.len()).unwrap_or(0);
                        }
                    }
                    (KeyCode::Left, _) => {
                        if field_pos > 0 {
                            field_pos -= 1;
                        }
                    }
                    (KeyCode::Right, _) => {
                        let max = edit_fields.get(field_cursor).map(|f| f.len()).unwrap_or(0);
                        if field_pos < max {
                            field_pos += 1;
                        }
                    }
                    (KeyCode::Up, _) if field_cursor == 5 => {
                        detail_scroll = detail_scroll.saturating_sub(1)
                    }
                    (KeyCode::Down, _) if field_cursor == 5 => detail_scroll += 1,
                    (KeyCode::Backspace, _) => {
                        let pos = field_pos;
                        if pos > 0 {
                            if let Some(f) = edit_fields.get_mut(field_cursor) {
                                let new_pos = f[..pos]
                                    .char_indices()
                                    .next_back()
                                    .map(|(i, _)| i)
                                    .unwrap_or(0);
                                f.drain(new_pos..pos);
                                field_pos = new_pos;
                                dirty = true;
                            }
                        }
                    }
                    (KeyCode::Char(c), m)
                        if m == KeyModifiers::NONE || m == KeyModifiers::SHIFT =>
                    {
                        let pos = field_pos;
                        if let Some(f) = edit_fields.get_mut(field_cursor) {
                            let pos = pos.min(f.len());
                            f.insert(pos, c);
                            field_pos = pos + c.len_utf8();
                            dirty = true;
                        }
                    }
                    _ => {}
                },
            }
        }
    }
}

fn render_hint(frame: &mut Frame, hint: &str, hint_area: Rect) {
    frame.render_widget(
        Paragraph::new(Span::styled(hint, Style::default().fg(RC::DarkGray))),
        hint_area,
    );
}
