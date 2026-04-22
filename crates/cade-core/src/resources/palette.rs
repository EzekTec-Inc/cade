

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PaletteCmd {
    Help,
    Clear,
    New,
    Agent(String),
    Agents,
    Memory,
    Search(String),
    Model(String),
    Context,
    Stats,
    Copy,
    Artifacts,
    Checkpoints,
    Skills,
    Mcp,
    Logout,
    // Settings / config commands
    Providers,
    Permissions,
    Hooks,
    Theme,
    Pricing,
    Mode(String),
    Toolset(String),
    Backend(String),
    Unsupported(String),
    Unknown(String),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CmdCategory {
    Navigation,
    Memory,
    Tools,
    Session,
    Display,
}

#[derive(Debug)]
pub struct CmdDef {
    pub trigger: &'static str,
    pub description: &'static str,
    pub arg_hint: Option<&'static str>,
    pub category: CmdCategory,
}

pub const CMD_DEFS: &[CmdDef] = &[
    CmdDef {
        trigger: "help",
        description: "Show available commands",
        arg_hint: None,
        category: CmdCategory::Navigation,
    },
    CmdDef {
        trigger: "new",
        description: "Start a new conversation",
        arg_hint: None,
        category: CmdCategory::Session,
    },
    CmdDef {
        trigger: "clear",
        description: "Clear the timeline (local only)",
        arg_hint: None,
        category: CmdCategory::Display,
    },
    CmdDef {
        trigger: "agents",
        description: "Browse all agents",
        arg_hint: None,
        category: CmdCategory::Navigation,
    },
    CmdDef {
        trigger: "model",
        description: "Set the agent model",
        arg_hint: Some("<model-id>"),
        category: CmdCategory::Session,
    },
    CmdDef {
        trigger: "memory",
        description: "View / edit agent memory",
        arg_hint: None,
        category: CmdCategory::Memory,
    },
    CmdDef {
        trigger: "search",
        description: "Search conversation history",
        arg_hint: Some("<query>"),
        category: CmdCategory::Memory,
    },
    CmdDef {
        trigger: "context",
        description: "Show context-window usage",
        arg_hint: None,
        category: CmdCategory::Display,
    },
    CmdDef {
        trigger: "stats",
        description: "Show token usage / cost stats",
        arg_hint: None,
        category: CmdCategory::Display,
    },
    CmdDef {
        trigger: "copy",
        description: "Copy last assistant message to clipboard",
        arg_hint: None,
        category: CmdCategory::Display,
    },
    CmdDef {
        trigger: "artifacts",
        description: "Browse stored artifacts",
        arg_hint: None,
        category: CmdCategory::Tools,
    },
    CmdDef {
        trigger: "checkpoints",
        description: "Browse / restore checkpoints",
        arg_hint: None,
        category: CmdCategory::Tools,
    },
    CmdDef {
        trigger: "skills",
        description: "Browse loaded skills",
        arg_hint: None,
        category: CmdCategory::Tools,
    },
    CmdDef {
        trigger: "mcp",
        description: "MCP server connections",
        arg_hint: None,
        category: CmdCategory::Tools,
    },
    CmdDef {
        trigger: "providers",
        description: "Manage AI providers (OpenAI, Anthropic, etc)",
        arg_hint: None,
        category: CmdCategory::Session,
    },
    CmdDef {
        trigger: "permissions",
        description: "Manage tool approval permissions",
        arg_hint: None,
        category: CmdCategory::Session,
    },
    CmdDef {
        trigger: "hooks",
        description: "Manage custom session hooks",
        arg_hint: None,
        category: CmdCategory::Session,
    },
    CmdDef {
        trigger: "theme",
        description: "Change UI color theme",
        arg_hint: None,
        category: CmdCategory::Display,
    },
    CmdDef {
        trigger: "pricing",
        description: "Manage token pricing rules",
        arg_hint: None,
        category: CmdCategory::Session,
    },
    CmdDef {
        trigger: "mode",
        description: "Set permission mode (auto/plan/edits/yolo)",
        arg_hint: Some("<mode>"),
        category: CmdCategory::Session,
    },
    CmdDef {
        trigger: "toolset",
        description: "Change active toolset (default/codex/gemini)",
        arg_hint: Some("<toolset>"),
        category: CmdCategory::Tools,
    },
    CmdDef {
        trigger: "backend",
        description: "Change execution backend (local/docker/ssh)",
        arg_hint: None,
        category: CmdCategory::Session,
    },
    CmdDef {
        trigger: "logout",
        description: "Log out and return to login screen",
        arg_hint: None,
        category: CmdCategory::Session,
    },
];

pub fn parse_palette_input(raw: &str) -> PaletteCmd {
    let trimmed = raw.trim().trim_start_matches('/');
    let mut parts = trimmed.splitn(2, ' ');
    let trigger = parts.next().unwrap_or("").trim();
    let arg = parts.next().unwrap_or("").trim().to_string();

    match trigger {
        "help" | "?" | "menu" => PaletteCmd::Help,
        "clear" => PaletteCmd::Clear,
        "new" => PaletteCmd::New,
        "agent" => PaletteCmd::Agent(arg),
        "agents" | "agent-list" => PaletteCmd::Agents,
        "memory" | "mem" => PaletteCmd::Memory,
        "search" | "s" => PaletteCmd::Search(arg),
        "model" | "m" => PaletteCmd::Model(arg),
        "context" | "ctx" => PaletteCmd::Context,
        "stats" | "usage" | "cost" => PaletteCmd::Stats,
        "copy" | "cp" => PaletteCmd::Copy,
        "artifacts" | "artifact" => PaletteCmd::Artifacts,
        "checkpoints" | "checkpoint" | "undo" | "tree" => PaletteCmd::Checkpoints,
        "skills" | "skill" => PaletteCmd::Skills,
        "mcp" => PaletteCmd::Mcp,
        "logout" | "exit" | "quit" => PaletteCmd::Logout,
        "resume" => PaletteCmd::Unsupported("resume".into()),
        "rename" => PaletteCmd::Unsupported("rename".into()),
        "delete" | "del" | "rm-agent" => PaletteCmd::Unsupported("delete".into()),
        "new-agent" => PaletteCmd::Unsupported("new-agent".into()),
        "pin" => PaletteCmd::Unsupported("pin".into()),
        "init" => PaletteCmd::Unsupported("init".into()),
        "info" => PaletteCmd::Unsupported("info".into()),
        "feedback" => PaletteCmd::Unsupported("feedback".into()),
        "plan" => PaletteCmd::Unsupported("plan".into()),
        "yolo" => PaletteCmd::Unsupported("yolo".into()),
        "default" | "normal" => PaletteCmd::Unsupported("default".into()),
        "mode" => PaletteCmd::Mode(arg),
        "todos" => PaletteCmd::Unsupported("todos".into()),
        "todo" => PaletteCmd::Unsupported("todo".into()),
        "reasoning" => PaletteCmd::Unsupported("reasoning".into()),
        "stream" => PaletteCmd::Unsupported("stream".into()),
        "mouse" | "select" => PaletteCmd::Unsupported("mouse".into()),
        "toolset" => PaletteCmd::Toolset(arg),
        "theme" => PaletteCmd::Theme,
        "providers" | "provider-list" => PaletteCmd::Providers,
        "connect" => PaletteCmd::Unsupported("connect".into()),
        "disconnect" => PaletteCmd::Unsupported("disconnect".into()),
        "permissions" => PaletteCmd::Permissions,
        "hooks" => PaletteCmd::Hooks,
        "subagents" | "agents-list" => PaletteCmd::Unsupported("subagents".into()),
        "mcp-save" => PaletteCmd::Unsupported("mcp-save".into()),
        "link" => PaletteCmd::Unsupported("link".into()),
        "unlink" => PaletteCmd::Unsupported("unlink".into()),
        "approve-always" => PaletteCmd::Unsupported("approve-always".into()),
        "deny-always" => PaletteCmd::Unsupported("deny-always".into()),
        "reflect" => PaletteCmd::Unsupported("reflect".into()),
        "summarize" | "summary" => PaletteCmd::Unsupported("summarize".into()),
        "export" => PaletteCmd::Unsupported("export".into()),
        "remember" => PaletteCmd::Unsupported("remember".into()),
        "pricing" => PaletteCmd::Pricing,
        "backend" => PaletteCmd::Backend(arg),
        "compaction-model" => PaletteCmd::Unsupported("compaction-model".into()),
        "debug-last" | "debug_last" => PaletteCmd::Unsupported("debug-last".into()),
        "fork" => PaletteCmd::Unsupported("fork".into()),
        other => PaletteCmd::Unknown(other.to_string()),
    }
}

pub fn fuzzy_score(query: &str, label: &str, description: &str, section: &str) -> Option<i32> {
    if query.is_empty() {
        return Some(0);
    }

    let label_lower = label.to_lowercase();
    let desc_lower = description.to_lowercase();
    let section_lower = section.to_lowercase();

    let mut score: i32 = 0;
    let mut matched = false;

    if label_lower.starts_with(query) {
        score += 100;
        matched = true;
    } else if label_lower.starts_with('/') && label_lower[1..].starts_with(query) {
        score += 95;
        matched = true;
    } else if label_lower.contains(query) {
        score += 50;
        matched = true;
    } else if is_subsequence(query, &label_lower) {
        score += 30;
        score += word_boundary_bonus(query, &label_lower);
        matched = true;
    }

    if desc_lower.contains(query) {
        score += 20;
        matched = true;
    }

    if section_lower.contains(query) {
        score += 10;
        matched = true;
    }

    if matched {
        score += (50i32).saturating_sub(label.len() as i32);
        Some(score)
    } else {
        None
    }
}

fn is_subsequence(needle: &str, haystack: &str) -> bool {
    let mut it = haystack.chars();
    for nc in needle.chars() {
        loop {
            match it.next() {
                Some(hc) if hc == nc => break,
                Some(_) => continue,
                None => return false,
            }
        }
    }
    true
}

fn word_boundary_bonus(query: &str, label: &str) -> i32 {
    let boundaries: Vec<usize> = std::iter::once(0)
        .chain(
            label
                .char_indices()
                .filter(|(_, c)| *c == '-' || *c == '_' || *c == ' ' || *c == '/')
                .map(|(i, _)| i + 1),
        )
        .collect();

    let mut bonus = 0i32;
    let mut qi = 0;
    let query_chars: Vec<char> = query.chars().collect();

    for &bi in &boundaries {
        if qi >= query_chars.len() {
            break;
        }
        if let Some(lc) = label.chars().nth(bi)
            && lc == query_chars[qi] {
                bonus += 5;
                qi += 1;
            }
    }
    bonus
}
