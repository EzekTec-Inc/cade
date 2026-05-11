use crate::colors::ThemeColorsExt;
use crate::{Result, colors::ThemeColors, overlay};
use cade_core::skills::Skill;
use crossterm::event::{self, Event, KeyCode, KeyEventKind, KeyModifiers};
use crossterm::{
    execute,
    terminal::{EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode},
};
use ratatui::{
    DefaultTerminal,
    layout::{Constraint, Direction, Layout, Rect},
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Cell, Paragraph, Row, Table, TableState, Wrap},
};
use std::path::Path;

pub enum SkillsAction {
    Reload,
}

const HINT: &str = " ↑↓/j k  •  PgUp/PgDn scroll  •  e edit  •  r reload  •  Esc close ";

pub fn show_skills_manager(
    terminal: &mut DefaultTerminal,
    skills: Vec<Skill>,
    colors: &ThemeColors,
) -> Result<Option<SkillsAction>> {
    if skills.is_empty() {
        return show_empty_state(terminal, colors);
    }

    let mut selected_idx: usize = 0;
    let mut preview_scroll: u16 = 0;
    let mut result: Option<SkillsAction> = None;

    loop {
        if selected_idx >= skills.len() {
            selected_idx = skills.len().saturating_sub(1);
        }

        terminal.draw(|f| {
            let area = f.area();
            let inner_shell = overlay::render_overlay_shell(f, area, "Skills", colors);

            let top_chunks = Layout::default()
                .direction(Direction::Horizontal)
                .constraints([Constraint::Percentage(40), Constraint::Percentage(60)].as_ref())
                .split(Rect {
                    x: inner_shell.x,
                    y: inner_shell.y,
                    width: inner_shell.width,
                    height: inner_shell.height.saturating_sub(1), // leave room for footer
                });

            // -- Left pane (table)
            let rows: Vec<Row> = skills
                .iter()
                .enumerate()
                .map(|(i, s)| {
                    let is_sel = i == selected_idx;
                    let row_style = if is_sel {
                        Style::default()
                            .bg(colors.c_bg_surface1())
                            .add_modifier(Modifier::BOLD)
                    } else {
                        Style::default()
                    };
                    let scope_str = s.scope.to_string();
                    Row::new(vec![
                        Cell::from(Span::styled(scope_str, colors.text_primary())),
                        Cell::from(Span::styled(s.name.clone(), colors.text_primary())),
                        Cell::from(Span::styled(
                            s.category.clone().unwrap_or_default(),
                            colors.text_muted(),
                        )),
                    ])
                    .style(row_style)
                })
                .collect();

            let table = Table::new(
                rows,
                [
                    Constraint::Length(10),
                    Constraint::Length(25),
                    Constraint::Min(15),
                ],
            )
            .header(
                Row::new(vec!["Scope", "Name", "Category"]).style(
                    Style::default()
                        .fg(colors.c_primary())
                        .add_modifier(Modifier::BOLD),
                ),
            )
            .block(
                Block::default()
                    .borders(Borders::ALL)
                    .border_type(colors.c_border_style())
                    .title(" Skills ")
                    .border_style(colors.border_accent()),
            );

            let mut ts = TableState::default().with_selected(Some(selected_idx));
            f.render_stateful_widget(table, top_chunks[0], &mut ts);

            // -- Right pane (preview)
            let preview_text = if selected_idx < skills.len() {
                let s = &skills[selected_idx];
                let meta = format!(
                    "ID: {}\nDescription: {}\nTags: {}\nTriggers: {}\nPath: {}\n\n",
                    s.id,
                    s.description,
                    s.tags.join(", "),
                    s.triggers.join(", "),
                    s.path.display(),
                );
                format!("{}{}", meta, s.body)
            } else {
                String::new()
            };

            let preview = Paragraph::new(preview_text)
                .wrap(Wrap { trim: false })
                .scroll((preview_scroll, 0))
                .block(
                    Block::default()
                        .borders(Borders::ALL)
                        .border_type(colors.c_border_style())
                        .title(" Preview ")
                        .border_style(colors.border_accent()),
                );
            f.render_widget(preview, top_chunks[1]);

            // Single canonical footer
            let hint_area = Rect {
                x: inner_shell.x,
                y: inner_shell.y + inner_shell.height.saturating_sub(1),
                width: inner_shell.width,
                height: 1,
            };
            overlay::render_overlay_hint(f, hint_area, HINT, colors);
        })?;

        if !event::poll(std::time::Duration::from_millis(200))? {
            continue;
        }
        if let Ok(Event::Key(key)) = event::read() {
            if key.kind != KeyEventKind::Press {
                continue;
            }
            match (key.code, key.modifiers) {
                (KeyCode::Esc, _)
                | (KeyCode::Char('q'), KeyModifiers::NONE)
                | (KeyCode::Char('c'), KeyModifiers::CONTROL) => break,

                (KeyCode::Up, _) | (KeyCode::Char('k'), _) | (KeyCode::BackTab, _) => {
                    selected_idx = selected_idx.saturating_sub(1);
                    preview_scroll = 0;
                }
                (KeyCode::Down, _) | (KeyCode::Char('j'), _) | (KeyCode::Tab, _) => {
                    if selected_idx + 1 < skills.len() {
                        selected_idx += 1;
                        preview_scroll = 0;
                    }
                }

                (KeyCode::PageDown, _) | (KeyCode::Char('d'), KeyModifiers::CONTROL) => {
                    preview_scroll = preview_scroll.saturating_add(5);
                }
                (KeyCode::PageUp, _) | (KeyCode::Char('u'), KeyModifiers::CONTROL) => {
                    preview_scroll = preview_scroll.saturating_sub(5);
                }

                (KeyCode::Char('r'), KeyModifiers::NONE) => {
                    result = Some(SkillsAction::Reload);
                    break;
                }

                (KeyCode::Char('e'), KeyModifiers::NONE) | (KeyCode::Enter, KeyModifiers::NONE) => {
                    if let Some(skill) = skills.get(selected_idx) {
                        launch_editor(terminal, &skill.path)?;
                        result = Some(SkillsAction::Reload);
                        break;
                    }
                }

                _ => {}
            }
        }
    }

    Ok(result)
}

