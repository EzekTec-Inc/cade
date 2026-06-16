/// Full-screen `/menu` command browser for CADE.
///
/// Renders a navigable list of all slash commands grouped by category.
/// Returns the selected command string (e.g. "/agents") or None if cancelled.
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

// -- Command catalogue

struct CmdEntry {
    cmd: &'static str,
    desc: &'static str,
}

struct Section {
    name: &'static str,
    items: &'static [CmdEntry],
}

const SECTIONS: &[Section] = &[
    Section {
        name: "Session",
        items: &[
            CmdEntry {
                cmd: "/info",
                desc: "Agent, model, mode, cwd",
            },
            CmdEntry {
                cmd: "/agent",
                desc: "Show current agent name and ID",
            },
            CmdEntry {
                cmd: "/agents",
                desc: "List + switch agents  (r rename, d delete)",
            },
            CmdEntry {
                cmd: "/new-agent",
                desc: "Create a brand-new agent",
            },
            CmdEntry {
                cmd: "/rename",
                desc: "Rename current agent",
            },
            CmdEntry {
                cmd: "/delete",
                desc: "/delete <name>  — delete an agent by name/id",
            },
            CmdEntry {
                cmd: "/pin",
                desc: "Pin current agent to settings",
            },
            CmdEntry {
                cmd: "/new",
                desc: "Start a fresh conversation on the current agent",
            },
            CmdEntry {
                cmd: "/resume",
                desc: "Browse past conversations and switch to one",
            },
            CmdEntry {
                cmd: "/checkpoint",
                desc: "/checkpoint [label]  — save a checkpoint",
            },
            CmdEntry {
                cmd: "/tree",
                desc: "Browse and restore checkpoints  (fullscreen picker)",
            },
            CmdEntry {
                cmd: "/fork",
                desc: "/fork [label]  — create a new conversation from a checkpoint",
            },
            CmdEntry {
                cmd: "/artifacts",
                desc: "List stored artifacts (logs, diffs, reports)",
            },
        ],
    },
    Section {
        name: "Model & Mode",
        items: &[
            CmdEntry {
                cmd: "/theme",
                desc: "Change colorscheme  (/theme [name])",
            },
            CmdEntry {
                cmd: "/theme list",
                desc: "List all available themes (built-in + custom)",
            },
            CmdEntry {
                cmd: "/model",
                desc: "Interactive model picker  (or /model provider/name)",
            },
            CmdEntry {
                cmd: "/compaction-model",
                desc: "Set the cheaper model to use for history summarization",
            },
            CmdEntry {
                cmd: "/reasoning",
                desc: "Set reasoning effort (none, low, medium, high, xhigh)",
            },
            CmdEntry {
                cmd: "/toolset",
                desc: "/toolset [default|codex|gemini]",
            },
            CmdEntry {
                cmd: "/mode",
                desc: "Show or set permission mode",
            },
            CmdEntry {
                cmd: "/plan",
                desc: "Switch to read-only plan mode (write/exec tools blocked)",
            },
            CmdEntry {
                cmd: "/todo",
                desc: "Display the agent's scratchpad (.cade-todo.md)",
            },
            CmdEntry {
                cmd: "/todos",
                desc: "Toggle live plan panel (set via set_plan tool)",
            },
            CmdEntry {
                cmd: "/default",
                desc: "Return to default permission mode",
            },
            CmdEntry {
                cmd: "/yolo",
                desc: "Bypass permissions (auto-approve all tools)",
            },
            CmdEntry {
                cmd: "/approve-always",
                desc: "/approve-always <pattern>  — add allow rule",
            },
            CmdEntry {
                cmd: "/deny-always",
                desc: "/deny-always <pattern>   — add deny rule",
            },
            CmdEntry {
                cmd: "/permissions",
                desc: "Show current permission mode + rules",
            },
        ],
    },
    Section {
        name: "Memory",
        items: &[
            CmdEntry {
                cmd: "/memory",
                desc: "List all memory blocks",
            },
            CmdEntry {
                cmd: "/memory view",
                desc: "/memory view <label>  — show full block",
            },
            CmdEntry {
                cmd: "/memory set",
                desc: "/memory set <label> <value>",
            },
            CmdEntry {
                cmd: "/memory edit",
                desc: "/memory edit <label>  — interactive edit",
            },
            CmdEntry {
                cmd: "/memory delete",
                desc: "/memory delete <label>",
            },
            CmdEntry {
                cmd: "/memory history",
                desc: "/memory history <label>  — last 5 revisions",
            },
            CmdEntry {
                cmd: "/memory export",
                desc: "/memory export [path]  — dump memory as .md files for cade-rag-mcp",
            },
            CmdEntry {
                cmd: "/init",
                desc: "Analyse project + populate memory",
            },
            CmdEntry {
                cmd: "/remember",
                desc: "/remember [text]  — ask agent to update memory",
            },
        ],
    },
    Section {
        name: "Tools & Providers",
        items: &[
            CmdEntry {
                cmd: "/backend",
                desc: "/backend [local|docker|ssh|readonly|virtual]  — show or switch backend",
            },
            CmdEntry {
                cmd: "/link",
                desc: "Register + attach all tools to current agent",
            },
            CmdEntry {
                cmd: "/unlink",
                desc: "Detach all tools from current agent",
            },
            CmdEntry {
                cmd: "/mcp",
                desc: "Show MCP server status + tools",
            },
            CmdEntry {
                cmd: "/connect",
                desc: "Connect a new AI provider interactively",
            },
            CmdEntry {
                cmd: "/disconnect",
                desc: "/disconnect <name>  — remove a provider",
            },
            CmdEntry {
                cmd: "/providers",
                desc: "List configured providers",
            },
        ],
    },
    Section {
        name: "Web & Grounding",
        items: &[
            CmdEntry {
                cmd: "web_search",
                desc: "Agent tool: search the web (set BRAVE_SEARCH_API_KEY)",
            },
            CmdEntry {
                cmd: "fetch_doc",
                desc: "Agent tool: fetch and read a URL as clean text",
            },
            CmdEntry {
                cmd: "index_repository",
                desc: "Agent tool: index the repository for symbol search",
            },
        ],
    },
    Section {
        name: "Skills",
        items: &[
            CmdEntry {
                cmd: "/skills",
                desc: "Open interactive skills manager",
            },
            CmdEntry {
                cmd: "/skills new",
                desc: "/skills new <name>  — scaffold a new skill",
            },
            CmdEntry {
                cmd: "/skills reload",
                desc: "Reload skills from disk",
            },
            CmdEntry {
                cmd: "/teams",
                desc: "List available teams and their members",
            },
        ],
    },
    Section {
        name: "Diagnostics",
        items: &[
            CmdEntry {
                cmd: "/search",
                desc: "/search <query>  — search message history",
            },
            CmdEntry {
                cmd: "/compact",
                desc: "Manually consolidate dropped turns (alias: /consolidate)",
            },
            CmdEntry {
                cmd: "/context",
                desc: "Show context window usage bar chart",
            },
            CmdEntry {
                cmd: "/usage",
                desc: "Token usage this session",
            },
            CmdEntry {
                cmd: "/cost",
                desc: "Estimate API costs for this session",
            },
            CmdEntry {
                cmd: "/stats",
                desc: "Full session stats — tokens, tool calls, timing, per-model breakdown",
            },
            CmdEntry {
                cmd: "/stats model",
                desc: "Per-model detail: requests, input, cache, output per model",
            },
            CmdEntry {
                cmd: "/stream",
                desc: "Toggle streaming mode",
            },
            CmdEntry {
                cmd: "/hooks",
                desc: "Show configured hooks",
            },
            CmdEntry {
                cmd: "/feedback",
                desc: "Report issues / give feedback",
            },
        ],
    },
    Section {
        name: "Misc",
        items: &[
            CmdEntry {
                cmd: "/mouse",
                desc: "Toggle scroll-wheel capture on/off (text selection always works)",
            },
            CmdEntry {
                cmd: "/export",
                desc: "/export [file.json]  — export agent to JSON",
            },
            CmdEntry {
                cmd: "/clear",
                desc: "Clear screen + context window",
            },
            CmdEntry {
                cmd: "/logout",
                desc: "Clear stored API key and exit",
            },
            CmdEntry {
                cmd: "/help",
                desc: "Show this menu",
            },
        ],
    },
];

