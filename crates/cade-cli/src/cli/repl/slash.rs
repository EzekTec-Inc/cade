use crate::ui::SlashCommandDef;

/// All known slash-command trigger words with a one-line description.
/// Used by the TUI autocomplete overlay so users can Tab-complete every
/// registered command instead of only a hardcoded subset.
pub(crate) fn all_slash_command_defs() -> Vec<SlashCommandDef> {
    vec![
        SlashCommandDef {
            name: "help".into(),
            description: "Show available commands".into(),
        },
        SlashCommandDef {
            name: "exit".into(),
            description: "Exit CADE".into(),
        },
        SlashCommandDef {
            name: "clear".into(),
            description: "Clear the timeline".into(),
        },
        SlashCommandDef {
            name: "agent".into(),
            description: "Switch to a different agent".into(),
        },
        SlashCommandDef {
            name: "info".into(),
            description: "Show session and agent info".into(),
        },
        SlashCommandDef {
            name: "new".into(),
            description: "Start a new conversation".into(),
        },
        SlashCommandDef {
            name: "new-agent".into(),
            description: "Create a brand-new agent".into(),
        },
        SlashCommandDef {
            name: "pin".into(),
            description: "Pin or unpin a memory block".into(),
        },
        SlashCommandDef {
            name: "agents".into(),
            description: "Browse all agents".into(),
        },
        SlashCommandDef {
            name: "resume".into(),
            description: "Pick a conversation to resume".into(),
        },
        SlashCommandDef {
            name: "init".into(),
            description: "Analyse project + populate memory".into(),
        },
        SlashCommandDef {
            name: "remember".into(),
            description: "Ask agent to update memory".into(),
        },
        SlashCommandDef {
            name: "memory".into(),
            description: "View or manage memory blocks".into(),
        },
        SlashCommandDef {
            name: "search".into(),
            description: "Search conversation history".into(),
        },
        SlashCommandDef {
            name: "feedback".into(),
            description: "Send feedback to the developers".into(),
        },
        SlashCommandDef {
            name: "skills".into(),
            description: "List, reload or create skills".into(),
        },
        SlashCommandDef {
            name: "subagents".into(),
            description: "Browse available subagent modes".into(),
        },
        SlashCommandDef {
            name: "teams".into(),
            description: "Manage agent teams".into(),
        },
        SlashCommandDef {
            name: "providers".into(),
            description: "Show all configured AI providers".into(),
        },
        SlashCommandDef {
            name: "connect".into(),
            description: "Connect a new AI provider interactively".into(),
        },
        SlashCommandDef {
            name: "disconnect".into(),
            description: "Remove a provider by name".into(),
        },
        SlashCommandDef {
            name: "approve-always".into(),
            description: "Always approve a tool by name".into(),
        },
        SlashCommandDef {
            name: "deny-always".into(),
            description: "Always deny a tool by name".into(),
        },
        SlashCommandDef {
            name: "permissions".into(),
            description: "Manage tool approval permissions".into(),
        },
        SlashCommandDef {
            name: "hooks".into(),
            description: "Manage custom session hooks".into(),
        },
        SlashCommandDef {
            name: "rename".into(),
            description: "Rename the current agent".into(),
        },
        SlashCommandDef {
            name: "theme".into(),
            description: "Change UI colour theme".into(),
        },
        SlashCommandDef {
            name: "toolset".into(),
            description: "Switch active toolset".into(),
        },
        SlashCommandDef {
            name: "delete".into(),
            description: "Delete the current agent".into(),
        },
        SlashCommandDef {
            name: "yolo".into(),
            description: "Enable bypass-permissions mode".into(),
        },
        SlashCommandDef {
            name: "plan".into(),
            description: "Show or switch to plan mode".into(),
        },
        SlashCommandDef {
            name: "todos".into(),
            description: "List current todos".into(),
        },
        SlashCommandDef {
            name: "todo".into(),
            description: "Manage a specific todo".into(),
        },
        SlashCommandDef {
            name: "default".into(),
            description: "Return to auto mode".into(),
        },
        SlashCommandDef {
            name: "mode".into(),
            description: "Set permission mode".into(),
        },
        SlashCommandDef {
            name: "model".into(),
            description: "Show or switch active LLM".into(),
        },
        SlashCommandDef {
            name: "reasoning".into(),
            description: "Set reasoning effort".into(),
        },
        SlashCommandDef {
            name: "mcp".into(),
            description: "Show MCP server status".into(),
        },
        SlashCommandDef {
            name: "mcp-save".into(),
            description: "Save MCP server configuration".into(),
        },
        SlashCommandDef {
            name: "link".into(),
            description: "Register + attach all tools".into(),
        },
        SlashCommandDef {
            name: "unlink".into(),
            description: "Detach all tools".into(),
        },
        SlashCommandDef {
            name: "logout".into(),
            description: "Log out and return to login".into(),
        },
        SlashCommandDef {
            name: "stream".into(),
            description: "Toggle token streaming".into(),
        },
        SlashCommandDef {
            name: "usage".into(),
            description: "Show token usage for this session".into(),
        },
        SlashCommandDef {
            name: "stats".into(),
            description: "Show per-model token statistics".into(),
        },
        SlashCommandDef {
            name: "export".into(),
            description: "Export the current agent to JSON".into(),
        },
        SlashCommandDef {
            name: "context".into(),
            description: "Show context window usage".into(),
        },
        SlashCommandDef {
            name: "debug-last".into(),
            description: "Dump the last assistant message".into(),
        },
        SlashCommandDef {
            name: "cost".into(),
            description: "Show session cost breakdown".into(),
        },
        SlashCommandDef {
            name: "pricing".into(),
            description: "Manage token pricing rules".into(),
        },
        SlashCommandDef {
            name: "checkpoint".into(),
            description: "Create a working-tree checkpoint".into(),
        },
        SlashCommandDef {
            name: "undo".into(),
            description: "Undo the last checkpoint".into(),
        },
        SlashCommandDef {
            name: "tree".into(),
            description: "Browse and restore checkpoints".into(),
        },
        SlashCommandDef {
            name: "fork".into(),
            description: "Fork a conversation from a checkpoint".into(),
        },
        SlashCommandDef {
            name: "artifacts".into(),
            description: "Browse stored artifacts".into(),
        },
        SlashCommandDef {
            name: "reflect".into(),
            description: "Extract memory from conversation".into(),
        },
        SlashCommandDef {
            name: "summarize".into(),
            description: "Show session summary".into(),
        },
        SlashCommandDef {
            name: "compact".into(),
            description: "Trigger session consolidation".into(),
        },
        SlashCommandDef {
            name: "compaction-model".into(),
            description: "Set the compaction model".into(),
        },
        SlashCommandDef {
            name: "backend".into(),
            description: "Show or switch execution backend".into(),
        },
        SlashCommandDef {
            name: "marketplace".into(),
            description: "Browse the plugin marketplace".into(),
        },
        SlashCommandDef {
            name: "reload".into(),
            description: "Reload Lua UI plugins".into(),
        },
        SlashCommandDef {
            name: "update".into(),
            description: "Check for CADE updates".into(),
        },
        SlashCommandDef {
            name: "trust".into(),
            description: "Trust the current project directory".into(),
        },
        SlashCommandDef {
            name: "mouse".into(),
            description: "Toggle mouse capture for native text selection".into(),
        },
        SlashCommandDef {
            name: "gui".into(),
            description: "Open the Web GUI dashboard in browser".into(),
        },
        SlashCommandDef {
            name: "dashboard".into(),
            description: "Open the Web GUI dashboard in browser".into(),
        },
    ]
}