fn show_empty_state(
    terminal: &mut DefaultTerminal,
    colors: &ThemeColors,
) -> Result<Option<SkillsAction>> {
    loop {
        terminal.draw(|f| {
            let area = f.area();
            let inner_shell = overlay::render_overlay_shell(f, area, "Skills", colors);
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
                        "  No skills found.",
                        overlay::overlay_muted_style(colors),
                    )),
                    Line::from(""),
                    Line::from(Span::styled(
                        "  /skills new <name>  to scaffold your first skill",
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
            overlay::render_overlay_hint(f, hint_area, " Esc close ", colors);
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
                _ => {}
            }
        }
    }
}

/// Suspend the ratatui alt-screen, launch `$EDITOR` (or a sensible fallback) on
/// the skill file, then restore the TUI. We do not surface the editor's exit
/// status — the caller triggers a reload unconditionally so edits are picked up.
fn launch_editor(terminal: &mut DefaultTerminal, path: &Path) -> Result<()> {
    let editor = std::env::var("VISUAL")
        .or_else(|_| std::env::var("EDITOR"))
        .unwrap_or_else(|_| {
            // Platform-reasonable fallback
            if cfg!(windows) {
                "notepad".to_string()
            } else {
                "vi".to_string()
            }
        });

    // Suspend TUI so the editor owns the terminal.
    disable_raw_mode()?;
    execute!(std::io::stdout(), LeaveAlternateScreen)?;

    let status = std::process::Command::new(&editor).arg(path).status();

    // Restore TUI regardless of editor outcome.
    enable_raw_mode()?;
    execute!(std::io::stdout(), EnterAlternateScreen)?;
    terminal.clear()?;

    // Surface spawn errors as a no-op; non-zero exit codes are fine (user may
    // have :q'd). We can't tracing!() here (cade-tui doesn't depend on tracing),
    // and writing to the restored TUI would clobber the redrawn frame.
    let _ = status;

    Ok(())
}