// -- Flat item list

#[derive(Clone)]
enum MenuItem {
    Header(String),
    Cmd { cmd: String, desc: String },
}

/// Commands that require specific capabilities.
fn cmd_required_capability(cmd: &str) -> Option<cade_core::capabilities::Capability> {
    use cade_core::capabilities::Capability;
    match cmd {
        "/agents" | "/teams" | "/reflect" | "/artifacts" => Some(Capability::Agentic),
        "/mcp" => Some(Capability::Mcp),
        "web_search" | "fetch_doc" => Some(Capability::Web),
        _ => None,
    }
}

fn build_flat_items_filtered(
    caps: Option<&cade_core::capabilities::CapabilitySet>,
) -> Vec<MenuItem> {
    let mut out = Vec::new();
    for section in SECTIONS {
        let mut section_items = Vec::new();
        for entry in section.items {
            let visible = match caps {
                None => true,
                Some(cs) => match cmd_required_capability(entry.cmd) {
                    None => true,
                    Some(cap) => cs.is_enabled(cap),
                },
            };
            if visible {
                section_items.push(MenuItem::Cmd {
                    cmd: entry.cmd.to_string(),
                    desc: entry.desc.to_string(),
                });
            }
        }
        if !section_items.is_empty() {
            out.push(MenuItem::Header(section.name.to_string()));
            out.extend(section_items);
        }
    }
    out
}

