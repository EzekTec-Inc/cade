/// Full-screen `/menu` command browser for CADE.
///
/// Renders a navigable list of all slash commands grouped by category.
/// Returns the selected command string (e.g. "/agents") or None if cancelled.
use crate::Result;
use crossterm::event::{self, Event, KeyCode};
use ratatui::{
    DefaultTerminal,
    layout::{Constraint, Layout},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, List, ListItem, ListState, Paragraph},
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
                desc: "/checkpoint [label]  — save a checkpoint of the current state",
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
                cmd: "/model",
                desc: "Interactive model picker  (or /model provider/name)",
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
                desc: "Switch to read-only plan mode",
            },
            CmdEntry {
                cmd: "/todos",
                desc: "Toggle visibility of the active plan / checklist",
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
                desc: "/backend [local|docker|ssh|readonly]  — show or switch execution backend",
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
                desc: "Agent tool: search the web (set BRAVE_SEARCH_API_KEY for best results)",
            },
            CmdEntry {
                cmd: "fetch_doc",
                desc: "Agent tool: fetch and read a URL as clean text",
            },
            CmdEntry {
                cmd: "index_repository",
                desc: "Agent tool: index codebase for symbol_search / find_references",
            },
        ],
    },
    Section {
        name: "Skills & Subagents",
        items: &[
            CmdEntry {
                cmd: "/skills",
                desc: "List loaded skills",
            },
            CmdEntry {
                cmd: "/skills create",
                desc: "/skills create <name>  — scaffold a new skill",
            },
            CmdEntry {
                cmd: "/skills show",
                desc: "/skills show <id>  — show skill detail",
            },
            CmdEntry {
                cmd: "/skills reload",
                desc: "Reload skills from disk",
            },
            CmdEntry {
                cmd: "/subagents",
                desc: "List available subagent definitions",
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
                cmd: "/context",
                desc: "Show current context window usage",
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
                cmd: "/copy",
                desc: "Toggle copy mode (disables mouse scroll for text selection)",
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

fn build_flat_items() -> Vec<MenuItem> {
    let mut out = Vec::new();
    for section in SECTIONS {
        out.push(MenuItem::Header(section.name.to_string()));
        for entry in section.items {
            out.push(MenuItem::Cmd {
                cmd: entry.cmd.to_string(),
                desc: entry.desc.to_string(),
            });
        }
    }
    out
}

fn first_cmd_idx(items: &[MenuItem]) -> usize {
    items
        .iter()
        .position(|i| matches!(i, MenuItem::Cmd { .. }))
        .unwrap_or(0)
}

fn next_cmd(items: &[MenuItem], pos: usize) -> usize {
    let n = items.len();
    let mut p = (pos + 1) % n;
    for _ in 0..n {
        if matches!(items[p], MenuItem::Cmd { .. }) {
            return p;
        }
        p = (p + 1) % n;
    }
    pos
}

fn prev_cmd(items: &[MenuItem], pos: usize) -> usize {
    let n = items.len();
    let mut p = if pos == 0 { n - 1 } else { pos - 1 };
    for _ in 0..n {
        if matches!(items[p], MenuItem::Cmd { .. }) {
            return p;
        }
        p = if p == 0 { n - 1 } else { p - 1 };
    }
    pos
}

// -- Public entry point

/// Present the full-screen command browser. Returns the selected command
/// string (e.g. `"/agents"`) or `None` if the user cancels.
pub fn show_command_menu(terminal: &mut DefaultTerminal) -> Result<Option<String>> {
    let items = build_flat_items();
    let mut sel = first_cmd_idx(&items);

    loop {
        let list_items: Vec<ListItem<'static>> = items
            .iter()
            .enumerate()
            .map(|(i, item)| match item {
                MenuItem::Header(name) => ListItem::new(Line::from(vec![
                    Span::raw("  "),
                    Span::styled(
                        name.clone(),
                        Style::default()
                            .fg(Color::Yellow)
                            .add_modifier(Modifier::BOLD),
                    ),
                ])),
                MenuItem::Cmd { cmd, desc } => {
                    let is_sel = i == sel;
                    ListItem::new(Line::from(vec![
                        Span::styled(
                            if is_sel { "  ▶ " } else { "    " }.to_string(),
                            Style::default().fg(if is_sel {
                                Color::Green
                            } else {
                                Color::DarkGray
                            }),
                        ),
                        Span::styled(
                            format!("{cmd:<22}"),
                            Style::default()
                                .fg(if is_sel {
                                    Color::White
                                } else {
                                    Color::Rgb(160, 160, 230)
                                })
                                .add_modifier(if is_sel {
                                    Modifier::BOLD
                                } else {
                                    Modifier::empty()
                                }),
                        ),
                        Span::styled(desc.clone(), Style::default().fg(Color::DarkGray)),
                    ]))
                }
            })
            .collect();

        let detail = if let Some(MenuItem::Cmd { cmd, desc }) = items.get(sel) {
            format!("  {cmd}  —  {desc}")
        } else {
            String::new()
        };

        let mut ls = ListState::default().with_selected(Some(sel));
        terminal.draw(|f| {
            let area = f.area();
            let chunks = Layout::vertical([
                Constraint::Fill(1),
                Constraint::Length(1),
                Constraint::Length(1),
            ])
            .split(area);

            let block = Block::default()
                .borders(Borders::ALL)
                .title(" CADE Commands  ↑↓/jk navigate · Enter run · Esc close ")
                .border_style(Style::default().fg(Color::Cyan));
            let list = List::new(list_items).block(block);
            f.render_stateful_widget(list, chunks[0], &mut ls);
            f.render_widget(
                Paragraph::new(Span::styled(detail, Style::default().fg(Color::White))),
                chunks[1],
            );
            f.render_widget(
                Paragraph::new(Span::styled(
                    "  Enter to run · Esc to close",
                    Style::default().fg(Color::DarkGray),
                )),
                chunks[2],
            );
        })?;

        if !event::poll(std::time::Duration::from_millis(200))? {
            continue;
        }
        if let Event::Key(k) = event::read()? { match k.code {
            KeyCode::Esc | KeyCode::Char('q') => return Ok(None),
            KeyCode::Enter => {
                if let Some(MenuItem::Cmd { cmd, .. }) = items.get(sel) {
                    return Ok(Some(cmd.clone()));
                }
            }
            KeyCode::Up | KeyCode::Char('k') => {
                sel = prev_cmd(&items, sel);
            }
            KeyCode::Down | KeyCode::Char('j') => {
                sel = next_cmd(&items, sel);
            }
            KeyCode::PageUp => {
                for _ in 0..8 {
                    sel = prev_cmd(&items, sel);
                }
            }
            KeyCode::PageDown => {
                for _ in 0..8 {
                    sel = next_cmd(&items, sel);
                }
            }
            _ => {}
        } }
    }
}
