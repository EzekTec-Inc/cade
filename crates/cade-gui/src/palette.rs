//! Slash-command palette for the cade-gui dashboard.
//!
//! This module is **pure Rust** — no browser dependencies.  It contains:
//!   - [`PaletteCmd`] — every command the palette can issue.
//!   - [`CMD_DEFS`] — static table of (trigger, description, category).
//!   - [`parse_palette_input`] — converts raw text into a `PaletteCmd`.
//!   - [`fuzzy_filter`] — ranks entries by how well they match a query.
//!
//! The wasm-side render loop (`app.rs`) opens the palette on `Ctrl+K` or
//! when the user types `/` at the start of a blank input, and dispatches
//! the resolved command.

// ── Command enum ────────────────────────────────────────────────────────

/// Every action the palette / slash-command system can trigger.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PaletteCmd {
    /// Show in-app help (list of available commands).
    Help,
    /// Clear the timeline (local only — does not delete server messages).
    Clear,
    /// Start a new conversation on the currently selected agent.
    New,
    /// Switch to a different agent: `/agent <name-or-id>`.
    Agent(String),
    /// Open the agents list panel.
    Agents,
    /// Open the memory viewer/editor panel.
    Memory,
    /// Run a text search: `/search <query>`.
    Search(String),
    /// Set the model on the current agent: `/model <model-id>`.
    Model(String),
    /// Show the context-window usage panel.
    Context,
    /// Show the usage/cost stats panel.
    Stats,
    /// Copy the last assistant message to clipboard.
    Copy,
    /// Open the artifacts browser.
    Artifacts,
    /// Open the checkpoints browser.
    Checkpoints,
    /// Open the skills browser.
    Skills,
    /// Open the MCP servers panel.
    Mcp,
    /// Log out and return to the login screen.
    Logout,
    /// Unknown command — carries the raw string for error display.
    Unknown(String),
}

// ── Static command table ─────────────────────────────────────────────────

/// Category tags — used to group entries in the palette UI.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CmdCategory {
    Navigation,
    Memory,
    Tools,
    Session,
    Display,
}

/// One entry in the static command table.
#[derive(Debug)]
pub struct CmdDef {
    /// Primary trigger (without the leading `/`).
    pub trigger: &'static str,
    /// Short, human-readable description shown in the palette row.
    pub description: &'static str,
    /// Optional argument placeholder shown in the palette hint.
    pub arg_hint: Option<&'static str>,
    pub category: CmdCategory,
}

/// All commands exposed in the palette, in display order.
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
        trigger: "logout",
        description: "Log out and return to login screen",
        arg_hint: None,
        category: CmdCategory::Session,
    },
];

// ── Parser ────────────────────────────────────────────────────────────────

/// Parse raw input into a [`PaletteCmd`].
///
/// Accepts both bare triggers (`help`) and slash-prefixed ones (`/help`).
/// Leading/trailing whitespace is stripped.
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
        other => PaletteCmd::Unknown(other.to_string()),
    }
}

// ── Fuzzy filter ──────────────────────────────────────────────────────────

/// A filtered + scored entry for palette display.
#[derive(Debug, Clone)]
pub struct FilteredCmd<'a> {
    pub def: &'a CmdDef,
    /// Score: higher = better match. `0` means query was empty (show all).
    pub score: usize,
}

/// Filter and rank `CMD_DEFS` by how well they match `query`.
///
/// Empty query → all entries at score 0 (preserve table order).
/// Otherwise uses a simple substring / initials heuristic:
///   - Full prefix match → 100
///   - Substring match in trigger → 50
///   - Substring match in description → 20
///   - No match → excluded
pub fn fuzzy_filter<'a>(query: &str) -> Vec<FilteredCmd<'a>> {
    let q = query.trim().to_lowercase();
    if q.is_empty() {
        return CMD_DEFS
            .iter()
            .map(|d| FilteredCmd { def: d, score: 0 })
            .collect();
    }

    let mut results: Vec<FilteredCmd<'_>> = CMD_DEFS
        .iter()
        .filter_map(|def| {
            let trigger = def.trigger.to_lowercase();
            let desc = def.description.to_lowercase();
            if trigger.starts_with(&q) {
                Some(FilteredCmd { def, score: 100 })
            } else if trigger.contains(&q) {
                Some(FilteredCmd { def, score: 50 })
            } else if desc.contains(&q) {
                Some(FilteredCmd { def, score: 20 })
            } else {
                None
            }
        })
        .collect();

    results.sort_by(|a, b| b.score.cmp(&a.score));
    results
}

// ── Tests ─────────────────────────────────────────────────────────────────
#[cfg(test)]
mod tests {
    use super::*;

