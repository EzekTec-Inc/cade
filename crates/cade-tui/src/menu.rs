//! Full-screen command browser. The host supplies the executable catalogue.
use crate::colors::ThemeColorsExt;
use crate::{Result, colors::ThemeColors, overlay};
use crossterm::event::{self, Event, KeyCode, KeyEventKind, KeyModifiers};
use ratatui::{
    DefaultTerminal,
    layout::{Constraint, Layout},
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::{Block, List, ListItem, ListState, Paragraph},
};

pub struct CommandMenuEntry {
    pub command: String,
    pub description: String,
    pub section: &'static str,
}

#[derive(Clone)]
enum MenuItem {
    Header(String),
    Cmd { cmd: String, desc: String },
}

fn filtered_items(entries: &[CommandMenuEntry], query: &str) -> Vec<MenuItem> {
    let query = query.to_lowercase();
    let mut items = Vec::new();
    let mut section = None;
    for entry in entries {
        if !entry.command.to_lowercase().contains(&query)
            && !entry.description.to_lowercase().contains(&query)
        {
            continue;
        }
        if section != Some(entry.section) {
            section = Some(entry.section);
            items.push(MenuItem::Header(entry.section.into()));
        }
        items.push(MenuItem::Cmd {
            cmd: entry.command.clone(),
            desc: entry.description.clone(),
        });
    }
    items
}

