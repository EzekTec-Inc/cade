use crate::ui::SlashCommandDef;
use cade_core::capabilities::{Capability, CapabilitySet};

struct CommandDef {
    names: &'static [&'static str],
    section: &'static str,
    description: &'static str,
    needs_args: bool,
    capability: Option<Capability>,
    parse: fn(&str, Option<String>) -> SlashCmd,
    examples: &'static [(&'static str, bool)],
}

// One declaration owns the builtin variants, aliases, parsing and discoverability.
// The exhaustive handler match in commands.rs makes a newly declared variant
// require an implementation. Dynamic templates/Lua/Skills keep their existing
// precedence in the REPL; they are not inserted into this builtin catalogue.
macro_rules! command_catalogue {
    ($( $variant:ident $(($arg:ty))? => [$($name:literal),+], $section:literal,
        $description:literal, $needs_args:expr, $capability:expr, $parse:expr
        $(, examples: [$($example:literal => $example_args:expr),*])?; )*) => {
        #[derive(Debug, PartialEq, Eq)]
        pub(crate) enum SlashCmd {
            RunSkill(String, Option<String>),
            $( $variant $(($arg))?, )*
        }

        const COMMANDS: &[CommandDef] = &[
            $(CommandDef {
                names: &[$($name),+], section: $section,
                description: $description, needs_args: $needs_args,
                capability: $capability, parse: $parse,
                examples: &[$($(($example, $example_args)),*)?],
            },)*
        ];
    };
}