    // -- parse_palette_input

    #[test]
    fn parse_slash_help() {
        assert_eq!(parse_palette_input("/help"), PaletteCmd::Help);
        assert_eq!(parse_palette_input("help"), PaletteCmd::Help);
        assert_eq!(parse_palette_input("?"), PaletteCmd::Help);
    }

    #[test]
    fn parse_slash_clear() {
        assert_eq!(parse_palette_input("/clear"), PaletteCmd::Clear);
    }

    #[test]
    fn parse_slash_new() {
        assert_eq!(parse_palette_input("/new"), PaletteCmd::New);
    }

    #[test]
    fn parse_slash_model_with_arg() {
        assert_eq!(
            parse_palette_input("/model claude-3-5-sonnet"),
            PaletteCmd::Model("claude-3-5-sonnet".to_string())
        );
    }

    #[test]
    fn parse_slash_model_empty_arg() {
        assert_eq!(
            parse_palette_input("/model"),
            PaletteCmd::Model(String::new())
        );
    }

    #[test]
    fn parse_slash_search_with_arg() {
        assert_eq!(
            parse_palette_input("/search foo bar"),
            PaletteCmd::Search("foo bar".to_string())
        );
    }

    #[test]
    fn parse_slash_logout_aliases() {
        assert_eq!(parse_palette_input("/logout"), PaletteCmd::Logout);
        assert_eq!(parse_palette_input("/exit"), PaletteCmd::Logout);
        assert_eq!(parse_palette_input("/quit"), PaletteCmd::Logout);
    }

    #[test]
    fn parse_slash_aliases_work() {
        assert_eq!(parse_palette_input("/mem"), PaletteCmd::Memory);
        assert_eq!(parse_palette_input("/ctx"), PaletteCmd::Context);
        assert_eq!(parse_palette_input("/usage"), PaletteCmd::Stats);
        assert_eq!(parse_palette_input("/cost"), PaletteCmd::Stats);
        assert_eq!(parse_palette_input("/undo"), PaletteCmd::Checkpoints);
        assert_eq!(parse_palette_input("/tree"), PaletteCmd::Checkpoints);
    }

    #[test]
    fn parse_slash_unknown() {
        match parse_palette_input("/zzzunknown") {
            PaletteCmd::Unknown(s) => assert_eq!(s, "zzzunknown"),
            other => panic!("expected Unknown, got {other:?}"),
        }
    }

    #[test]
    fn parse_trims_whitespace() {
        assert_eq!(parse_palette_input("  /new  "), PaletteCmd::New);
    }

    // -- fuzzy_filter

    #[test]
    fn fuzzy_empty_returns_all() {
        let results = fuzzy_filter("");
        assert_eq!(results.len(), CMD_DEFS.len());
        assert!(results.iter().all(|r| r.score == 0));
    }

    #[test]
    fn fuzzy_prefix_match_scores_100() {
        let results = fuzzy_filter("hel");
        let help = results.iter().find(|r| r.def.trigger == "help");
        assert!(help.is_some(), "help should match 'hel'");
        assert_eq!(help.unwrap().score, 100);
    }

    #[test]
    fn fuzzy_substring_scores_lower() {
        let results = fuzzy_filter("ory");
        let mem = results.iter().find(|r| r.def.trigger == "memory");
        assert!(mem.is_some(), "memory should match 'ory'");
        assert_eq!(mem.unwrap().score, 50);
    }

    #[test]
    fn fuzzy_desc_match() {
        // "clipboard" appears in copy's description
        let results = fuzzy_filter("clipboard");
        let copy = results.iter().find(|r| r.def.trigger == "copy");
        assert!(copy.is_some());
        assert_eq!(copy.unwrap().score, 20);
    }

    #[test]
    fn fuzzy_no_match_excluded() {
        let results = fuzzy_filter("zzzzznotacommand");
        assert!(results.is_empty());
    }

    #[test]
    fn fuzzy_results_sorted_by_score_desc() {
        let results = fuzzy_filter("s");
        for i in 1..results.len() {
            assert!(results[i - 1].score >= results[i].score);
        }
    }

    // -- CMD_DEFS completeness

    #[test]
    fn all_cmd_defs_have_nonempty_trigger_and_description() {
        for def in CMD_DEFS {
            assert!(!def.trigger.is_empty(), "empty trigger");
            assert!(!def.description.is_empty(), "empty description for {}", def.trigger);
        }
    }

    #[test]
    fn cmd_defs_triggers_are_unique() {
        let mut seen = std::collections::HashSet::new();
        for def in CMD_DEFS {
            assert!(seen.insert(def.trigger), "duplicate trigger: {}", def.trigger);
        }
    }
}
