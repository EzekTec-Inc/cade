#[derive(Debug)]
pub(crate) enum SlashCmd {
    Help,
    /// Invoke a loaded skill by its id (e.g. /commit → RunSkill("commit"))
    RunSkill(String),
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
    Default,
    Mode(Option<String>),
    Mcp,
    Link,
    Unlink,
    Logout,
    Stream,
    Usage,
    /// /stats [model]
    Stats(Option<String>),
    Copy,
    /// Export the current agent to a JSON file: /export [output.json]
    Export(Option<String>),
    /// Show current context window usage.
    Context,
    /// Dump the last assistant message as stored on the server.
    DebugLast,
    /// Show session cost breakdown (tokens × pricing).
    Cost,
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
    /// Show or change the execution backend.
    Backend(Option<String>),
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
        "skills" => Some(SlashCmd::Skills(arg)),
        "subagents" | "agents-list" => Some(SlashCmd::Subagents),
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
        "default" | "normal" => Some(SlashCmd::Default),
        "mode" => Some(SlashCmd::Mode(arg)),
        "model" => Some(SlashCmd::Model(arg.unwrap_or_default())),
        "reasoning" => Some(SlashCmd::Reasoning(arg.unwrap_or_default())),
        "mcp" => Some(SlashCmd::Mcp),
        "link" => Some(SlashCmd::Link),
        "unlink" => Some(SlashCmd::Unlink),
        "logout" => Some(SlashCmd::Logout),
        "stream" => Some(SlashCmd::Stream),
        "usage" => Some(SlashCmd::Usage),
        "stats" => Some(SlashCmd::Stats(arg)),
        "cost" => Some(SlashCmd::Cost),
        "context" => Some(SlashCmd::Context),
        "debug-last" | "debug_last" => Some(SlashCmd::DebugLast),
        "copy" => Some(SlashCmd::Copy),
        "export" => Some(SlashCmd::Export(arg)),
        "checkpoint" | "cp" => Some(SlashCmd::Checkpoint(arg)),
        "undo" => Some(SlashCmd::Undo),
        "tree" | "session-tree" => Some(SlashCmd::Tree),
        "fork" => Some(SlashCmd::Fork(arg)),
        "artifacts" => Some(SlashCmd::Artifacts),
        "reflect" => Some(SlashCmd::Reflect(arg)),
        "backend" => Some(SlashCmd::Backend(arg)),
        // Skill slash commands: /commit, /review, etc.
        other if skill_ids.iter().any(|id| id == other) => {
            Some(SlashCmd::RunSkill(other.to_string()))
        }
        _ => None,
    }
}