command_catalogue! {
    Help => ["help", "?", "menu"], "Session", "Browse available commands", false, None, |_, _| SlashCmd::Help;
    Exit => ["exit", "quit", "q"], "Session", "Exit CADE", false, None, |_, _| SlashCmd::Exit;
    Clear => ["clear"], "Session", "Clear the timeline and agent message context", false, None, |_, _| SlashCmd::Clear;
    Agent => ["agent"], "Session", "Show the current agent name and ID", false, None, |_, _| SlashCmd::Agent;
    Info => ["info"], "Session", "Show agent, conversation, model, mode and workspace", false, None, |_, _| SlashCmd::Info;
    New => ["new"], "Session", "Start a new conversation on the current agent", false, None, |_, _| SlashCmd::New;
    NewAgent => ["new-agent"], "Session", "Create a brand-new agent", false, None, |_, _| SlashCmd::NewAgent;
    Pin => ["pin"], "Session", "Pin the current agent in settings", false, None, |_, _| SlashCmd::Pin;
    Agents => ["agents"], "Session", "Browse, switch, rename or delete agents", false, Some(Capability::Agentic), |_, _| SlashCmd::Agents;
    Resume => ["resume", "session"], "Session", "Resume a conversation; /session new starts a fresh one", false, None, |name, arg| {
        if name == "session" && arg.as_deref() == Some("new") { SlashCmd::New } else { SlashCmd::Resume }
    };
    Rename(String) => ["rename"], "Session", "Rename the current agent", true, None, |_, arg| SlashCmd::Rename(arg.unwrap_or_default());
    Delete(Option<String>) => ["delete", "del", "rm-agent"], "Session", "Delete an agent by name or ID", true, None, |_, arg| SlashCmd::Delete(arg);
    Save => ["save"], "Session", "Save current TUI display settings", false, None, |_, _| SlashCmd::Save;
    Logout => ["logout"], "Session", "Clear the API key and exit", false, None, |_, _| SlashCmd::Logout;
    Model(String) => ["model"], "Model & Mode", "Show or switch the active model", false, None, |_, arg| SlashCmd::Model(arg.unwrap_or_default());
    Reasoning(String) => ["reasoning"], "Model & Mode", "Set reasoning effort (none, low, medium, high, xhigh)", false, None, |_, arg| SlashCmd::Reasoning(arg.unwrap_or_default());
    CompactionModel(String) => ["compaction-model"], "Model & Mode", "Set compaction model; bare command clears the override", true, None, |_, arg| SlashCmd::CompactionModel(arg.unwrap_or_default());
    Theme(Option<String>) => ["theme"], "Model & Mode", "Change theme; /theme list or /theme reload", false, None, |_, arg| SlashCmd::Theme(arg), examples: ["/theme list" => false];
    Toolset(Option<String>) => ["toolset"], "Model & Mode", "Show or switch the active toolset", false, None, |_, arg| SlashCmd::Toolset(arg);
    Mode(Option<String>) => ["mode"], "Model & Mode", "Show or set permission mode", false, None, |_, arg| SlashCmd::Mode(arg);
    Yolo => ["yolo"], "Model & Mode", "Enable bypass-permissions mode", false, None, |_, _| SlashCmd::Yolo;
    Plan => ["plan"], "Model & Mode", "Switch to read-only plan mode", false, None, |_, _| SlashCmd::Plan;
    Default => ["default", "normal"], "Model & Mode", "Return to default permission mode", false, None, |_, _| SlashCmd::Default;
    Todos => ["todos"], "Model & Mode", "Toggle the active plan panel", false, None, |_, _| SlashCmd::Todos;
    Todo => ["todo"], "Model & Mode", "Display the agent scratchpad (.cade-todo.md)", false, None, |_, _| SlashCmd::Todo;
    ApproveAlways(String) => ["approve-always"], "Permissions & Supervision", "Add a configured tool allow rule", true, None, |_, arg| SlashCmd::ApproveAlways(arg.unwrap_or_default());
    DenyAlways(String) => ["deny-always"], "Permissions & Supervision", "Add a configured tool deny rule", true, None, |_, arg| SlashCmd::DenyAlways(arg.unwrap_or_default());
    Permissions => ["permissions"], "Permissions & Supervision", "Manage tool permissions", false, None, |_, _| SlashCmd::Permissions;
    Approvals => ["approvals", "approval-list"], "Permissions & Supervision", "List pending approvals", false, None, |_, _| SlashCmd::Approvals;
    Approve(String) => ["approve"], "Permissions & Supervision", "Approve a pending action: /approve <id>", true, None, |_, arg| SlashCmd::Approve(arg.unwrap_or_default());
    Deny(String) => ["deny"], "Permissions & Supervision", "Deny a pending action: /deny <id> [feedback]", true, None, |_, arg| SlashCmd::Deny(arg.unwrap_or_default());
    Steer(String) => ["steer"], "Permissions & Supervision", "Guide a running child: /steer <subagent_id> <message>", true, None, |_, arg| SlashCmd::Steer(arg.unwrap_or_default());
    Subagents => ["subagents", "agents-list"], "Permissions & Supervision", "Browse available subagent modes", false, Some(Capability::Agentic), |_, _| SlashCmd::Subagents;
    Teams => ["teams", "team"], "Permissions & Supervision", "List discovered teams and their members", false, Some(Capability::Agentic), |_, _| SlashCmd::Teams;
    Memory => ["memory"], "Memory & History", "View/manage memory: view, set, edit, delete, history, export", false, None, |_, _| SlashCmd::Memory, examples: ["/memory view" => true, "/memory set" => true, "/memory edit" => true, "/memory delete" => true, "/memory history" => true, "/memory export" => true];
    Init => ["init"], "Memory & History", "Analyse the project and populate memory", false, None, |_, _| SlashCmd::Init;
    Remember(String) => ["remember"], "Memory & History", "Ask the agent to update memory", true, None, |_, arg| SlashCmd::Remember(arg.unwrap_or_default());
    Search(String) => ["search"], "Memory & History", "Search conversation history", true, None, |_, arg| SlashCmd::Search(arg.unwrap_or_default());
    Summarize => ["summarize", "summary"], "Memory & History", "Show the background-computed session summary", false, None, |_, _| SlashCmd::Summarize;
    Reflect(Option<String>) => ["reflect"], "Memory & History", "Extract memory from conversation history", false, Some(Capability::Agentic), |_, arg| SlashCmd::Reflect(arg);
    Compact => ["compact", "consolidate"], "Memory & History", "Consolidate dropped conversation turns", false, None, |_, _| SlashCmd::Compact;
    Checkpoint(Option<String>) => ["checkpoint", "cp"], "Memory & History", "Create a working-tree checkpoint: /checkpoint [label]", true, None, |_, arg| SlashCmd::Checkpoint(arg);
    Undo => ["undo"], "Memory & History", "Restore the latest checkpoint", false, None, |_, _| SlashCmd::Undo;
    Tree => ["tree", "session-tree", "checkpoints", "timeline"], "Memory & History", "Browse and restore checkpoints", false, None, |_, _| SlashCmd::Tree;
    Fork(Option<String>) => ["fork"], "Memory & History", "Fork a conversation from a checkpoint", true, None, |_, arg| SlashCmd::Fork(arg);
    Artifacts => ["artifacts"], "Memory & History", "Browse stored artifacts", false, Some(Capability::Agentic), |_, _| SlashCmd::Artifacts;
    Export(Option<String>) => ["export"], "Memory & History", "Export the current agent to JSON", true, None, |_, arg| SlashCmd::Export(arg);
    Backend(Option<String>) => ["backend"], "Tools & Providers", "Show/switch execution backend (local, docker, ssh, readonly, virtual)", false, None, |_, arg| SlashCmd::Backend(arg);
    Providers => ["providers", "provider-list"], "Tools & Providers", "Show configured AI providers", false, None, |_, _| SlashCmd::Providers;
    Connect(Option<String>) => ["connect"], "Tools & Providers", "Connect an AI provider interactively", true, None, |_, arg| SlashCmd::Connect(arg);
    Disconnect(String) => ["disconnect"], "Tools & Providers", "Remove a provider by name", true, None, |_, arg| SlashCmd::Disconnect(arg.unwrap_or_default());
    Mcp => ["mcp"], "Tools & Providers", "Show MCP server status and tools", false, Some(Capability::Mcp), |_, _| SlashCmd::Mcp;
    McpSave(String) => ["mcp-save"], "Tools & Providers", "Save MCP server configuration as JSON", true, None, |_, arg| SlashCmd::McpSave(arg.unwrap_or_default());
    Link(Option<String>) => ["link"], "Tools & Providers", "Register and attach tools", false, None, |_, arg| SlashCmd::Link(arg);
    Unlink(Option<String>) => ["unlink"], "Tools & Providers", "Detach tools", false, None, |_, arg| SlashCmd::Unlink(arg);
    Skills(Option<String>) => ["skills", "skill"], "Extensions", "Manage skills: list, new, show, reload", false, None, |_, arg| SlashCmd::Skills(arg), examples: ["/skills new" => true, "/skills reload" => false];
    Hooks => ["hooks"], "Extensions", "Manage session hooks", false, None, |_, _| SlashCmd::Hooks;
    Marketplace => ["marketplace", "plugins"], "Extensions", "Browse the plugin marketplace", false, None, |_, _| SlashCmd::Marketplace;
    Plugin(Option<String>) => ["plugin"], "Extensions", "Manage installed plugin inventory and lifecycle", false, None, |_, arg| SlashCmd::Plugin(arg);
    Reload => ["reload"], "Extensions", "Reload Lua UI plugins", false, None, |_, _| SlashCmd::Reload;
    Stream => ["stream"], "Display & Diagnostics", "Toggle live text output (off buffers text until turn end)", false, None, |_, _| SlashCmd::Stream;
    Details => ["details", "detail"], "Display & Diagnostics", "Toggle timeline detail expansion", false, None, |_, _| SlashCmd::Details;
    Mouse => ["mouse"], "Display & Diagnostics", "Toggle mouse capture for native text selection", false, None, |_, _| SlashCmd::Mouse;
    Usage => ["usage"], "Display & Diagnostics", "Show token usage for this session", false, None, |_, _| SlashCmd::Usage;
    Stats(Option<String>) => ["stats"], "Display & Diagnostics", "Show session statistics; /stats model for per-model detail", false, None, |_, arg| SlashCmd::Stats(arg), examples: ["/stats model" => false];
    Cost => ["cost"], "Display & Diagnostics", "Show session cost breakdown", false, None, |_, _| SlashCmd::Cost;
    Pricing(Option<String>) => ["pricing"], "Display & Diagnostics", "Manage token pricing rules", false, None, |_, arg| SlashCmd::Pricing(arg);
    Context => ["context"], "Display & Diagnostics", "Show context window usage", false, None, |_, _| SlashCmd::Context;
    DebugLast => ["debug-last", "debug_last"], "Display & Diagnostics", "Show the last assistant message stored by the server", false, None, |_, _| SlashCmd::DebugLast;
    Gui => ["gui", "dashboard"], "Display & Diagnostics", "Open the Web GUI dashboard", false, None, |_, _| SlashCmd::Gui;
    Doctor => ["doctor"], "Display & Diagnostics", "Check system health and terminal key passthrough", false, None, |_, _| SlashCmd::Doctor;
    Trust => ["trust"], "Display & Diagnostics", "Trust the current project directory", false, None, |_, _| SlashCmd::Trust;
    Update => ["update"], "Display & Diagnostics", "Check for and apply CADE updates", false, None, |_, _| SlashCmd::Update;
    Feedback => ["feedback"], "Display & Diagnostics", "Show the issue and feedback URL", false, None, |_, _| SlashCmd::Feedback;
}