#[derive(Debug)]
pub(crate) enum SlashCmd {
    Help,
    /// Invoke a loaded skill by its id (e.g. /commit → RunSkill("commit", Some("custom prompt")))
    RunSkill(String, Option<String>),
    Exit,
    Clear,
    Agent,
    Info,
    Model(String),
    Reasoning(String),
    New,      // new conversation on same agent
    NewAgent, // create a brand-new agent
    Pin,
    Agents,
    Resume, // conversation picker
    Init,
    Remember(String),
    Memory,
    Search(String),
    Feedback,
    /// /skills [list|create <name>|show <id>|reload]
    Skills(Option<String>),
    Subagents,
    Teams,
    Approvals,
    Approve(String),
    Deny(String),
    Steer(String),
    Providers,
    Connect(Option<String>),
    Disconnect(String),
    ApproveAlways(String),
    DenyAlways(String),
    Permissions,
    Hooks,
    Rename(String),
    Theme(Option<String>),
    Toolset(Option<String>),
    Delete(Option<String>),
    Yolo,
    Plan,
    Todos,
    Todo,
    Default,
    Mode(Option<String>),
    Mcp,
    McpSave(String),
    Link(Option<String>),
    Unlink(Option<String>),
    Logout,
    Stream,
    Usage,
    /// /stats [model]
    Stats(Option<String>),
    /// Export the current agent to a JSON file: /export [output.json]
    Export(Option<String>),
    /// Show current context window usage.
    Context,
    /// Dump the last assistant message as stored on the server.
    DebugLast,
    /// Show session cost breakdown (tokens × pricing).
    Cost,
    /// Configure or sync pricing rules.
    Pricing(Option<String>),
    /// Create a checkpoint of the current working-tree state.
    Checkpoint(Option<String>),
    Undo,
    /// Browse and restore checkpoints (session tree).
    Tree,
    /// Fork a new conversation from a checkpoint.
    Fork(Option<String>),
    /// List all stored artifacts for this agent.
    Artifacts,
    /// Trigger reflection to extract memory from conversation history.
    Reflect(Option<String>),
    /// Show the background-computed session summary.
    Summarize,
    /// Show or change the execution backend.
    Backend(Option<String>),
    CompactionModel(String),
    /// Manually trigger session_summary consolidation.
    Compact,