/// Present the host's command catalogue with type-to-filter. Capability filtering
/// and command identity belong to the host, not a second TUI command registry.
pub fn show_command_menu(
    terminal: &mut DefaultTerminal,
    colors: &ThemeColors,
    entries: &[CommandMenuEntry],
) -> Result<Option<String>> {
    let mut query = String::new();
    let first_cmd = |items: &[MenuItem]| -> usize {
        items
            .iter()
            .position(|i| matches!(i, MenuItem::Cmd { .. }))
            .unwrap_or(0)
    };
    let next_sel = |items: &[MenuItem], pos: usize| -> usize {
        let n = items.len();
        if n == 0 {
            return 0;
        }
        let mut p = (pos + 1) % n;
        for _ in 0..n {
            if matches!(items[p], MenuItem::Cmd { .. }) {
                return p;
            }
            p = (p + 1) % n;
        }
        pos
    };
    let prev_sel = |items: &[MenuItem], pos: usize| -> usize {
        let n = items.len();
        if n == 0 {
            return 0;
        }
        let mut p = if pos == 0 { n - 1 } else { pos - 1 };
        for _ in 0..n {
            if matches!(items[p], MenuItem::Cmd { .. }) {
                return p;
            }
            p = if p == 0 { n - 1 } else { p - 1 };
        }
        pos
    };
    let mut items = filtered_items(entries, &query);
    let mut sel = first_cmd(&items);

    loop {
        let list_items: Vec<ListItem<'static>> = items
            .iter()
            .enumerate()
            .map(|(i, item)| match item {
                MenuItem::Header(name) => {
                    let rule_len = 40usize.saturating_sub(name.len() + 3);
                    ListItem::new(Line::from(vec![
                        Span::raw("  "),
                        Span::styled(name.clone(), overlay::overlay_section_style(colors)),
                        Span::styled(
                            format!(" {}", "─".repeat(rule_len)),
                            Style::default().fg(colors.c_border_base()),
                        ),
                    ]))
                }
                MenuItem::Cmd { cmd, desc } => {
                    let is_sel = i == sel;
                    ListItem::new(Line::from(vec![
                        Span::styled(
                            if is_sel { "  ▶ " } else { "    " },
                            Style::default().fg(if is_sel {
                                colors.c_primary()
                            } else {
                                colors.c_text_muted()
                            }),
                        ),
                        Span::styled(
                            format!("{cmd:<22}"),
                            Style::default()
                                .fg(if is_sel {
                                    colors.c_text_primary()
                                } else {
                                    colors.c_primary()
                                })
                                .add_modifier(if is_sel {
                                    Modifier::BOLD
                                } else {
                                    Modifier::empty()
                                }),
                        ),
                        Span::styled(desc.clone(), overlay::overlay_muted_style(colors)),
                    ]))
                }
            })
            .collect();

        let detail = if let Some(MenuItem::Cmd { cmd, desc }) = items.get(sel) {
            Some((cmd.clone(), desc.clone()))
        } else {
            None
        };
        let mut ls = ListState::default().with_selected(Some(sel));
        terminal.draw(|f| {
            let area = f.area();
            let inner = overlay::render_overlay_shell(
                f,
                area,
                "CADE Commands  ·  type to filter  ·  ↑↓ navigate  ·  Enter run  ·  Esc close",
                colors,
            );
            let [filter_area, list_area, detail_area, hint_area] = Layout::vertical([
                Constraint::Length(1),
                Constraint::Fill(1),
                Constraint::Length(1),
                Constraint::Length(1),
            ])
            .areas(inner);
            let filter_line = Line::from(vec![
                Span::styled(" / ", colors.text_muted()),
                Span::styled(
                    if query.is_empty() {
                        "type to filter…".into()
                    } else {
                        query.clone()
                    },
                    Style::default().fg(if query.is_empty() {
                        colors.c_text_muted()
                    } else {
                        colors.c_text_primary()
                    }),
                ),
            ]);
            f.render_widget(Paragraph::new(filter_line), filter_area);
            let list = List::new(list_items)
                .block(Block::default().style(Style::default().bg(colors.c_bg_surface2())))
                .highlight_style(overlay::overlay_selected_style(colors));
            f.render_stateful_widget(list, list_area, &mut ls);
            let detail_line = if let Some((cmd, desc)) = &detail {
                Line::from(vec![
                    Span::raw(" "),
                    Span::styled(cmd.clone(), overlay::overlay_badge_style(colors)),
                    Span::raw(" "),
                    Span::styled(desc.clone(), colors.text_primary()),
                ])
            } else {
                Line::from("")
            };
            f.render_widget(Paragraph::new(detail_line), detail_area);
            overlay::render_overlay_hint(
                f,
                hint_area,
                "Enter to run  ·  Backspace to clear filter  ·  Esc to close",
                colors,
            );
        })?;

        if !event::poll(std::time::Duration::from_millis(200))? {
            continue;
        }
        if let Event::Key(k) = event::read()? {
            if k.kind != KeyEventKind::Press {
                continue;
            }
            match k.code {
                KeyCode::Esc => return Ok(None),
                KeyCode::Enter => {
                    if let Some(MenuItem::Cmd { cmd, .. }) = items.get(sel) {
                        return Ok(Some(cmd.clone()));
                    }
                }
                KeyCode::Char('k') if query.is_empty() => sel = prev_sel(&items, sel),
                KeyCode::Char('j') if query.is_empty() => sel = next_sel(&items, sel),
                KeyCode::Up => sel = prev_sel(&items, sel),
                KeyCode::Down => sel = next_sel(&items, sel),
                KeyCode::PageUp => {
                    for _ in 0..8 {
                        sel = prev_sel(&items, sel);
                    }
                }
                KeyCode::PageDown => {
                    for _ in 0..8 {
                        sel = next_sel(&items, sel);
                    }
                }
                KeyCode::Backspace => {
                    query.pop();
                    items = filtered_items(entries, &query);
                    sel = first_cmd(&items);
                }
                KeyCode::Char(c)
                    if !k.modifiers.contains(KeyModifiers::CONTROL)
                        && !k.modifiers.contains(KeyModifiers::ALT) =>
                {
                    query.push(c);
                    items = filtered_items(entries, &query);
                    sel = first_cmd(&items);
                }
                _ => {}
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn menu_filters_host_commands_and_alias_descriptions_without_empty_sections() {
        let entries = [
            CommandMenuEntry {
                command: "/info".into(),
                description: "Show agent".into(),
                section: "Session",
            },
            CommandMenuEntry {
                command: "/tree".into(),
                description: "Checkpoints (aliases: /timeline)".into(),
                section: "History",
            },
        ];
        let items = filtered_items(&entries, "TIMELINE");
        assert_eq!(items.len(), 2);
        assert!(matches!(&items[0], MenuItem::Header(section) if section == "History"));
        assert!(matches!(&items[1], MenuItem::Cmd { cmd, .. } if cmd == "/tree"));
        assert!(filtered_items(&entries, "missing").is_empty());
    }
}