pub(crate) fn all_slash_command_defs() -> Vec<SlashCommandDef> {
    COMMANDS
        .iter()
        .flat_map(|command| {
            command.names.iter().map(|name| SlashCommandDef {
                name: (*name).into(),
                description: command.description.into(),
            })
        })
        .collect()
}

pub(crate) fn command_menu_entries(
    caps: Option<&CapabilitySet>,
) -> Vec<crate::ui::menu::CommandMenuEntry> {
    let mut entries: Vec<_> = COMMANDS
        .iter()
        .filter(|command| {
            caps.is_none_or(|caps| command.capability.is_none_or(|cap| caps.is_enabled(cap)))
        })
        .flat_map(|command| {
            let aliases = &command.names[1..];
            let description = if aliases.is_empty() {
                command.description.to_owned()
            } else {
                format!(
                    "{} (aliases: /{})",
                    command.description,
                    aliases.join(", /")
                )
            };
            let primary = crate::ui::menu::CommandMenuEntry {
                command: format!("/{}", command.names[0]),
                description,
                section: command.section,
            };
            std::iter::once(primary).chain(command.examples.iter().map(|(example, _)| {
                crate::ui::menu::CommandMenuEntry {
                    command: (*example).into(),
                    description: command.description.into(),
                    section: command.section,
                }
            }))
        })
        .collect();
    // Informational agent-tool hints are not executable slash commands.
    for (command, description, capability) in [
        (
            "web_search",
            "Agent tool: search the web",
            Some(Capability::Web),
        ),
        (
            "fetch_doc",
            "Agent tool: fetch a URL as text",
            Some(Capability::Web),
        ),
        (
            "index_repository",
            "Agent tool: index repository symbols",
            None,
        ),
    ] {
        if caps.is_none_or(|caps| capability.is_none_or(|cap| caps.is_enabled(cap))) {
            entries.push(crate::ui::menu::CommandMenuEntry {
                command: command.into(),
                description: description.into(),
                section: "Agent tool hints",
            });
        }
    }
    entries
}