    /// Browse the plugin marketplace.
    Marketplace,
    /// Reload Lua UI plugins.
    Reload,
    /// Check for and apply CADE updates.
    Update,
    /// Trust the current project directory.
    Trust,
    /// Toggle mouse capture dynamically
    Mouse,
    /// Open the Web GUI dashboard in browser (ADR 17)
    Gui,
}

pub(crate) fn parse_slash_with_skills(input: &str, skill_ids: &[String]) -> Option<SlashCmd> {
    let trimmed = input.trim();
    if !trimmed.starts_with('/') {
        return None;
    }
    let parts: Vec<&str> = trimmed[1..].splitn(2, ' ').collect();
    let arg = parts
        .get(1)
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty());
    // NOTE: prompt template expansion is handled separately in the REPL loop
    // before this function is called, so templates won't appear here.
    match parts[0] {
        "help" | "?" | "menu" => Some(SlashCmd::Help),
        "exit" | "quit" | "q" => Some(SlashCmd::Exit),
        "clear" => Some(SlashCmd::Clear),
        "summarize" | "summary" => Some(SlashCmd::Summarize),
        "agent" => Some(SlashCmd::Agent),
        "info" => Some(SlashCmd::Info),
        "new" => Some(SlashCmd::New),
        "new-agent" => Some(SlashCmd::NewAgent),
        "pin" => Some(SlashCmd::Pin),
        "agents" => Some(SlashCmd::Agents),
        "resume" => Some(SlashCmd::Resume),
        "delete" | "del" | "rm-agent" => Some(SlashCmd::Delete(arg)),
        "init" => Some(SlashCmd::Init),
        "remember" => Some(SlashCmd::Remember(arg.unwrap_or_default())),
        "memory" => Some(SlashCmd::Memory),
        "search" => Some(SlashCmd::Search(arg.unwrap_or_default())),
        "feedback" => Some(SlashCmd::Feedback),
        "skills" | "skill" => Some(SlashCmd::Skills(arg)),
        "subagents" | "agents-list" => Some(SlashCmd::Subagents),
        "teams" | "team" => Some(SlashCmd::Teams),
        "approvals" | "approval-list" => Some(SlashCmd::Approvals),
        "approve" => Some(SlashCmd::Approve(arg.unwrap_or_default())),
        "deny" => Some(SlashCmd::Deny(arg.unwrap_or_default())),
        "steer" => Some(SlashCmd::Steer(arg.unwrap_or_default())),
        "marketplace" | "plugins" => Some(SlashCmd::Marketplace),
        "reload" => Some(SlashCmd::Reload),
        "update" => Some(SlashCmd::Update),
        "trust" => Some(SlashCmd::Trust),
        "mouse" => Some(SlashCmd::Mouse),
        "theme" => Some(SlashCmd::Theme(arg)),
        "providers" | "provider-list" => Some(SlashCmd::Providers),
        "connect" => Some(SlashCmd::Connect(arg)),
        "disconnect" => Some(SlashCmd::Disconnect(arg.unwrap_or_default())),
        "approve-always" => Some(SlashCmd::ApproveAlways(arg.unwrap_or_default())),
        "deny-always" => Some(SlashCmd::DenyAlways(arg.unwrap_or_default())),
        "permissions" => Some(SlashCmd::Permissions),
        "hooks" => Some(SlashCmd::Hooks),
        "rename" => Some(SlashCmd::Rename(arg.unwrap_or_default())),
        "toolset" => Some(SlashCmd::Toolset(arg)),
        "yolo" => Some(SlashCmd::Yolo),
        "plan" => Some(SlashCmd::Plan),
        "todos" => Some(SlashCmd::Todos),
        "todo" => Some(SlashCmd::Todo),
        "default" | "normal" => Some(SlashCmd::Default),
        "mode" => Some(SlashCmd::Mode(arg)),
        "model" => Some(SlashCmd::Model(arg.unwrap_or_default())),
        "reasoning" => Some(SlashCmd::Reasoning(arg.unwrap_or_default())),
        "mcp" => Some(SlashCmd::Mcp),
        "mcp-save" => Some(SlashCmd::McpSave(arg.unwrap_or_default())),
        "link" => Some(SlashCmd::Link(arg)),
        "unlink" => Some(SlashCmd::Unlink(arg)),
        "logout" => Some(SlashCmd::Logout),
        "stream" => Some(SlashCmd::Stream),
        "usage" => Some(SlashCmd::Usage),
        "stats" => Some(SlashCmd::Stats(arg)),
        "cost" => Some(SlashCmd::Cost),
        "pricing" => Some(SlashCmd::Pricing(arg)),
        "context" => Some(SlashCmd::Context),
        "debug-last" | "debug_last" => Some(SlashCmd::DebugLast),
        "export" => Some(SlashCmd::Export(arg)),
        "checkpoint" | "cp" => Some(SlashCmd::Checkpoint(arg)),
        "undo" => Some(SlashCmd::Undo),
        "tree" | "session-tree" | "checkpoints" => Some(SlashCmd::Tree),
        "fork" => Some(SlashCmd::Fork(arg)),
        "artifacts" => Some(SlashCmd::Artifacts),
        "reflect" => Some(SlashCmd::Reflect(arg)),
        "backend" => Some(SlashCmd::Backend(arg)),
        "compaction-model" => Some(SlashCmd::CompactionModel(arg.unwrap_or_default())),
        "compact" | "consolidate" => Some(SlashCmd::Compact),
        "gui" | "dashboard" => Some(SlashCmd::Gui),
        // Skill slash commands: /skill:commit, /skill:review, or just /commit, /review, etc.
        other if skill_ids.contains(&other.to_string()) => {
            Some(SlashCmd::RunSkill(other.to_string(), arg))
        }
        other if other.starts_with("skill:") => {
            let id = other.strip_prefix("skill:").unwrap_or("").to_string();
            if !id.is_empty() && skill_ids.contains(&id) {
                Some(SlashCmd::RunSkill(id, arg))
            } else {
                None
            }
        }
        _ => None,
    }
}