// -- Public entry point

/// Present the full-screen command browser. Returns the selected command
/// string (e.g. `"/agents"`) or `None` if the user cancels.
pub fn show_command_menu(
    terminal: &mut DefaultTerminal,
    colors: &ThemeColors,
) -> Result<Option<String>> {
    show_command_menu_with_caps(terminal, colors, None)
}

/// Present the full-screen command browser with type-to-filter.
///
/// - Type any text to filter commands by name or description in real time.
/// - ↑↓ arrows  always navigate; j/k navigate only when filter is empty.
/// - Backspace   removes the last filter character.
/// - Enter       runs the selected command.
/// - Esc         closes without running anything.
pub fn show_command_menu_with_caps(
    terminal: &mut DefaultTerminal,
    colors: &ThemeColors,
    caps: Option<&cade_core::capabilities::CapabilitySet>,
) -> Result<Option<String>> {
    let all_items = build_flat_items_filtered(caps);
    let mut query = String::new();

    // Build filtered list from a query string (section headers only shown if ≥1 child matches).
    let apply_filter = |q: &str, items: &[MenuItem]| -> Vec<MenuItem> {
        let q_low = q.to_lowercase();
        if q_low.is_empty() {
            return items.to_vec();
        }
        let mut out: Vec<MenuItem> = Vec::new();
        let mut i = 0;
        while i < items.len() {
            if matches!(&items[i], MenuItem::Header(_)) {
                let mut matching: Vec<MenuItem> = Vec::new();
                let mut j = i + 1;
                while j < items.len() {
                    if matches!(items[j], MenuItem::Header(_)) {
                        break;
                    }
                    if let MenuItem::Cmd { cmd, desc } = &items[j]
                        && (cmd.to_lowercase().contains(&q_low)
                            || desc.to_lowercase().contains(&q_low))
                    {
                        matching.push(items[j].clone());
                    }
                    j += 1;
                }
                if !matching.is_empty() {
                    out.push(items[i].clone());
                    out.extend(matching);
                }
                i = j;
            } else {
                i += 1;
            }
        }
        out
    };

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

    let mut items = apply_filter(&query, &all_items);
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
                            if is_sel { "  ▶ " } else { "    " }.to_string(),
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
        let query_display = query.clone();
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

            // Filter bar — shows placeholder when empty, query text when active
            let filter_line = Line::from(vec![
                Span::styled(" / ", colors.text_muted()),
                Span::styled(
                    if query_display.is_empty() {
                        "type to filter…".to_string()
                    } else {
                        query_display.clone()
                    },
                    Style::default().fg(if query_display.is_empty() {
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
                // j/k navigate only when filter empty (typing a filter uses these chars)
                KeyCode::Char('k') if query.is_empty() => {
                    sel = prev_sel(&items, sel);
                }
                KeyCode::Char('j') if query.is_empty() => {
                    sel = next_sel(&items, sel);
                }
                KeyCode::Up => {
                    sel = prev_sel(&items, sel);
                }
                KeyCode::Down => {
                    sel = next_sel(&items, sel);
                }
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
                    items = apply_filter(&query, &all_items);
                    sel = first_cmd(&items);
                }
                KeyCode::Char(c)
                    if !k.modifiers.contains(KeyModifiers::CONTROL)
                        && !k.modifiers.contains(KeyModifiers::ALT) =>
                {
                    query.push(c);
                    items = apply_filter(&query, &all_items);
                    sel = first_cmd(&items);
                }
                _ => {}
            }
        }
    }
}