pub(crate) fn menu_selection_needs_args(input: &str) -> bool {
    let Some(name) = input.strip_prefix('/') else {
        return true;
    };
    if let Some((_, needs_args)) = COMMANDS
        .iter()
        .flat_map(|command| command.examples)
        .find(|(example, _)| *example == input)
    {
        return *needs_args;
    }
    COMMANDS
        .iter()
        .find(|command| command.names.contains(&name))
        .is_none_or(|command| command.needs_args)
}

pub(crate) fn parse_slash_with_skills(input: &str, skill_ids: &[String]) -> Option<SlashCmd> {
    let input = input.trim().strip_prefix('/')?;
    let (name, arg) = input.split_once(char::is_whitespace).unwrap_or((input, ""));
    let arg = (!arg.trim().is_empty()).then(|| arg.trim().to_owned());
    if let Some(command) = COMMANDS
        .iter()
        .find(|command| command.names.contains(&name))
    {
        return Some((command.parse)(name, arg));
    }
    // A loaded literal ID has priority over the explicit skill: prefix, as before.
    let id = if skill_ids.iter().any(|id| id == name) {
        name
    } else {
        name.strip_prefix("skill:")?
    };
    (!id.is_empty() && skill_ids.iter().any(|skill| skill == id))
        .then(|| SlashCmd::RunSkill(id.into(), arg))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeSet;

    #[test]
    fn executable_plugin_command_is_discoverable() {
        assert_eq!(
            parse_slash_with_skills("/plugin list", &[]),
            Some(SlashCmd::Plugin(Some("list".into())))
        );
        assert!(
            all_slash_command_defs()
                .iter()
                .any(|def| def.name == "plugin")
        );
        assert!(
            command_menu_entries(None)
                .iter()
                .any(|entry| entry.command == "/plugin")
        );
    }

    #[test]
    fn command_completion_names_are_unique() {
        let defs = all_slash_command_defs();
        let names: BTreeSet<_> = defs.iter().map(|def| &def.name).collect();
        assert_eq!(names.len(), defs.len());
        assert_eq!(names.len(), 97, "builtin compatibility inventory changed");
    }

    #[test]
    fn every_builtin_is_reachable_from_help_and_completion() {
        let entries = command_menu_entries(None);
        assert_eq!(
            entries.len(),
            87,
            "74 builtin commands, 10 examples and 3 tool hints"
        );
        let mut variants = std::collections::HashSet::new();
        for entry in entries
            .iter()
            .filter(|entry| entry.command.starts_with('/') && !entry.command.contains(' '))
        {
            let command =
                parse_slash_with_skills(&entry.command, &[]).expect("menu command must dispatch");
            assert!(
                variants.insert(std::mem::discriminant(&command)),
                "{} duplicates another handler",
                entry.command
            );
        }
        assert_eq!(variants.len(), 74);
        for example in entries
            .iter()
            .filter(|entry| entry.command.starts_with('/') && entry.command.contains(' '))
        {
            assert!(parse_slash_with_skills(&example.command, &[]).is_some());
        }
        for completion in all_slash_command_defs() {
            assert!(parse_slash_with_skills(&format!("/{}", completion.name), &[]).is_some());
        }
    }

    #[test]
    fn help_filters_only_disabled_capabilities_and_keeps_control_commands() {
        let core = CapabilitySet::core();
        let entries = command_menu_entries(Some(&core));
        let names: BTreeSet<_> = entries.iter().map(|entry| entry.command.as_str()).collect();
        assert!(!names.contains("/agents"));
        assert!(!names.contains("/subagents"));
        assert!(!names.contains("/mcp"));
        for control in ["/approvals", "/approve", "/deny", "/steer", "/plugin"] {
            assert!(names.contains(control));
        }
        assert_eq!(command_menu_entries(Some(&CapabilitySet::full())).len(), 87);
    }

    #[test]
    fn aliases_arguments_and_skill_precedence_remain_compatible() {
        let skills = vec!["plugin".into(), "review".into()];
        assert_eq!(
            parse_slash_with_skills(" /session new ", &skills),
            Some(SlashCmd::New)
        );
        assert_eq!(
            parse_slash_with_skills("/resume new", &skills),
            Some(SlashCmd::Resume)
        );
        assert_eq!(
            parse_slash_with_skills("/timeline", &skills),
            Some(SlashCmd::Tree)
        );
        assert_eq!(
            parse_slash_with_skills("/plugin", &skills),
            Some(SlashCmd::Plugin(None))
        );
        assert_eq!(
            parse_slash_with_skills("/skill:plugin inspect", &skills),
            Some(SlashCmd::RunSkill("plugin".into(), Some("inspect".into())))
        );
        assert_eq!(
            parse_slash_with_skills("/review details", &skills),
            Some(SlashCmd::RunSkill("review".into(), Some("details".into())))
        );
        assert_eq!(parse_slash_with_skills("/unknown", &skills), None);
        assert_eq!(
            parse_slash_with_skills("/approve\tapp-1", &skills),
            Some(SlashCmd::Approve("app-1".into()))
        );
    }

    #[test]
    fn help_does_not_execute_argument_required_commands_on_selection() {
        for command in [
            "/approve",
            "/deny",
            "/steer",
            "/mcp-save",
            "/compaction-model",
            "/rename",
        ] {
            assert!(menu_selection_needs_args(command), "{command}");
        }
        for command in [
            "/approvals",
            "/agents",
            "/plugin",
            "/stream",
            "/skills reload",
            "/theme list",
            "/stats model",
        ] {
            assert!(!menu_selection_needs_args(command), "{command}");
        }
    }
}
