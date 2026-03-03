use anyhow::Result;
use std::io;
use crossterm::event::KeyCode;

use std::sync::{Arc, Mutex};

use crate::agent::{CadeClient, client::{AgentState, CadeMessage}};
use crate::agent::session::SessionStore;
use crate::permissions::{PermissionManager, PermissionMode};
use crate::settings::SettingsManager;
use crate::skills::Skill;
use crate::subagents::{BackgroundResult, discover_all_subagents, find_subagent};
use crate::toolsets::Toolset;
use crate::tools::dispatch;
use crate::ui::{TuiApp, RenderLine, cycle_mode, cycle_mode_back};

const BANNER: &str = r#"
   ___    _    ____  _____
  / __|  / \  |  _ \| ____|
 | |    / _ \ | | | |  _|
 | |_  / ___ \| |_| | |___
  \__|/_/   \_|____/|_____|

 Coding AI assistant with Desktop Extensions
 Type /help for commands, /exit to quit
"#;

// ── Slash commands ─────────────────────────────────────────────────────────────

/// Result from the agent TUI picker.
enum AgentPickerResult {
    Switch(AgentState),
    DeleteMany(Vec<AgentState>),
    Rename { agent: AgentState, new_name: String },
}

#[derive(Debug)]
enum SlashCmd {
    Help,
    Menu,
    /// Invoke a loaded skill by its id (e.g. /commit → RunSkill("commit"))
    RunSkill(String),
    Exit,
    Clear,
    Agent,
    Info,
    Model(String),
    New,          // new conversation on same agent
    NewAgent,     // create a brand-new agent
    Pin,
    Agents,
    Resume,       // conversation picker
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
    Toolset(Option<String>),
    Delete(Option<String>),
    Yolo,
    Plan,
    Default,
    Mode(Option<String>),
    Mcp,
    Link,
    Unlink,
    Logout,
    Stream,
    Usage,
}

fn parse_slash(input: &str) -> Option<SlashCmd> {
    parse_slash_with_skills(input, &[])
}

fn parse_slash_with_skills(input: &str, skill_ids: &[String]) -> Option<SlashCmd> {
    let trimmed = input.trim();
    if !trimmed.starts_with('/') {
        return None;
    }
    let parts: Vec<&str> = trimmed[1..].splitn(2, ' ').collect();
    let arg = parts.get(1).map(|s| s.trim().to_string()).filter(|s| !s.is_empty());
    match parts[0] {
        "help" | "?" | "menu"    => Some(SlashCmd::Help),
        "exit" | "quit" | "q"   => Some(SlashCmd::Exit),
        "clear"                  => Some(SlashCmd::Clear),
        "agent"                  => Some(SlashCmd::Agent),
        "info"                   => Some(SlashCmd::Info),
        "new"                    => Some(SlashCmd::New),
        "new-agent"              => Some(SlashCmd::NewAgent),
        "pin"                    => Some(SlashCmd::Pin),
        "agents"                 => Some(SlashCmd::Agents),
        "resume"                 => Some(SlashCmd::Resume),
        "delete" | "del" | "rm-agent" => Some(SlashCmd::Delete(arg)),
        "init"                   => Some(SlashCmd::Init),
        "remember" if arg.is_some() => Some(SlashCmd::Remember(arg.unwrap())),
        "memory"                 => Some(SlashCmd::Memory),
        "search" if arg.is_some()   => Some(SlashCmd::Search(arg.unwrap())),
        "feedback"               => Some(SlashCmd::Feedback),
        "skills"                 => Some(SlashCmd::Skills(arg)),
        "subagents" | "agents-list" => Some(SlashCmd::Subagents),
        "providers" | "provider-list" => Some(SlashCmd::Providers),
        "connect"    => Some(SlashCmd::Connect(arg)),
        "disconnect" => Some(SlashCmd::Disconnect(arg.unwrap_or_default())),
        "approve-always" => Some(SlashCmd::ApproveAlways(arg.unwrap_or_default())),
        "deny-always"    => Some(SlashCmd::DenyAlways(arg.unwrap_or_default())),
        "permissions"    => Some(SlashCmd::Permissions),
        "hooks"          => Some(SlashCmd::Hooks),
        "rename"         => Some(SlashCmd::Rename(arg.unwrap_or_default())),
        "toolset"        => Some(SlashCmd::Toolset(arg)),
        "yolo"                   => Some(SlashCmd::Yolo),
        "plan"                   => Some(SlashCmd::Plan),
        "default" | "normal" => Some(SlashCmd::Default),
        "mode"                   => Some(SlashCmd::Mode(arg)),
        "model"  => Some(SlashCmd::Model(arg.unwrap_or_default())),
        "mcp"    => Some(SlashCmd::Mcp),
        "link"   => Some(SlashCmd::Link),
        "unlink" => Some(SlashCmd::Unlink),
        "logout" => Some(SlashCmd::Logout),
        "stream" => Some(SlashCmd::Stream),
        "usage"  => Some(SlashCmd::Usage),
        // Skill slash commands: /commit, /review, etc.
        other if skill_ids.iter().any(|id| id == other) => {
            Some(SlashCmd::RunSkill(other.to_string()))
        }
        _ => None,
    }
}

// ── Repl ──────────────────────────────────────────────────────────────────────

pub struct Repl {
    client: CadeClient,
    /// Shared-mutable so /new and /agents can hot-swap the agent mid-session
    agent_id:   Arc<Mutex<String>>,
    agent_name: Arc<Mutex<String>>,
    permissions: PermissionManager,
    current_model: Arc<Mutex<String>>,
    settings:   Arc<Mutex<SettingsManager>>,
    session:    Arc<Mutex<SessionStore>>,
    /// Working directory (for /init context)
    cwd: std::path::PathBuf,
    /// Currently loaded skills
    skills:     Arc<Mutex<Vec<Skill>>>,
    /// Directory from which skills are discovered
    skills_dir: std::path::PathBuf,
    /// Completed background subagent results waiting to be shown
    background_results: Arc<Mutex<Vec<BackgroundResult>>>,
    /// Active toolset — switches with /model
    current_toolset: Arc<Mutex<Toolset>>,
    /// Hook engine — fires user-defined scripts at lifecycle events
    hooks: crate::hooks::HookEngine,
    /// `true` until the first real user message is sent this session.
    /// Used to inject the environment context block (OS, cwd, git) on turn 1.
    first_turn: std::sync::Arc<std::sync::atomic::AtomicBool>,
    /// Set to `true` by a SIGINT handler while a turn is running.
    /// `stream_turn()` checks this flag and aborts the SSE stream early.
    cancel_turn: std::sync::Arc<std::sync::atomic::AtomicBool>,
    /// Active conversation ID — None means the default (legacy) conversation.
    conversation_id: Arc<Mutex<Option<String>>>,
    /// MCP server manager — routes tool calls with `{server}__` prefix.
    mcp: std::sync::Arc<crate::mcp::McpManager>,
    /// Whether SSE token streaming is enabled (toggled by /stream).
    streaming_enabled: std::sync::Arc<std::sync::atomic::AtomicBool>,
    /// Cumulative token usage for the session (input, output).
    session_input_tokens:  std::sync::Arc<std::sync::atomic::AtomicU64>,
    session_output_tokens: std::sync::Arc<std::sync::atomic::AtomicU64>,
    /// Fullscreen ratatui TUI — single render path for all output + input.
    app: Arc<Mutex<TuiApp>>,
}

impl Repl {
    pub fn new(
        client: CadeClient,
        agent_id: String,
        agent_name: String,
        permissions: PermissionManager,
        current_model: String,
        settings: Arc<Mutex<SettingsManager>>,
        session: Arc<Mutex<SessionStore>>,
        cwd: std::path::PathBuf,
        skills: Vec<Skill>,
        skills_dir: std::path::PathBuf,
        toolset: Toolset,
        hooks: crate::hooks::HookEngine,
        conversation_id: Option<String>,
        mcp: std::sync::Arc<crate::mcp::McpManager>,
    ) -> Self {
        let perm_mode        = permissions.mode();
        let agent_name_clone = agent_name.clone();
        let current_model_clone = current_model.clone();
        Self {
            client,
            agent_id:   Arc::new(Mutex::new(agent_id)),
            agent_name: Arc::new(Mutex::new(agent_name)),
            permissions,
            current_model: Arc::new(Mutex::new(current_model)),
            settings,
            session,
            cwd,
            skills:     Arc::new(Mutex::new(skills)),
            skills_dir,
            background_results: Arc::new(Mutex::new(vec![])),
            current_toolset: Arc::new(Mutex::new(toolset)),
            hooks,
            first_turn:            std::sync::Arc::new(std::sync::atomic::AtomicBool::new(true)),
            cancel_turn:           std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false)),
            conversation_id:       Arc::new(Mutex::new(conversation_id)),
            mcp,
            streaming_enabled:     std::sync::Arc::new(std::sync::atomic::AtomicBool::new(true)),
            session_input_tokens:  std::sync::Arc::new(std::sync::atomic::AtomicU64::new(0)),
            session_output_tokens: std::sync::Arc::new(std::sync::atomic::AtomicU64::new(0)),
            app: Arc::new(Mutex::new(TuiApp::new(
                perm_mode,
                agent_name_clone.clone(),
                current_model_clone.clone(),
            ))),
        }
    }

    fn agent_id(&self)       -> String        { self.agent_id.lock().unwrap().clone() }
    fn agent_name(&self)     -> String        { self.agent_name.lock().unwrap().clone() }
    fn model(&self)          -> String        { self.current_model.lock().unwrap().clone() }
    fn conversation_id(&self) -> Option<String> { self.conversation_id.lock().unwrap().clone() }

    /// Called when `--continue` is set — suppress first-turn env injection.
    pub fn mark_continued(&self) {
        use std::sync::atomic::Ordering;
        self.first_turn.store(false, Ordering::SeqCst);
    }

    pub async fn run(mut self) -> Result<()> {
        let mut stdout = io::stdout();

        // Push banner + agent info into TuiApp content.
        {
            let mut app = self.app.lock().unwrap();
            let agent_id   = self.agent_id.lock().unwrap().clone();
            let agent_name = self.agent_name.lock().unwrap().clone();
            let model      = self.current_model.lock().unwrap().clone();
            let mode_str   = format!("{}", self.permissions.mode());
            let banner_text = format!(
                "{BANNER}\n  Agent  : {agent_name}  ({agent_id})\n  Model  : {model}\n  Mode   : {mode_str}"
            );
            app.push_silent(RenderLine::SystemMsg(banner_text));
            app.draw()?;
        }

        // SessionStart hook (non-blocking)
        self.hooks.session_start(&self.agent_id()).await;

        let mut history: Vec<String> = Vec::new();
        let mut hist_idx: Option<usize> = None;

        let mut pending_input: Option<String> = None;
        loop {
            // Check for completed background subagent results
            {
                let mut results = self.background_results.lock().unwrap();
                for r in results.drain(..) {
                    let msg = format!(
                        "  ✓ Subagent '{}' finished:\n{}",
                        r.subagent, r.result
                    );
                    let _ = self.app.lock().unwrap().push(RenderLine::SystemMsg(msg));
                    let notify = format!(
                        "[Background subagent '{}' completed (task ID: {})]:\n{}",
                        r.subagent, r.task_id, r.result
                    );
                    let _ = self.client.send_message(&self.agent_id(), &notify).await;
                }
            }

            // Update app footer to reflect current mode/model before reading input.
            {
                let mut app = self.app.lock().unwrap();
                app.update_mode(self.permissions.mode());
                app.update_model(self.current_model.lock().unwrap().clone());
                app.update_agent_name(self.agent_name());
            }

            // Read input — either from pending (menu dispatch) or from the user.
            let input = if let Some(cmd) = pending_input.take() {
                cmd
            } else {
                match self.app.lock().unwrap().read_input(&mut history, &mut hist_idx)? {
                    Some(s) => s,
                    None => break,
                }
            };
            let mut input = input.trim().to_string();

            // Handle Tab / BackTab mode-cycle sentinels.
            if input == "__TAB__" {
                let next = cycle_mode(self.permissions.mode());
                self.permissions.set_mode(next);
                self.app.lock().unwrap().update_mode(next);
                continue;
            }
            if input == "__BACKTAB__" {
                let prev = cycle_mode_back(self.permissions.mode());
                self.permissions.set_mode(prev);
                self.app.lock().unwrap().update_mode(prev);
                continue;
            }

            if input.is_empty() { continue; }
            history.push(input.clone());
            hist_idx = None;

            // Echo user message.
            let _ = self.app.lock().unwrap().push(RenderLine::UserMessage(input.clone()));

            // Direct bash: lines starting with '!'
            if let Some(cmd) = input.strip_prefix('!') {
                let cmd = cmd.trim();
                if !cmd.is_empty() {
                    let output = tokio::process::Command::new("sh")
                        .arg("-c")
                        .arg(cmd)
                        .output()
                        .await;
                    match output {
                        Ok(out) => {
                            let text = if out.stdout.is_empty() {
                                String::from_utf8_lossy(&out.stderr).to_string()
                            } else {
                                String::from_utf8_lossy(&out.stdout).to_string()
                            };
                            let _ = self.app.lock().unwrap().push(RenderLine::SystemMsg(text));
                        }
                        Err(e) => {
                            let _ = self.app.lock().unwrap().push(RenderLine::ErrorMsg(format!("bash: {e}")));
                        }
                    }
                }
                continue;
            }

            // Slash commands (include loaded skill ids so /commit etc. work)
            let skill_ids: Vec<String> = self.skills.lock().unwrap()
                .iter().map(|s| s.id.clone()).collect();
            if let Some(cmd) = parse_slash_with_skills(&input, &skill_ids) {
                match cmd {
                    SlashCmd::Exit => {
                        use std::sync::atomic::Ordering;
                        let in_tok  = self.session_input_tokens.load(Ordering::SeqCst);
                        let out_tok = self.session_output_tokens.load(Ordering::SeqCst);
                        if in_tok > 0 || out_tok > 0 {
                            let _ = self.app.lock().unwrap().push(RenderLine::SystemMsg(
                                format!("  Session tokens — in: {in_tok}  out: {out_tok}  total: {}", in_tok + out_tok)
                            ));
                        }
                        let _ = self.app.lock().unwrap().push(RenderLine::SystemMsg("Bye!".to_string()));
                        break;
                    }
                    // SlashCmd::Clear is handled below (with context clearing)
                    SlashCmd::RunSkill(skill_id) => {
                        // Find the skill, build a prompt that injects its content,
                        // and send it as an agent turn so the agent follows the skill.
                        let skill_body = self.skills.lock().unwrap()
                            .iter()
                            .find(|s| s.id == skill_id)
                            .map(|s| s.to_context_block());
                        if let Some(body) = skill_body {
                            let prompt = format!(
                                "[Skill invoked: /{skill_id}]\n\nFollow this skill:\n\n{body}"
                            );
                            self.tui_sys(format!("  Running skill: /{skill_id}"));
                            self.agent_turn(&mut stdout, &prompt).await?;
                        } else {
                            self.tui_err(format!("  Skill '{skill_id}' not found. Try /skills reload"));
                        }
                        continue;
                    }
                    SlashCmd::Help | SlashCmd::Menu => {
                        // Open full-screen command browser
                        let chosen = {
                            let mut app = self.app.lock().unwrap();
                            crate::ui::menu::show_command_menu(&mut app.terminal)?
                        };
                        let _ = self.app.lock().unwrap().draw();
                        if let Some(cmd) = chosen {
                            pending_input = Some(cmd);
                        }
                        continue;
                    }
                    SlashCmd::Agent => {
                        let msg = format!("  Agent: {} ({})", self.agent_name(), self.agent_id());
                        let _ = self.app.lock().unwrap().push(RenderLine::SystemMsg(msg));
                    }
                    SlashCmd::Info => {
                        let msg = format!(
                            "  Agent   : {} ({})\n  Conv    : {}\n  Model   : {}\n  Mode    : {}\n  CWD     : {}\n  Version : {}",
                            self.agent_name(), self.agent_id(),
                            self.conversation_id().as_deref().unwrap_or("default"),
                            self.model(), self.permissions.mode(),
                            self.cwd.display(), env!("CARGO_PKG_VERSION")
                        );
                        let _ = self.app.lock().unwrap().push(RenderLine::SystemMsg(msg));
                    }
                    SlashCmd::Yolo => {
                        self.permissions.set_mode(PermissionMode::BypassPermissions);
                        self.app.lock().unwrap().update_mode(PermissionMode::BypassPermissions);
                        let _ = self.app.lock().unwrap().push(RenderLine::SystemMsg(
                            "⚡ Permission mode: bypassPermissions — all tools auto-approved".to_string()
                        ));
                    }
                    SlashCmd::Mcp => {
                        let statuses = self.mcp.status();
                        self.tui_blank();
                        self.tui_hdr("  MCP Servers");
                        self.tui_blank();
                        if statuses.is_empty() {
                            self.tui_dim("  No MCP servers configured.");
                            self.tui_blank();
                            self.tui_dim("  Add servers to ~/.cade/settings.json:");
                            self.tui_dim("  {");
                            self.tui_dim("    \"mcpServers\": {");
                            self.tui_dim("      \"git\": { \"command\": \"/path/to/git-mcp-server\" }");
                            self.tui_dim("    }");
                            self.tui_dim("  }");
                        } else {
                            for s in &statuses {
                                self.tui_ok(format!("  ● {:<16} {} tool(s)", s.key, s.tools.len()));
                                // Show tool names (strip prefix for clarity)
                                for chunk in s.tools.chunks(4) {
                                    let names: Vec<&str> = chunk.iter()
                                        .map(|t| t.splitn(2, "__").nth(1).unwrap_or(t))
                                        .collect();
                                    self.tui_dim(format!("    {}", names.join("  ")));
                                }
                                self.tui_blank();
                            }
                        }
                    }
                    SlashCmd::Link => {
                        self.tui_dim("  Linking tools…");
                        let client2  = self.client.clone();
                        let mcp2     = std::sync::Arc::clone(&self.mcp);
                        let toolset2 = *self.current_toolset.lock().unwrap();
                        let agent_id = self.agent_id();
                        use crate::agent::tools::{register_cade_tools, register_mcp_tools};
                        let native_ids: Vec<String> = register_cade_tools(&client2, toolset2)
                            .await.unwrap_or_default().into_iter().map(|t| t.id).collect();
                        let n_native = native_ids.len();
                        if !native_ids.is_empty() {
                            let _ = client2.attach_agent_tools(&agent_id, &native_ids).await;
                        }
                        let mcp_ids: Vec<String> = register_mcp_tools(&client2, mcp2.all_tool_schemas())
                            .await.unwrap_or_default().into_iter().map(|t| t.id).collect();
                        let n_mcp = mcp_ids.len();
                        if !mcp_ids.is_empty() {
                            let _ = client2.attach_agent_tools(&agent_id, &mcp_ids).await;
                        }
                        self.tui_ok(format!("  ✓ Linked {n_native} native + {n_mcp} MCP tool(s)"));
                    }
                    SlashCmd::Unlink => {
                        let agent_id = self.agent_id();
                        match self.client.detach_agent_tools(&agent_id).await {
                            Ok(n)  => self.tui_ok(format!("  ✓ Detached {n} tool(s) from agent")),
                            Err(e) => self.tui_err(e.to_string()),
                        }
                    }
                    SlashCmd::Stream => {
                        use std::sync::atomic::Ordering;
                        let current = self.streaming_enabled.load(Ordering::SeqCst);
                        self.streaming_enabled.store(!current, Ordering::SeqCst);
                        let label = if !current { "on" } else { "off" };
                        self.tui_hdr(format!("  Streaming: {label}"));
                    }
                    SlashCmd::Usage => {
                        use std::sync::atomic::Ordering;
                        let in_tok  = self.session_input_tokens.load(Ordering::SeqCst);
                        let out_tok = self.session_output_tokens.load(Ordering::SeqCst);
                        let total   = in_tok + out_tok;
                        self.tui_blank();
                        self.tui_hdr("  Token usage this session:");
                        self.tui_dim(format!("    Input  : {:>8}", in_tok));
                        self.tui_dim(format!("    Output : {:>8}", out_tok));
                        self.tui_dim(format!("    Total  : {:>8}", total));
                        if total == 0 {
                            self.tui_dim("    (no usage recorded yet — requires Anthropic/OpenAI)");
                        }
                    }
                    SlashCmd::Logout => {
                        if let Ok(mut s) = self.settings.lock() {
                            s.clear_api_key();
                        }
                        self.tui_ok("  ✓ API key cleared. Restart CADE to re-authenticate.");
                        return Ok(());
                    }
                    SlashCmd::Plan => {
                        self.permissions.set_mode(PermissionMode::Plan);
                        self.tui_hdr("📖 Permission mode: plan (read-only) — write/exec tools blocked. Use /default to resume.");
                    }
                    SlashCmd::Default => {
                        self.permissions.set_mode(PermissionMode::Default);
                        self.tui_ok("✅ Permission mode: default — tools require approval");
                    }
                    SlashCmd::Mode(arg) => {
                        match arg.as_deref() {
                            None | Some("") => {
                                let (icon, label, hint) = mode_display(self.permissions.mode());
                                self.tui_sys(format!("{icon} Current mode: {label}  {hint}"));
                            }
                            Some(name) => {
                                match name.to_lowercase().as_str() {
                                    "default" | "normal" => {
                                        self.permissions.set_mode(PermissionMode::Default);
                                        self.tui_ok("✅ Permission mode: default");
                                    }
                                    "plan" | "readonly" | "read-only" => {
                                        self.permissions.set_mode(PermissionMode::Plan);
                                        self.tui_hdr("📖 Permission mode: plan (read-only). Use /default to resume.");
                                    }
                                    "yolo" | "bypass" | "bypasspermissions" => {
                                        self.permissions.set_mode(PermissionMode::BypassPermissions);
                                        self.tui_sys("⚡ Permission mode: bypassPermissions");
                                    }
                                    "acceptedits" | "accept-edits" | "edits" => {
                                        self.permissions.set_mode(PermissionMode::AcceptEdits);
                                        self.tui_ok("📝 Permission mode: acceptEdits — file edits auto-approved");
                                    }
                                    other => {
                                        self.tui_err(format!("Unknown mode '{other}'. Valid: default | plan | yolo | acceptEdits"));
                                    }
                                }
                            }
                        }
                    }
                    // SlashCmd::New is handled below (hot-swap)
                    SlashCmd::Model(m) => {
                        // Empty arg → open interactive picker
                        let m = if m.is_empty() {
                            match self.interactive_model_picker(Arc::clone(&self.app)).await? {
                                Some(picked) => picked,
                                None => { let _ = self.app.lock().unwrap().draw(); continue; }
                            }
                        } else {
                            m
                        };
                        let new_toolset = Toolset::for_model(&m);
                        let old_toolset = *self.current_toolset.lock().unwrap();
                        self.tui_dim(format!("  Switching model → {m}…"));
                        match self.client.patch_agent_model(&self.agent_id(), &m).await {
                            Ok(new_model) => {
                                *self.current_model.lock().unwrap() = new_model.clone();
                                if new_toolset != old_toolset {
                                    *self.current_toolset.lock().unwrap() = new_toolset;
                                    let agent_id = self.agent_id();
                                    let client = self.client.clone();
                                    let mcp_ts  = std::sync::Arc::clone(&self.mcp);
                                    tokio::spawn(async move {
                                        use crate::agent::tools::{register_cade_tools, register_mcp_tools};
                                        let tools = register_cade_tools(&client, new_toolset)
                                            .await.unwrap_or_default();
                                        let ids: Vec<String> = tools.into_iter().map(|t| t.id).collect();
                                        if !ids.is_empty() {
                                            let _ = client.attach_agent_tools(&agent_id, &ids).await;
                                        }
                                        let mcp_ids: Vec<String> = register_mcp_tools(&client, mcp_ts.all_tool_schemas())
                                            .await.unwrap_or_default()
                                            .into_iter().map(|t| t.id).collect();
                                        if !mcp_ids.is_empty() {
                                            let _ = client.attach_agent_tools(&agent_id, &mcp_ids).await;
                                        }
                                    });
                                    self.tui_hdr(format!("  Toolset → {}", new_toolset.display_name()));
                                }
                                self.tui_ok(format!("  ✓ Model: {new_model}"));
                                let _ = self.app.lock().unwrap().draw();
                            }
                            Err(e) => self.tui_err(e.to_string()),
                        }
                    }

                    // ── New commands ──────────────────────────────────────────

                    SlashCmd::Clear => {
                        let _ = self.app.lock().unwrap().clear_content();
                        match self.client.clear_messages(&self.agent_id()).await {
                            Ok(n)  => self.tui_ok(format!("✓ Context window cleared ({n} messages deleted)")),
                            Err(e) => self.tui_sys(format!("⚠ Screen cleared (context clear failed: {e})")),
                        }
                    }

                    SlashCmd::New => {
                        let agent_id = self.agent_id();
                        match self.client.create_conversation(&agent_id, "").await {
                            Ok(conv) => {
                                let cid = conv["id"].as_str().unwrap_or("").to_string();
                                *self.conversation_id.lock().unwrap() = Some(cid.clone());
                                if let Ok(mut s) = self.session.lock() {
                                    let _ = s.set_conversation(Some(cid.clone()));
                                }
                                self.first_turn.store(true, std::sync::atomic::Ordering::SeqCst);
                                self.tui_ok(format!("  ✓ New conversation started  ({})", &cid[..cid.len().min(20)]));
                            }
                            Err(e) => self.tui_err(e.to_string()),
                        }
                    }

                    SlashCmd::NewAgent => {
                        let _ = self.app.lock().unwrap().push(RenderLine::SystemMsg("  Creating new agent…".to_string()));

                        // S5: Offer to copy `human` and `project` blocks from current agent
                        let prev_agent_id = self.agent_id();
                        let inherit_blocks: Vec<(String, String, String)> = {
                            let blocks = self.client.get_memory(&prev_agent_id).await.unwrap_or_default();
                            blocks.into_iter()
                                .filter(|b| (b.label == "human" || b.label == "project") && !b.value.trim().is_empty())
                                .map(|b| (b.label.clone(), b.value.clone(), b.description.clone().unwrap_or_default()))
                                .collect()
                        };
                        let copy_memory = if !inherit_blocks.is_empty() {
                            let summary: String = inherit_blocks.iter()
                                .map(|(l, v, _)| format!("{} ({} chars)", l, v.chars().count()))
                                .collect::<Vec<_>>().join(", ");
                            let q = crate::ui::question::Question {
                                header: "Copy memory",
                                text: &format!("Copy memory to new agent? ({summary})"),
                                options: &[
                                    crate::ui::question::QuestionOption { label: "Yes — copy human + project blocks".to_string(), description: "Start new agent with existing context".to_string() },
                                    crate::ui::question::QuestionOption { label: "No — start fresh".to_string(), description: "New agent gets empty memory blocks".to_string() },
                                ],
                                multi_select: false,
                                allow_other: false,
                                progress: None,
                            };
                            let ans = {
                                let mut app = self.app.lock().unwrap();
                                let r = crate::ui::question::QuestionWidget::ask(&mut app.terminal, &q);
                                let _ = app.draw();
                                r
                            };
                            matches!(ans, Ok(Some(ref a)) if a.as_str().starts_with("Yes"))
                        } else {
                            false
                        };

                        let model = self.model();
                        let req = crate::agent::client::CreateAgentRequest {
                            name: Some(format!("CADE-{}", chrono::Local::now().format("%Y%m%d-%H%M%S"))),
                            model,
                            description: Some("CADE coding agent".to_string()),
                            system_prompt: None,
                            memory_blocks: vec![],
                            tool_ids: vec![],
                        };
                        match self.client.create_agent(req).await {
                            Ok(a) => {
                                *self.agent_id.lock().unwrap()   = a.id.clone();
                                *self.agent_name.lock().unwrap() = a.name.clone();
                                *self.conversation_id.lock().unwrap() = None;
                                if let Ok(mut s) = self.settings.lock() {
                                    let _ = s.set_last_agent(&a.id);
                                }
                                if let Ok(mut s) = self.session.lock() {
                                    let _ = s.set_agent(a.id.clone(), Some(a.name.clone()));
                                }
                                let _ = self.app.lock().unwrap().push(RenderLine::SystemMsg(
                                    format!("  ✓ New agent: {} ({})", a.name, a.id)
                                ));

                                // S5: copy inherited blocks to new agent
                                if copy_memory {
                                    for (label, value, desc) in &inherit_blocks {
                                        let desc_opt = if desc.is_empty() { None } else { Some(desc.as_str()) };
                                        let _ = self.client.upsert_memory(&a.id, label, value, desc_opt).await;
                                    }
                                    let n = inherit_blocks.len();
                                    let _ = self.app.lock().unwrap().push(RenderLine::SystemMsg(
                                        format!("  ✓ Copied {n} memory block(s) from previous agent")
                                    ));
                                }

                                // Attach native + MCP tools in background
                                let client2  = self.client.clone();
                                let mcp2     = std::sync::Arc::clone(&self.mcp);
                                let toolset2 = *self.current_toolset.lock().unwrap();
                                let new_id   = a.id.clone();
                                tokio::spawn(async move {
                                    use crate::agent::tools::{register_cade_tools, register_mcp_tools};
                                    let native_ids: Vec<String> = register_cade_tools(&client2, toolset2)
                                        .await.unwrap_or_default()
                                        .into_iter().map(|t| t.id).collect();
                                    if !native_ids.is_empty() {
                                        let _ = client2.attach_agent_tools(&new_id, &native_ids).await;
                                    }
                                    let mcp_ids: Vec<String> = register_mcp_tools(&client2, mcp2.all_tool_schemas())
                                        .await.unwrap_or_default()
                                        .into_iter().map(|t| t.id).collect();
                                    if !mcp_ids.is_empty() {
                                        let _ = client2.attach_agent_tools(&new_id, &mcp_ids).await;
                                    }
                                });
                            }
                            Err(e) => self.tui_err(e.to_string()),
                        }
                    }

                    SlashCmd::Resume => {
                        self.tui_dim("  Fetching conversations…");
                        let agent_id = self.agent_id();
                        match self.client.list_conversations(&agent_id).await {
                            Ok(convs) => {
                                if convs.is_empty() {
                                    let _ = self.app.lock().unwrap().push(RenderLine::DimMsg("  No saved conversations yet. Use /new to start one.".to_string()));
                                } else if let Some(picked) = self.conversation_picker(Arc::clone(&self.app), &convs, &agent_id).await? {
                                    let cid = picked["id"].as_str().unwrap_or("").to_string();
                                    *self.conversation_id.lock().unwrap() = Some(cid.clone());
                                    if let Ok(mut s) = self.session.lock() {
                                        let _ = s.set_conversation(Some(cid));
                                    }
                                    self.first_turn.store(false, std::sync::atomic::Ordering::SeqCst);
                                    let _ = self.app.lock().unwrap().push(RenderLine::SuccessMsg(
                                        format!("  ✓ Switched to: {}", picked["title"].as_str().unwrap_or("(untitled)"))
                                    ));
                                }
                                let _ = self.app.lock().unwrap().draw();
                            }
                            Err(e) => { let _ = self.app.lock().unwrap().push(RenderLine::ErrorMsg(e.to_string())); }
                        }
                    }

                    SlashCmd::Pin => {
                        let id   = self.agent_id();
                        let name = self.agent_name();
                        if let Ok(mut s) = self.settings.lock() {
                            match s.pin_agent(&id, &name) {
                                Ok(_)  => self.tui_ok(format!("  ✓ Pinned: {name} ({id})")),
                                Err(e) => self.tui_err(format!("Pin failed: {e}")),
                            }
                        }
                    }

                    SlashCmd::Agents => {
                        self.tui_dim("  Fetching agents…");
                        match self.client.list_agents().await {
                            Ok(agents) if agents.is_empty() => {
                                self.tui_dim("  (no agents found)");
                            }
                            Ok(mut agents) => {
                                if let Some(result) = self.agent_picker(Arc::clone(&self.app), &mut agents).await? {
                                    match result {
                                        AgentPickerResult::Switch(a) => {
                                            *self.agent_id.lock().unwrap()   = a.id.clone();
                                            *self.agent_name.lock().unwrap() = a.name.clone();
                                            if let Ok(mut s) = self.settings.lock() {
                                                let _ = s.set_last_agent(&a.id);
                                            }
                                            self.tui_ok(format!("  ✓ Switched to: {} ({})", a.name, a.id));
                                        }
                                        AgentPickerResult::Rename { agent, new_name } => {
                                            match self.client.rename_agent(&agent.id, &new_name).await {
                                                Ok(_) => {
                                                    if agent.id == self.agent_id() {
                                                        *self.agent_name.lock().unwrap() = new_name.clone();
                                                    }
                                                    self.tui_ok(format!("  ✓ Renamed '{}' → '{new_name}'", agent.name));
                                                }
                                                Err(e) => self.tui_err(e.to_string()),
                                            }
                                        }
                                        AgentPickerResult::DeleteMany(to_delete) => {
                                            let current_id = self.agent_id();
                                            let mut deleted_active = false;
                                            for a in &to_delete {
                                                match self.client.delete_agent(&a.id).await {
                                                    Ok(_) => {
                                                        self.tui_ok(format!("  ✓ Deleted: {}", a.name));
                                                        if a.id == current_id { deleted_active = true; }
                                                    }
                                                    Err(e) => self.tui_err(e.to_string()),
                                                }
                                            }
                                            if deleted_active {
                                                match self.client.list_agents().await {
                                                    Ok(remaining) if !remaining.is_empty() => {
                                                        let first = &remaining[0];
                                                        *self.agent_id.lock().unwrap()   = first.id.clone();
                                                        *self.agent_name.lock().unwrap() = first.name.clone();
                                                        if let Ok(mut s) = self.settings.lock() {
                                                            let _ = s.set_last_agent(&first.id);
                                                        }
                                                        self.tui_dim(format!("  → Now using: {}", first.name));
                                                    }
                                                    _ => {
                                                        self.tui_dim("  No remaining agents — run /new to create one");
                                                    }
                                                }
                                            }
                                        }
                                    }
                                }
                                let _ = self.app.lock().unwrap().draw();
                            }
                            Err(e) => self.tui_err(e.to_string()),
                        }
                    }

                    SlashCmd::Delete(target) => {
                        // /delete [name-or-id] — delete a specific agent by name/id prefix
                        let agents = match self.client.list_agents().await {
                            Ok(a) => a,
                            Err(e) => { self.print_error(&mut stdout, &e.to_string())?; vec![] }
                        };
                        if agents.is_empty() { self.tui_dim("  (no agents)"); }
                        else if let Some(query) = target {
                            let q = query.to_lowercase();
                            let matched: Vec<_> = agents.iter()
                                .filter(|a| a.name.to_lowercase().contains(&q) || a.id.starts_with(&q))
                                .collect();
                            match matched.len() {
                                0 => self.tui_err(format!("No agent matching '{query}'")),
                                1 => {
                                    let a = matched[0];
                                    use crate::ui::question::{Question, QuestionOption, QuestionWidget};
                                    let opts = vec![
                                        QuestionOption { label: "Yes — delete".to_string(), description: String::new() },
                                        QuestionOption { label: "No — cancel".to_string(),  description: String::new() },
                                    ];
                                    let q_widget = Question {
                                        header: "Confirm delete", text: &format!("Delete '{}'?", a.name),
                                        options: &opts, multi_select: false, allow_other: false, progress: None,
                                    };
                                    let confirmed = {
                                        let mut app = self.app.lock().unwrap();
                                        let r = QuestionWidget::ask(&mut app.terminal, &q_widget)?;
                                        let _ = app.draw();
                                        matches!(r, Some(ref a) if a.as_str().starts_with("Yes"))
                                    };
                                    if confirmed {
                                        match self.client.delete_agent(&a.id).await {
                                            Ok(_) => {
                                                self.tui_ok(format!("  ✓ Deleted: {}", a.name));
                                                if a.id == self.agent_id() {
                                                    self.tui_dim("  Active agent deleted — use /new or /agents to continue");
                                                }
                                            }
                                            Err(e) => self.tui_err(e.to_string()),
                                        }
                                    } else {
                                        self.tui_dim("  (cancelled)");
                                    }
                                }
                                n => self.tui_err(format!("{n} agents match '{query}' — be more specific")),
                            }
                        } else {
                            self.tui_dim("  Usage: /delete <name-or-id>  or  /agents then press d");
                        }
                    }

                    SlashCmd::Init => {
                        self.tui_dim(format!("  Analysing project at {}…", self.cwd.display()));

                        let explore_prompt = format!(
                            "Analyse the project at '{}'. \
                             Read: README.md, Cargo.toml / package.json / pyproject.toml / go.mod (whichever exist), \
                             src/ or lib/ directory structure (top-level only), .env.example if present. \
                             Return a concise report covering: \
                             (1) Project name and purpose (2 sentences), \
                             (2) Language + framework / stack, \
                             (3) Key source directories and their purpose, \
                             (4) Build / test commands, \
                             (5) Any important conventions or notes from README. \
                             Be specific and factual. Maximum 400 words.",
                            self.cwd.display()
                        );

                        let agent_id = self.agent_id();
                        let client = self.client.clone();
                        let cwd = self.cwd.clone();
                        let all_defs = crate::subagents::discover_all_subagents(&cwd);
                        let explore_def = crate::subagents::find_subagent("explore", &all_defs).cloned();
                        let main_model = self.model();

                        // Run explore subagent synchronously
                        let summary = {
                            use crate::permissions::PermissionManager;
                            use crate::cli::headless::run_headless;

                            let _system_prompt = explore_def.map(|d| d.system_prompt)
                                .unwrap_or_else(|| "You are an expert code explorer. Be concise and precise.".to_string());

                            let req = crate::agent::client::CreateAgentRequest {
                                name: Some("init-explore".to_string()),
                                model: main_model,
                                description: Some("Ephemeral init analysis".to_string()),
                                system_prompt: Some("You are an expert code explorer. Be concise and precise.".to_string()),
                                memory_blocks: vec![],
                                tool_ids: vec![],
                            };
                            match client.create_agent(req).await {
                                Ok(sub) => {
                                    let perm = PermissionManager::default();
                                    let result = run_headless(&client, &sub.id, &explore_prompt, &perm, &crate::mcp::McpManager::empty()).await;
                                    let _ = client.delete_agent(&sub.id).await;
                                    result.map(|(s, _)| s).unwrap_or_else(|e| format!("Analysis failed: {e}"))
                                }
                                Err(e) => format!("Could not spawn explore agent: {e}"),
                            }
                        };

                        // Write summary into project memory block
                        let _ = self.client.upsert_memory(&agent_id, "project", &summary, None).await;

                        // Tell the main agent what was discovered
                        let init_prompt = format!(
                            "[/init completed] Project analysis summary:\n\n{summary}\n\n\
                             I've stored this in your 'project' memory block. \
                             Acknowledge and summarise what you learned in 2-3 sentences."
                        );
                        self.agent_turn(&mut stdout, &init_prompt).await?;
                        let _ = self.app.lock().unwrap().commit_streaming();
                    }

                    SlashCmd::Remember(text) => {
                        // Route through the agent — it decides what to store and where.
                        // This matches Letta's /remember behaviour exactly.
                        let msg = if text.is_empty() {
                            "[/remember] Please review our recent conversation and update your \
                             memory blocks with anything important you've learned about me, \
                             my preferences, or this project."
                                .to_string()
                        } else {
                            format!("[/remember] {text}")
                        };
                        self.agent_turn(&mut stdout, &msg).await?;
                        let _ = self.app.lock().unwrap().commit_streaming();
                    }

                    SlashCmd::Memory => {
                        // Parse subcommand from the raw input line
                        let raw = input.trim();
                        let mem_arg = raw.strip_prefix("/memory").unwrap_or("").trim().to_string();
                        let parts: Vec<&str> = mem_arg.splitn(3, ' ').collect();
                        let sub = parts.first().copied().unwrap_or("");

                        match sub {
                            // /memory view <label> — show full value untruncated
                            "view" | "show" if parts.len() >= 2 => {
                                let label = parts[1];
                                let id = self.agent_id();
                                match self.client.get_memory(&id).await {
                                    Ok(blocks) => {
                                        if let Some(b) = blocks.iter().find(|b| b.label == label) {
                                            self.tui_blank();
                                            self.tui_hdr(format!("  [{label}]"));
                                            if let Some(desc) = &b.description {
                                                if !desc.is_empty() { self.tui_dim(format!("  {desc}")); }
                                            }
                                            self.tui_blank();
                                            if b.value.is_empty() {
                                                self.tui_dim("  (empty)");
                                            } else {
                                                for ln in b.value.lines() {
                                                    self.tui_sys(ln.to_string());
                                                }
                                            }
                                        } else {
                                            self.tui_err(format!("Block '{label}' not found"));
                                        }
                                    }
                                    Err(e) => self.tui_err(e.to_string()),
                                }
                            }
                            // /memory set <label> <value>
                            "set" if parts.len() >= 3 => {
                                let label = parts[1];
                                let value = parts[2..].join(" ");
                                let id = self.agent_id();
                                match self.client.upsert_memory(&id, label, &value, None).await {
                                    Ok(_) => self.tui_ok(format!("  ✓ [{label}] updated")),
                                    Err(e) => self.tui_err(e.to_string()),
                                }
                            }
                            // /memory delete <label>
                            "delete" | "del" | "rm" if parts.len() >= 2 => {
                                let label = parts[1];
                                let id = self.agent_id();
                                match self.client.delete_memory(&id, label).await {
                                    Ok(_) => self.tui_ok(format!("  ✓ [{label}] deleted")),
                                    Err(e) => self.tui_err(e.to_string()),
                                }
                            }
                            // /memory edit <label> — inline multi-line editor via QuestionWidget
                            "edit" if parts.len() >= 2 => {
                                let label = parts[1];
                                let id = self.agent_id();
                                let current = self.client.get_memory(&id).await
                                    .unwrap_or_default()
                                    .into_iter()
                                    .find(|b| b.label == label)
                                    .map(|b| b.value)
                                    .unwrap_or_default();
                                use crate::ui::question::{Question, QuestionOption, QuestionWidget};
                                let opts = vec![
                                    QuestionOption { label: format!("Keep: {}…", current.chars().take(60).collect::<String>()), description: String::new() },
                                    QuestionOption { label: "Clear (erase block)".to_string(), description: String::new() },
                                ];
                                let q = Question {
                                    header: "Edit memory",
                                    text: &format!("Type new value for [{label}] or pick action:"),
                                    options: &opts, multi_select: false, allow_other: true, progress: None,
                                };
                                let ans = {
                                    let mut app = self.app.lock().unwrap();
                                    let r = QuestionWidget::ask(&mut app.terminal, &q)?;
                                    let _ = app.draw();
                                    r
                                };
                                if let Some(ref a) = ans {
                                    let val = a.as_str();
                                    let new_value = if val.starts_with("Clear") { String::new() }
                                        else if val.starts_with("Keep") { current }
                                        else { val.to_string() };
                                    match self.client.upsert_memory(&id, label, &new_value, None).await {
                                        Ok(_) => self.tui_ok(format!("  ✓ [{label}] updated")),
                                        Err(e) => self.tui_err(e.to_string()),
                                    }
                                }
                            }
                            // /memory history <label> — show last 5 revisions
                            "history" if parts.len() >= 2 => {
                                let label = parts[1];
                                let id = self.agent_id();
                                match self.client.list_memory_history(&id, label, 5).await {
                                    Ok(revs) if revs.is_empty() => {
                                        let _ = self.app.lock().unwrap().push(RenderLine::SystemMsg(
                                            format!("  [{label}] no history recorded yet")
                                        ));
                                    }
                                    Ok(revs) => {
                                        let _ = self.app.lock().unwrap().push(RenderLine::Blank);
                                        for (i, rev) in revs.iter().enumerate() {
                                            let rev_id = rev["id"].as_str().unwrap_or("");
                                            let ts = rev["updated_at"].as_i64().unwrap_or(0);
                                            let val = rev["value"].as_str().unwrap_or("");
                                            let preview: String = val.chars().take(120).collect();
                                            let ellipsis = if val.len() > 120 { "…" } else { "" };
                                            let _ = self.app.lock().unwrap().push(RenderLine::SystemMsg(
                                                format!("  [{i}] {ts}  id={rev_id}")
                                            ));
                                            let _ = self.app.lock().unwrap().push(RenderLine::SystemMsg(
                                                format!("      {preview}{ellipsis}")
                                            ));
                                            let _ = self.app.lock().unwrap().push(RenderLine::Blank);
                                        }
                                        let _ = self.app.lock().unwrap().push(RenderLine::SystemMsg(
                                            format!("  Use: /memory restore {label} <id>")
                                        ));
                                    }
                                    Err(e) => {
                                        let _ = self.app.lock().unwrap().push(RenderLine::ErrorMsg(format!("  ✗ {e}")));
                                    }
                                }
                            }
                            // /memory restore <label> <rev_id>
                            "restore" if parts.len() >= 3 => {
                                let label = parts[1];
                                let rev_id = parts[2];
                                let id = self.agent_id();
                                match self.client.restore_memory(&id, label, rev_id).await {
                                    Ok(_) => {
                                        let _ = self.app.lock().unwrap().push(RenderLine::SystemMsg(
                                            format!("  ✓ [{label}] restored to revision {rev_id}")
                                        ));
                                    }
                                    Err(e) => {
                                        let _ = self.app.lock().unwrap().push(RenderLine::ErrorMsg(format!("  ✗ {e}")));
                                    }
                                }
                            }
                            // /memory (list)
                            _ => {
                                match self.client.get_memory(&self.agent_id()).await {
                                    Ok(blocks) if blocks.is_empty() => {
                                        self.tui_dim("  (no memory blocks)");
                                        self.tui_dim("  Run /init to populate, or use update_memory tool");
                                    }
                                    Ok(blocks) => {
                                        self.tui_blank();
                                        for b in &blocks {
                                            self.tui_hdr(format!("  [{}]", b.label));
                                            if let Some(desc) = &b.description {
                                                if !desc.is_empty() { self.tui_dim(format!("  {desc}")); }
                                            }
                                            self.tui_blank();
                                            if b.value.is_empty() {
                                                self.tui_dim("  (empty)");
                                            } else {
                                                let preview: String = b.value.chars().take(300).collect();
                                                let ellipsis = if b.value.len() > 300 { "…  (/memory view to see all)" } else { "" };
                                                self.tui_sys(format!("  {preview}{ellipsis}"));
                                            }
                                            self.tui_blank();
                                        }
                                    }
                                    Err(e) => self.tui_err(e.to_string()),
                                }
                            }
                        }
                    }

                    SlashCmd::Search(query) => {
                        match self.client.search_messages(&self.agent_id(), &query).await {
                            Ok(msgs) if msgs.is_empty() => {
                                self.tui_dim(format!("  No results for '{query}'"));
                            }
                            Ok(msgs) => {
                                self.tui_blank();
                                self.tui_hdr(format!("  {} result(s) for '{query}':", msgs.len()));
                                self.tui_blank();
                                for m in msgs.iter().take(10) {
                                    let role = m["role"].as_str().unwrap_or("?");
                                    let content = m["content"]["content"].as_str()
                                        .or_else(|| m["content"].as_str())
                                        .unwrap_or("");
                                    let preview: String = content.chars().take(120).collect();
                                    let ellipsis = if content.len() > 120 { "…" } else { "" };
                                    self.tui_dim(format!("  [{role}]  {preview}{ellipsis}"));
                                }
                            }
                            Err(e) => self.tui_err(e.to_string()),
                        }
                    }

                    SlashCmd::Skills(arg) => {
                        let sub = arg.as_deref().unwrap_or("list");
                        let (sub_cmd, sub_arg) = sub.splitn(2, ' ')
                            .collect::<Vec<_>>()
                            .split_first()
                            .map(|(c, r)| (*c, r.join(" ")))
                            .unwrap_or(("list", String::new()));

                        match sub_cmd {
                            "list" | "" => {
                                let skills = self.skills.lock().unwrap();
                                if skills.is_empty() {
                                    self.tui_dim("  No skills loaded.");
                                    self.tui_dim("  Create one: /skills create <name>");
                                    self.tui_dim("  Skills dirs: .skills/  ~/.cade/skills/  ~/.cade/agents/<id>/skills/");
                                } else {
                                    self.tui_blank();
                                    self.tui_hdr(format!("  Skills ({} loaded):", skills.len()));
                                    self.tui_blank();
                                    for s in skills.iter() {
                                        let cat = s.category.as_deref()
                                            .map(|c| format!("[{}]", c))
                                            .unwrap_or_default();
                                        self.tui_sys(format!("  {:<10} {:<28} {:<12} {}",
                                            format!("[{}]", s.scope), s.id, cat, s.description));
                                    }
                                    self.tui_blank();
                                    self.tui_dim("  Agent uses load_skill(<id>) to load full content on-demand.");
                                }
                            }

                            "create" => {
                                let name_raw = sub_arg.trim().to_string();
                                if name_raw.is_empty() {
                                    self.tui_dim("  Usage: /skills create <name>");
                                } else {
                                    let slug: String = name_raw
                                        .to_lowercase()
                                        .chars()
                                        .map(|c| if c.is_alphanumeric() { c } else { '-' })
                                        .collect::<String>()
                                        .trim_matches('-')
                                        .to_string();
                                    let skill_dir = self.skills_dir.join(&slug);
                                    let skill_file = skill_dir.join("SKILL.MD");
                                    if skill_file.exists() {
                                        self.tui_err(format!("Skill '{}' already exists: {}", slug, skill_file.display()));
                                    } else {
                                        match std::fs::create_dir_all(&skill_dir) {
                                            Ok(_) => {
                                                let title: String = slug.replace('-', " ")
                                                    .split_whitespace()
                                                    .map(|w| {
                                                        let mut c = w.chars();
                                                        match c.next() {
                                                            None => String::new(),
                                                            Some(f) => f.to_uppercase().collect::<String>() + c.as_str(),
                                                        }
                                                    })
                                                    .collect::<Vec<_>>()
                                                    .join(" ");
                                                let template = format!(
                                                    "---\nname: {title}\ndescription: One-line description of what this skill does\ncategory: general\ntags: []\n---\n\n\
                                                    # {title}\n\nDescribe the skill here. This text is injected into the agent's\n\
                                                    system prompt when this skill is loaded.\n\n\
                                                    You can use markdown, code blocks, examples, step-by-step instructions, etc.\n"
                                                );
                                                match std::fs::write(&skill_file, template) {
                                                    Ok(_) => {
                                                        self.tui_ok(format!("  ✓ Created: {}", skill_file.display()));
                                                        self.tui_dim("  Edit the file, then run /skills reload to activate it.");
                                                    }
                                                    Err(e) => self.tui_err(format!("Failed to write skill file: {e}")),
                                                }
                                            }
                                            Err(e) => self.tui_err(format!("Failed to create directory: {e}")),
                                        }
                                    }
                                }
                            }

                            "show" => {
                                let id = sub_arg.trim();
                                let skills = self.skills.lock().unwrap();
                                match skills.iter().find(|s| s.id == id) {
                                    None => {
                                        self.tui_dim(format!("  Skill '{id}' not found. Run /skills to list."));
                                    }
                                    Some(s) => {
                                        self.tui_blank();
                                        self.tui_hdr(format!("  [{id}]"));
                                        self.tui_sys(format!("  Name       : {}", s.name));
                                        self.tui_sys(format!("  Description: {}", s.description));
                                        if let Some(cat) = &s.category {
                                            self.tui_sys(format!("  Category   : {cat}"));
                                        }
                                        if !s.tags.is_empty() {
                                            self.tui_sys(format!("  Tags       : {}", s.tags.join(", ")));
                                        }
                                        self.tui_blank();
                                        self.tui_dim("---");
                                        for ln in s.body.lines() { self.tui_sys(ln.to_string()); }
                                        self.tui_dim("---");
                                    }
                                }
                            }

                            "reload" => {
                                {
                                        let new_skills = crate::skills::discover_all_skills(&self.cwd, None, None);
                                        let prev_count = self.skills.lock().unwrap().len();
                                        let new_count = new_skills.len();
                                        let agent_id = self.agent_id();

                                        let existing = self.client.get_memory(&agent_id).await.unwrap_or_default();
                                        for block in &existing {
                                            if block.label.starts_with("skill:") {
                                                let _ = self.client.delete_memory(&agent_id, &block.label).await;
                                            }
                                        }
                                        let mut names = vec![];
                                        for skill in &new_skills {
                                            let label = format!("skill:{}", skill.id);
                                            let _ = self.client.upsert_memory(&agent_id, &label, &skill.to_context_block(), None).await;
                                            names.push(skill.name.clone());
                                        }

                                        let listing = crate::skills::skills_listing(&new_skills);
                                        let _ = self.client.upsert_memory(
                                            &agent_id, "skills", listing.as_deref().unwrap_or(""), None
                                        ).await;

                                        *self.skills.lock().unwrap() = new_skills;

                                        self.tui_ok(format!("  ✓ Reloaded: {new_count} skills (was {prev_count})"));

                                        if new_count > 0 {
                                            let list = names.join(", ");
                                            let notify = format!(
                                                "[System: Skills reloaded. Now active: {list}. \
                                                 Use load_skill(id) to load any skill's full content.]"
                                            );
                                            self.agent_turn(&mut stdout, &notify).await?;
                                            let _ = self.app.lock().unwrap().commit_streaming();
                                        }
                                }
                            }

                            other => {
                                self.tui_err(format!("Unknown /skills subcommand: '{other}'"));
                                self.tui_dim("  Usage: /skills [list | create <name> | show <id> | reload]");
                            }
                        }
                    }

                    SlashCmd::Subagents => {
                        let all = discover_all_subagents(&self.cwd);
                        self.tui_blank();
                        self.tui_hdr(format!("  Available subagents ({}):", all.len()));
                        self.tui_blank();
                        for def in &all {
                            self.tui_sys(def.summary());
                        }
                        self.tui_blank();
                        self.tui_dim("  Usage: ask the agent to run_subagent(type, task)");
                        self.tui_dim("  Custom: create .cade/agents/<name>.md in this project");
                        self.tui_dim("  Global: create ~/.cade/agents/<name>.md");
                    }

                    SlashCmd::Providers => {
                        match self.client.list_providers().await {
                            Ok(body) => {
                                let empty = vec![];
                                let providers = body["providers"].as_array().unwrap_or(&empty);
                                self.tui_blank();
                                self.tui_hdr(format!("  Configured providers ({}):", providers.len()));
                                for p in providers {
                                    let name    = p["name"].as_str().unwrap_or("?");
                                    let kind    = p["kind"].as_str().unwrap_or("?");
                                    let live    = p["live"].as_bool().unwrap_or(false);
                                    let source  = p["source"].as_str().unwrap_or("db");
                                    let enabled = p["enabled"].as_bool().unwrap_or(true);
                                    let status  = if live { "✓ live" } else { "✗ offline" };
                                    let display_name = if enabled { name.to_string() } else { format!("{name} (disabled)") };
                                    if live {
                                        self.tui_ok(format!("  {status:<10} {display_name:<18} [{kind}] ({source})"));
                                    } else {
                                        self.tui_err(format!("  {status:<10} {display_name:<18} [{kind}] ({source})"));
                                    }
                                }
                                self.tui_blank();
                                self.tui_dim("  /connect <name>    — add a provider");
                                self.tui_dim("  /disconnect <name> — remove a provider");
                                let presets = self.client.list_provider_presets().await;
                                if !presets.is_empty() {
                                    self.tui_dim("  OpenAI-compatible presets:");
                                    for p in &presets {
                                        let n = p["name"].as_str().unwrap_or("?");
                                        let u = p["base_url"].as_str().unwrap_or("?");
                                        self.tui_dim(format!("    /connect {n:<14} — {u}"));
                                    }
                                }
                                self.tui_blank();
                            }
                            Err(e) => self.tui_err(e.to_string()),
                        }
                    }

                    SlashCmd::Connect(preset) => {
                        self.handle_connect(preset, &mut stdout).await?;
                    }

                    SlashCmd::Disconnect(name) => {
                        if name.is_empty() {
                            self.tui_err("/disconnect requires a provider name");
                        } else {
                            self.tui_dim(format!("  Disconnecting provider '{name}'…"));
                            match self.client.remove_provider(&name).await {
                                Ok(_)  => self.tui_ok(format!("  ✓ Provider '{name}' removed")),
                                Err(e) => self.tui_err(e.to_string()),
                            }
                        }
                    }

                    SlashCmd::Permissions => {
                        let mode  = self.permissions.mode();
                        let allow = self.permissions.allow_rules();
                        let deny  = self.permissions.deny_rules();

                        let (icon, label, _) = mode_display(mode);
                        let mode_hint = match mode {
                            crate::permissions::PermissionMode::Default           => "ask before each tool call",
                            crate::permissions::PermissionMode::AcceptEdits       => "file edits auto-approved; Bash still prompts",
                            crate::permissions::PermissionMode::Plan              => "read-only; write operations blocked",
                            crate::permissions::PermissionMode::BypassPermissions => "all tools auto-approved (deny rules still apply)",
                        };
                        self.tui_blank();
                        self.tui_hdr(format!("  Mode: {icon} {label}  —  {mode_hint}"));
                        self.tui_blank();

                        if allow.is_empty() && deny.is_empty() {
                            self.tui_dim("  No allow/deny rules active.");
                        } else {
                            if !allow.is_empty() {
                                self.tui_ok(format!("  Allow rules ({}):", allow.len()));
                                for r in &allow {
                                    self.tui_dim(format!("    {:<12} {}", r.tool(), r.arg_display()));
                                }
                                let _ = self.app.lock().unwrap().push(RenderLine::Blank);
                            }
                            if !deny.is_empty() {
                                self.tui_err(format!("  Deny rules ({}):", deny.len()));
                                for r in &deny {
                                    self.tui_dim(format!("    {:<12} {}", r.tool(), r.arg_display()));
                                }
                                self.tui_blank();
                            }
                        }
                        self.tui_dim("  /approve-always <pattern>    /deny-always <pattern>");
                        self.tui_dim("  Pattern:  Bash(cargo test)  ·  Read(src/**)  ·  Bash(rm -rf:*)");
                    }

                    SlashCmd::ApproveAlways(pattern) => {
                        if pattern.is_empty() {
                            self.tui_dim("  /approve-always <pattern>");
                            self.tui_dim("  Examples:  Bash(cargo test)  Read(src/**)  Bash(git commit:*)  Bash");
                        } else if let Some(rule) = crate::permissions::PermissionRule::parse(&pattern) {
                            self.permissions.add_allow_rule(rule.clone());
                            self.tui_ok(format!("  ✓ Allow  {:<12} {}", rule.tool(), rule.arg_display()));
                            use crate::ui::question::{Question, QuestionOption, QuestionWidget};
                            let opts = vec![
                                QuestionOption { label: "Yes — save to settings.json".to_string(), description: String::new() },
                                QuestionOption { label: "No — session only".to_string(), description: String::new() },
                            ];
                            let q = Question { header: "Save rule?", text: "Persist this rule to settings.json?",
                                options: &opts, multi_select: false, allow_other: false, progress: None };
                            let save = {
                                let mut app = self.app.lock().unwrap();
                                let r = QuestionWidget::ask(&mut app.terminal, &q)?;
                                let _ = app.draw();
                                matches!(r, Some(ref a) if a.as_str().starts_with("Yes"))
                            };
                            if save {
                                let mut settings = self.settings.lock().unwrap();
                                match settings.save_allow_rule(&pattern) {
                                    Ok(_)  => self.tui_ok("  ✓ Saved"),
                                    Err(e) => self.tui_err(e.to_string()),
                                }
                            }
                        } else {
                            self.tui_err(format!("invalid pattern: {pattern:?}  Expected: Tool  or  Tool(arg)  or  Tool(prefix:*)"));
                        }
                    }

                    SlashCmd::DenyAlways(pattern) => {
                        if pattern.is_empty() {
                            self.tui_dim("  /deny-always <pattern>");
                            self.tui_dim("  Examples:  Bash(rm -rf:*)  Bash(git push --force)  Bash");
                        } else if let Some(rule) = crate::permissions::PermissionRule::parse(&pattern) {
                            self.permissions.add_deny_rule(rule.clone());
                            self.tui_err(format!("  ✗ Deny   {:<12} {}", rule.tool(), rule.arg_display()));
                            use crate::ui::question::{Question, QuestionOption, QuestionWidget};
                            let opts = vec![
                                QuestionOption { label: "Yes — save to settings.json".to_string(), description: String::new() },
                                QuestionOption { label: "No — session only".to_string(), description: String::new() },
                            ];
                            let q = Question { header: "Save rule?", text: "Persist this rule to settings.json?",
                                options: &opts, multi_select: false, allow_other: false, progress: None };
                            let save = {
                                let mut app = self.app.lock().unwrap();
                                let r = QuestionWidget::ask(&mut app.terminal, &q)?;
                                let _ = app.draw();
                                matches!(r, Some(ref a) if a.as_str().starts_with("Yes"))
                            };
                            if save {
                                let mut settings = self.settings.lock().unwrap();
                                match settings.save_deny_rule(&pattern) {
                                    Ok(_)  => self.tui_ok("  ✓ Saved"),
                                    Err(e) => self.tui_err(e.to_string()),
                                }
                            }
                        } else {
                            self.tui_err(format!("invalid pattern: {pattern:?}  Expected: Tool  or  Tool(arg)  or  Tool(prefix:*)"));
                        }
                    }

                    SlashCmd::Hooks => {
                        let merged = self.settings.lock().unwrap().merged_hooks();
                        self.tui_blank();
                        if merged.is_empty() {
                            self.tui_dim("  No hooks configured.");
                            self.tui_dim("  Configure in ~/.cade/settings.json or .cade/settings.json");
                            self.tui_blank();
                            self.tui_dim("  Example: { \"hooks\": { \"PreToolUse\": [{ \"matcher\": \"Bash\", \"hooks\": [{ \"type\": \"command\", \"command\": \"./validate.sh\" }] }] } }");
                            self.tui_dim("  Exit codes:  0=allow  1=log+continue  2=block (stderr→agent)");
                        } else {
                            self.tui_hdr("  Hooks");
                            self.tui_blank();
                            let show_section = |name: &str, entries: &[crate::settings::manager::HookEntry]| {
                                if !entries.is_empty() {
                                    self.tui_hdr(format!("  {name}  ({}):", entries.len()));
                                    for entry in entries {
                                        let m = entry.matcher.as_deref().unwrap_or("*");
                                        self.tui_dim(format!("    matcher: {m}"));
                                        for hook in &entry.hooks {
                                            self.tui_dim(format!("      {hook}"));
                                        }
                                    }
                                    self.tui_blank();
                                }
                            };
                            show_section("PreToolUse",         &merged.pre_tool_use);
                            show_section("PostToolUse",        &merged.post_tool_use);
                            show_section("PostToolUseFailure", &merged.post_tool_use_failure);
                            show_section("PermissionRequest",  &merged.permission_request);
                            show_section("UserPromptSubmit",   &merged.user_prompt_submit);
                            show_section("Stop",               &merged.stop);
                            show_section("SubagentStop",       &merged.subagent_stop);
                            show_section("SessionStart",       &merged.session_start);
                            show_section("SessionEnd",         &merged.session_end);
                            show_section("Notification",       &merged.notification);
                            self.tui_dim("  Config: ~/.cade/settings.json  ·  .cade/settings.json  ·  .cade/settings.local.json");
                        }
                    }

                    SlashCmd::Rename(new_name) => {
                        let id = self.agent_id();
                        let new_name = new_name.trim().to_string();
                        let name = if new_name.is_empty() {
                            // Prompt for name via QuestionWidget
                            use crate::ui::question::{Question, QuestionOption, QuestionWidget};
                            let opts = vec![QuestionOption { label: "Cancel".to_string(), description: String::new() }];
                            let q = Question { header: "Rename agent", text: "Enter new agent name:",
                                options: &opts, multi_select: false, allow_other: true, progress: None };
                            let ans = {
                                let mut app = self.app.lock().unwrap();
                                let r = QuestionWidget::ask(&mut app.terminal, &q)?;
                                let _ = app.draw();
                                r
                            };
                            match ans {
                                Some(ref a) if a.as_str() != "Cancel" && !a.as_str().is_empty() => a.as_str().to_string(),
                                _ => String::new(),
                            }
                        } else {
                            new_name
                        };
                        if name.is_empty() {
                            self.tui_dim("  (cancelled)");
                        } else {
                            match self.client.rename_agent(&id, &name).await {
                                Ok(_) => {
                                    *self.agent_name.lock().unwrap() = name.clone();
                                    self.tui_ok(format!("  ✓ Renamed to: {name}"));
                                }
                                Err(e) => self.tui_err(e.to_string()),
                            }
                        }
                    }

                    SlashCmd::Toolset(arg) => {
                        let old_toolset = *self.current_toolset.lock().unwrap();
                        let new_toolset = if let Some(name) = arg.as_deref() {
                            match crate::toolsets::Toolset::from_str(name) {
                                Some(t) => t,
                                None => {
                                    self.tui_dim("  Toolsets: default | codex | gemini");
                                    continue;
                                }
                            }
                        } else {
                            self.tui_hdr(format!("  Current toolset: {old_toolset:?}"));
                            self.tui_dim("  /toolset default | codex | gemini");
                            continue;
                        };
                        if new_toolset != old_toolset {
                            *self.current_toolset.lock().unwrap() = new_toolset;
                            self.tui_ok(format!("  ✓ Toolset: {new_toolset:?}"));
                        } else {
                            self.tui_dim(format!("  Toolset already: {new_toolset:?}"));
                        }
                    }

                    SlashCmd::Feedback => {
                        self.tui_hdr("  Report issues or give feedback:");
                        self.tui_sys("  https://github.com/EzekTec-Inc/CADE/issues");
                    }
                }
                continue;
            }

            // UserPromptSubmit hook — can block the turn
            if let crate::hooks::HookOutcome::Block { reason } =
                self.hooks.user_prompt_submit(&input).await
            {
                self.tui_sys(format!("  ⚠ Hook blocked prompt: {reason}"));
                continue;
            }

            // Send to agent and handle tool loop
            self.agent_turn(&mut stdout, &input).await?;
            let _ = self.app.lock().unwrap().commit_streaming();
        }

        // SessionEnd hook (non-blocking)
        self.hooks.session_end(&self.agent_id()).await;

        Ok(())
    }

    /// Build environment context injected on the first user turn of each session.
    fn build_env_context(&self) -> String {
        use std::process::Command;

        let now = chrono::Local::now().format("%Y-%m-%d %H:%M %Z");

        // OS / kernel
        let os_info = {
            let uname = Command::new("uname").arg("-sr").output()
                .ok()
                .and_then(|o| String::from_utf8(o.stdout).ok())
                .unwrap_or_default();
            // Try /etc/os-release for distro name
            let distro = std::fs::read_to_string("/etc/os-release")
                .unwrap_or_default()
                .lines()
                .find(|l| l.starts_with("PRETTY_NAME="))
                .map(|l| l.trim_start_matches("PRETTY_NAME=").trim_matches('"').to_string())
                .unwrap_or_default();
            if distro.is_empty() {
                uname.trim().to_string()
            } else {
                format!("{} ({})", uname.trim(), distro)
            }
        };

        // CWD
        let cwd = self.cwd.display().to_string();

        // Git info
        let git_info = {
            let branch = Command::new("git")
                .args(["-C", &cwd, "rev-parse", "--abbrev-ref", "HEAD"])
                .output()
                .ok()
                .and_then(|o| if o.status.success() {
                    String::from_utf8(o.stdout).ok()
                } else { None })
                .map(|s| s.trim().to_string());

            let status = Command::new("git")
                .args(["-C", &cwd, "status", "--porcelain"])
                .output()
                .ok()
                .and_then(|o| if o.status.success() {
                    String::from_utf8(o.stdout).ok()
                } else { None });

            match (branch, status) {
                (Some(b), Some(s)) if !b.is_empty() => {
                    let lines: Vec<&str> = s.lines().collect();
                    if lines.is_empty() {
                        format!("branch={b}, clean")
                    } else {
                        format!("branch={b}, {} uncommitted change{}", lines.len(),
                            if lines.len() == 1 { "" } else { "s" })
                    }
                }
                _ => String::new(),
            }
        };

        let mut parts = vec![
            format!("Date:   {now}"),
            format!("OS:     {os_info}"),
            format!("CWD:    {cwd}"),
        ];
        if !git_info.is_empty() {
            parts.push(format!("Git:    {git_info}"));
        }
        format!("<environment>\n{}\n</environment>", parts.join("\n"))
    }

    /// Send a user message and drive the tool-call loop with live SSE streaming.
    async fn agent_turn(&mut self, stdout: &mut io::Stdout, input: &str) -> Result<()> {
        use std::sync::atomic::Ordering;

        let turn_start = std::time::Instant::now();
        let in_tok_before  = self.session_input_tokens.load(Ordering::SeqCst);
        let out_tok_before = self.session_output_tokens.load(Ordering::SeqCst);

        // Reset cancel flag and spawn SIGINT watcher for the duration of this turn
        self.cancel_turn.store(false, Ordering::SeqCst);
        let cancel_flag = self.cancel_turn.clone();
        let _sigint_guard = tokio::spawn(async move {
            #[cfg(unix)]
            {
                use tokio::signal::unix::{signal, SignalKind};
                if let Ok(mut sig) = signal(SignalKind::interrupt()) {
                    sig.recv().await;
                    cancel_flag.store(true, Ordering::SeqCst);
                }
            }
        });

        // On the first real turn, prefix with environment context
        let effective_input = if self.first_turn.compare_exchange(
            true, false, Ordering::SeqCst, Ordering::SeqCst
        ).is_ok() {
            let env = self.build_env_context();
            // Explicitly instruct the agent not to turn the env context into a
            // self-introduction. Without this it often opens with "I am CADE…"
            format!("{env}\n\n<system>Do not introduce yourself. Answer the user's message directly.</system>\n\n{input}")
        } else {
            input.to_string()
        };

        // ── Thinking animation ────────────────────────────────────────────────
        let bar_text = self.app.lock().unwrap().start_thinking(
            "assessing… (esc to interrupt · 0s · 0↑)"
        );

        // Redraw tick task — updates the spinner animation and assessing timer.
        let tick_app     = self.app.clone();
        let tick_cancel  = self.cancel_turn.clone();
        let tick_tokens  = self.session_output_tokens.clone();
        let tick_base    = out_tok_before;
        let tick_start   = turn_start;
        let tick_bar     = bar_text.clone();
        let tick_handle  = tokio::spawn(async move {
            use crossterm::event::{EventStream, Event, KeyCode};
            use futures::StreamExt;
            let mut reader = EventStream::new();
            loop {
                tokio::select! {
                    _ = tokio::time::sleep(tokio::time::Duration::from_millis(100)) => {
                        if tick_cancel.load(Ordering::SeqCst) { break; }
                        // Update assessing text once per second
                        let secs = tick_start.elapsed().as_secs();
                        let toks = tick_tokens.load(Ordering::SeqCst).saturating_sub(tick_base);
                        {
                            let cur = tick_bar.lock().unwrap().clone();
                            if cur.starts_with("assessing") || cur.starts_with("CADE thinking") {
                                *tick_bar.lock().unwrap() =
                                    format!("assessing… (esc to interrupt · {secs}s · {toks}↑)");
                            }
                        }
                        if let Ok(mut app) = tick_app.try_lock() {
                            let _ = app.draw();
                        }
                    }
                    Some(Ok(evt)) = reader.next() => {
                        if tick_cancel.load(Ordering::SeqCst) { break; }
                        if let Ok(mut app) = tick_app.try_lock() {
                            use crossterm::event::MouseEventKind;
                            match evt {
                                Event::Key(k) => match k.code {
                                    KeyCode::Char('K') => { app.scroll = app.scroll.saturating_add(10); let _ = app.draw(); }
                                    KeyCode::Char('J') => { app.scroll = app.scroll.saturating_sub(10); let _ = app.draw(); }
                                    _ => {}
                                },
                                Event::Mouse(m) => match m.kind {
                                    MouseEventKind::ScrollUp   => { app.scroll = app.scroll.saturating_add(3); let _ = app.draw(); }
                                    MouseEventKind::ScrollDown => { app.scroll = app.scroll.saturating_sub(3); let _ = app.draw(); }
                                    _ => {}
                                },
                                _ => {}
                            }
                        }
                    }
                }
            }
        });

        let messages = self.stream_turn(
            stdout, &effective_input, false, "", "",
            None,
            Some(bar_text.clone()),
        ).await;

        let messages = messages?;

        // Clear cancel flag after turn completes
        self.cancel_turn.store(false, Ordering::SeqCst);
        self.dispatch_tool_calls(stdout, messages, input, Some(bar_text)).await?;

        // ── Stop thinking animation ───────────────────────────────────────────
        tick_handle.abort();
        let _ = tick_handle.await;
        let secs = self.app.lock().unwrap().stop_thinking();
        let in_delta  = self.session_input_tokens.load(Ordering::SeqCst).saturating_sub(in_tok_before);
        let out_delta = self.session_output_tokens.load(Ordering::SeqCst).saturating_sub(out_tok_before);
        let summary = if secs >= 60 {
            format!("✻ Considered for {}m {}s · ↑{} ↓{} tokens", secs / 60, secs % 60, in_delta, out_delta)
        } else {
            format!("✻ Considered for {}s · ↑{} ↓{} tokens", secs, in_delta, out_delta)
        };
        self.app.lock().unwrap().set_last_status(Some(summary));
        let _ = self.app.lock().unwrap().draw();

        Ok(())
    }

    /// Stream one turn (user message or tool return) and render live.
    /// Returns the complete collected message list.
    ///
    /// `bar_text`: optional shared string updated by tool_call_message events
    /// to keep the ThinkingBar status current.
    async fn stream_turn(
        &self,
        _stdout: &mut io::Stdout,
        input: &str,
        is_tool_return: bool,
        tool_call_id: &str,
        tool_output: &str,
        _spinner: Option<std::sync::Arc<std::sync::atomic::AtomicBool>>,
        bar_text: Option<std::sync::Arc<std::sync::Mutex<String>>>,
    ) -> Result<Vec<CadeMessage>> {
        // ── Shared render state for the on_event closure ────────────────────────
        // OutputRenderer is on self but closures need Arcs for shared mutability.
        // We use two Arcs mirroring in_reasoning / in_assistant state so the closure
        // can track block transitions without touching &mut self.
        let in_reasoning = std::sync::Arc::new(std::sync::Mutex::new(false));
        let in_assistant = std::sync::Arc::new(std::sync::Mutex::new(false));
        let in_reasoning2 = in_reasoning.clone();
        let in_assistant2 = in_assistant.clone();

        // Clone Arcs before the move closure so conversation_id and run state can be saved
        let conv_arc     = self.conversation_id.clone();
        let session_arc  = self.session.clone();
        // Session-level token accumulators
        let sess_in_tok  = self.session_input_tokens.clone();
        let sess_out_tok = self.session_output_tokens.clone();
        // Track run_id/seq_id for crash recovery / reconnect
        let run_id_cell:   std::sync::Arc<std::sync::Mutex<Option<String>>> = Default::default();
        let seq_id_cell:   std::sync::Arc<std::sync::Mutex<Option<i64>>>   = Default::default();
        let run_id_cell2   = run_id_cell.clone();
        let seq_id_cell2   = seq_id_cell.clone();

        // Clone the Arc so the on_event closure can access TuiApp.
        let app_arc      = self.app.clone();
        let bar_text_arc = bar_text;

        let on_event = move |msg: &CadeMessage| {
            match msg.msg_type() {
                "stream_start" => {
                    if let Some(cid) = msg.data["conversation_id"].as_str() {
                        if !cid.is_empty() && conv_arc.lock().unwrap().as_deref() != Some(cid) {
                            let cid: String = cid.to_string();
                            *conv_arc.lock().unwrap() = Some(cid.clone());
                            if let Ok(mut s) = session_arc.lock() {
                                let _ = s.set_conversation(Some(cid));
                            }
                        }
                    }
                    if let Some(rid) = msg.run_id() {
                        *run_id_cell2.lock().unwrap() = Some(rid.to_string());
                    }
                }
                "reasoning_message" => {
                    if let Some(text) = msg.reasoning_text() {
                        let mut flag = in_reasoning2.lock().unwrap();
                        if !*flag { *flag = true; }
                        // Accumulate reasoning text; committed as collapsed header on done.
                        app_arc.lock().unwrap().push_reasoning_chunk(text);
                    }
                }
                "assistant_message" => {
                    {
                        let mut rflag = in_reasoning2.lock().unwrap();
                        if *rflag {
                            let _ = app_arc.lock().unwrap().commit_reasoning();
                            *rflag = false;
                        }
                    }
                    if let Some(text) = msg.assistant_text() {
                        if !text.is_empty() {
                            *in_assistant2.lock().unwrap() = true;
                            let _ = app_arc.lock().unwrap().push_streaming_chunk(text);
                            // Update thinking bar to show generation progress.
                            if let Some(ref bar) = bar_text_arc {
                                let words = app_arc.lock().unwrap()
                                    // count words in streaming_text via a snapshot
                                    .lines.len(); // rough proxy — update bar text
                                let cur = bar.lock().unwrap().clone();
                                if !cur.starts_with("●") {
                                    *bar.lock().unwrap() = "generating…".to_string();
                                }
                            }
                        }
                    }
                }
                "tool_call_message" => {
                    let _ = app_arc.lock().unwrap().commit_streaming();
                    *in_reasoning2.lock().unwrap() = false;
                    *in_assistant2.lock().unwrap() = false;
                    if let Some(ref bar) = bar_text_arc {
                        let tool_name = msg.data["tool_calls"][0]["function"]["name"]
                            .as_str().unwrap_or("tool");
                        let display = if let Some(pos) = tool_name.rfind("__") {
                            &tool_name[pos + 2..]
                        } else {
                            tool_name
                        };
                        *bar.lock().unwrap() = format!("● {}…", display);
                    }
                }
                "usage_statistics" => {
                    use std::sync::atomic::Ordering;
                    if let Some(n) = msg.data["input_tokens"].as_u64() {
                        sess_in_tok.fetch_add(n, Ordering::SeqCst);
                    }
                    if let Some(n) = msg.data["output_tokens"].as_u64() {
                        sess_out_tok.fetch_add(n, Ordering::SeqCst);
                    }
                }
                _ => {}
            }
            if let Some(s) = msg.seq_id() {
                *seq_id_cell2.lock().unwrap() = Some(s);
            }
        };

        let agent_id  = self.agent_id();
        let cancel    = &self.cancel_turn;

        // Helper: detect a user-triggered cancellation error vs a real error
        fn is_cancel(e: &anyhow::Error) -> bool { e.to_string() == "__cancelled__" }

        let conv_id   = self.conversation_id();
        let conv_ref  = conv_id.as_deref();

        let messages = if is_tool_return {
            match self
                .client
                .stream_tool_return_cancellable(&agent_id, tool_call_id, tool_output, false, conv_ref, on_event, Some(cancel))
                .await
            {
                Ok(m) => m,
                Err(e) if is_cancel(&e) => {
                    let mut app = self.app.lock().unwrap();
                    app.discard_streaming();
                    let _ = app.push(RenderLine::ErrorMsg("Turn interrupted".to_string()));
                    return Ok(vec![]);
                }
                Err(e) => {
                    let mut app = self.app.lock().unwrap();
                    app.discard_streaming();
                    let _ = app.push(RenderLine::ErrorMsg(e.to_string()));
                    return Ok(vec![]);
                }
            }
        } else {
            use std::sync::atomic::Ordering;
            let streaming = self.streaming_enabled.load(Ordering::SeqCst);
            if streaming {
                match self.client.stream_message_cancellable(&agent_id, input, conv_ref, on_event, Some(cancel)).await {
                    Ok(m) => m,
                    Err(e) if is_cancel(&e) => {
                        let mut app = self.app.lock().unwrap();
                        app.discard_streaming();
                        let _ = app.push(RenderLine::ErrorMsg("Turn interrupted".to_string()));
                        return Ok(vec![]);
                    }
                    Err(e) => {
                        let mut app = self.app.lock().unwrap();
                        app.discard_streaming();
                        let _ = app.push(RenderLine::ErrorMsg(e.to_string()));
                        return Ok(vec![]);
                    }
                }
            } else {
                // Non-streaming path — single HTTP request, print result at end
                match self.client.send_message(&agent_id, input).await {
                    Ok(msgs) => {
                        for msg in &msgs {
                            if let Some(text) = msg.assistant_text() {
                                if !text.is_empty() {
                                    let _ = self.app.lock().unwrap().push_streaming_chunk(text);
                                }
                            }
                        }
                        let _ = self.app.lock().unwrap().commit_streaming();
                        msgs
                    }
                    Err(e) => {
                        let _ = self.app.lock().unwrap().push(RenderLine::ErrorMsg(e.to_string()));
                        return Ok(vec![]);
                    }
                }
            }
        };

        // Commit any open streaming blocks after streaming ends.
        let _ = self.app.lock().unwrap().commit_streaming();

        // Save run_id + last seq_id for crash recovery / reconnect
        let saved_run_id  = run_id_cell.lock().unwrap().clone();
        let saved_seq_id  = seq_id_cell.lock().unwrap().clone();
        if saved_run_id.is_some() || saved_seq_id.is_some() {
            if let Ok(mut s) = self.session.lock() {
                let _ = s.set_run(saved_run_id, saved_seq_id);
            }
        }

        Ok(messages)
    }

    /// Collect tool calls from messages and execute them one by one.
    async fn dispatch_tool_calls(
        &self,
        stdout: &mut io::Stdout,
        messages: Vec<CadeMessage>,
        user_input: &str,
        bar_text: Option<std::sync::Arc<std::sync::Mutex<String>>>,
    ) -> Result<()> {
        let tool_calls: Vec<(String, String, serde_json::Value)> = messages
            .iter()
            .filter_map(|m| m.as_tool_call())
            .collect();

        if tool_calls.is_empty() {
            // No tool calls → agent has stopped. Collect final assistant text.
            let assistant_msg: String = messages.iter()
                .filter_map(|m| m.assistant_text())
                .collect::<Vec<_>>()
                .join(" ");

            // Stop hook — exit 2 feeds stderr back to agent as a continuation
            let stop_outcome = self.hooks.stop("end_turn", user_input, &assistant_msg).await;
            if let crate::hooks::HookOutcome::Block { reason } = stop_outcome {
                let _ = self.app.lock().unwrap().push(RenderLine::SystemMsg(
                    format!("  ⎿  Hook continuing: {reason}")
                ));
                // Feed the hook's stderr back to the agent as a new turn
                let follow_msgs = self.stream_turn(stdout, &reason, false, "", "", None, bar_text.clone()).await?;
                Box::pin(self.dispatch_tool_calls(stdout, follow_msgs, user_input, bar_text)).await?;
            }
            return Ok(());
        }

        for (call_id, tool_name, args) in tool_calls {
            // Update bar text for this tool execution
            if let Some(ref bar) = bar_text {
                let display = if let Some(pos) = tool_name.rfind("__") {
                    &tool_name[pos + 2..]
                } else {
                    &tool_name
                };
                *bar.lock().unwrap() = format!("● {}…", display);
            }

            let result = self.execute_tool(stdout, &call_id, &tool_name, &args).await?;

            // Stream the tool return and process any chained tool calls
            let follow = self
                .stream_turn(stdout, "", true, &call_id, &result.output, None, bar_text.clone())
                .await?;

            Box::pin(self.dispatch_tool_calls(stdout, follow, user_input, bar_text.clone())).await?;
        }

        Ok(())
    }

    /// Execute a single tool call, respecting permissions and printing status.
    async fn execute_tool(
        &self,
        stdout: &mut io::Stdout,
        call_id: &str,
        tool_name: &str,
        args: &serde_json::Value,
    ) -> Result<crate::tools::ToolResult> {
        // Build compact args preview — key arg depends on tool type
        let preview: String = {
            fn short(s: &str, n: usize) -> String {
                let s = s.trim();
                if s.chars().count() <= n { s.to_string() }
                else { format!("{}…", s.chars().take(n).collect::<String>()) }
            }
            let a = args;
            if let Some(cmd) = a["command"].as_str() {
                short(cmd, 80).to_string()
            } else if let Some(fp) = a["file_path"].as_str().or(a["path"].as_str()) {
                let extra = if let Some(old) = a["old_string"].as_str() {
                    format!("  \"{}\"", short(old, 40))
                } else if let Some(content) = a["content"].as_str() {
                    format!("  ({} chars)", content.len())
                } else { String::new() };
                format!("{fp}{extra}")
            } else if let Some(pat) = a["pattern"].as_str() {
                let in_path = a["path"].as_str().unwrap_or("");
                if in_path.is_empty() { format!("\"{}\"", short(pat, 60)) }
                else { format!("\"{}\" in {in_path}", short(pat, 40)) }
            } else if let Some(label) = a["label"].as_str() {
                let op = a["operation"].as_str().unwrap_or("set");
                format!("[{label}] ({op})")
            } else if let Some(id) = a["id"].as_str() {
                id.to_string()
            } else if let Some(task) = a["task"].as_str().or(a["prompt"].as_str()) {
                format!("\"{}\"", short(task, 60))
            } else if let Some(patch) = a["patch"].as_str() {
                format!("\"{}\"", short(patch, 60))
            } else {
                a.as_object().and_then(|m| {
                    m.values().find_map(|v| v.as_str()).map(|s| short(s, 60))
                }).unwrap_or_default()
            }
        };
        // Show tool call header.
        let _ = self.app.lock().unwrap().push(RenderLine::ToolCall {
            name:    tool_name.to_string(),
            preview: preview.clone(),
        });

        // Native tool intercepts (handled without going through generic dispatch)
        if tool_name == "update_memory" {
            return self.handle_update_memory(call_id, args).await;
        }
        if tool_name == "load_skill" {
            return self.handle_load_skill(call_id, args).await;
        }
        if tool_name == "install_skill" {
            return self.handle_install_skill(call_id, args, stdout).await;
        }
        if tool_name == "run_subagent" {
            return self.handle_run_subagent(call_id, args, stdout).await;
        }
        if tool_name == "ask_user_question" {
            return self.handle_ask_user_question(call_id, args).await;
        }

        // Permission check — plan mode / deny rules
        if self.permissions.is_blocked(tool_name, args) {
            let msg = self.permissions.block_reason(tool_name, args);
            let _ = self.app.lock().unwrap().push(RenderLine::ToolResult { is_error: true, content: msg.clone() });
            return Ok(crate::tools::ToolResult {
                tool_call_id: call_id.to_string(),
                tool_name: tool_name.to_string(),
                output: msg,
                is_error: true,
            });
        }

        if !self.permissions.auto_approve(tool_name, args) {
            // PermissionRequest hook — can block before showing prompt
            if let crate::hooks::HookOutcome::Block { reason } =
                self.hooks.permission_request(tool_name, args).await
            {
                let _ = self.app.lock().unwrap().push(RenderLine::ToolResult {
                    is_error: true,
                    content: format!("Hook denied: {reason}"),
                });
                return Ok(crate::tools::ToolResult {
                    tool_call_id: call_id.to_string(),
                    tool_name: tool_name.to_string(),
                    output: format!("Hook denied: {reason}"),
                    is_error: true,
                });
            }

            // Prompt for approval
            if !self.prompt_approval(stdout, tool_name, args)? {
                let msg = format!("Tool '{tool_name}' denied by user");
                let _ = self.app.lock().unwrap().push(RenderLine::ToolResult { is_error: true, content: msg.clone() });
                return Ok(crate::tools::ToolResult {
                    tool_call_id: call_id.to_string(),
                    tool_name: tool_name.to_string(),
                    output: msg,
                    is_error: true,
                });
            }
        }

        // PreToolUse hook — can block execution
        if let crate::hooks::HookOutcome::Block { reason } =
            self.hooks.pre_tool_use(tool_name, args).await
        {
            let _ = self.app.lock().unwrap().push(RenderLine::ToolResult { is_error: true, content: format!("Hook blocked: {reason}") });
            return Ok(crate::tools::ToolResult {
                tool_call_id: call_id.to_string(),
                tool_name: tool_name.to_string(),
                output: format!("Blocked by hook: {reason}"),
                is_error: true,
            });
        }

        let mut result = dispatch(call_id.to_string(), tool_name, args, &self.mcp).await;

        // PostToolUse / PostToolUseFailure hooks
        if result.is_error {
            self.hooks.post_tool_use_failure(tool_name, args, &result.output).await;
        } else {
            // PostToolUse may inject additionalContext into the tool output
            if let Some(extra) = self.hooks.post_tool_use(tool_name, args, &result.output).await {
                result.output = format!("{}\n\n[Hook context: {extra}]", result.output);
            }
        }

        // Show result summary.
        let (is_err, content) = if result.is_error {
            (true, truncate(&result.output, 200).to_string())
        } else {
            match tool_name {
                "bash" | "run_command" | "execute_command" => {
                    (false, result.output.clone())
                }
                "write_file" | "create_file" => {
                    (false, format!("written ({} chars)", result.output.len()))
                }
                "delete_file" | "move_file" | "rename_file" => (false, "done".to_string()),
                _ => (false, format!("{} lines", result.output.lines().count())),
            }
        };
        let _ = self.app.lock().unwrap().push(RenderLine::ToolResult { is_error: is_err, content });

        Ok(result)
    }

    /// Prompt the user to approve/deny a tool call.
    /// Returns true = approved, false = denied.
    ///
    /// Shows a ratatui inline menu with three options:
    ///   1. Yes — run once
    ///   2. Yes, don't ask again — session-allow + run
    ///   3. No — deny
    /// Generate a diff preview for file-mutation tools shown before the approval prompt.
    fn build_diff_preview(tool_name: &str, args: &serde_json::Value) -> Option<Vec<RenderLine>> {
        match tool_name {
            "edit_file" => {
                let path       = args["path"].as_str()?;
                let old_string = args["old_string"].as_str()?;
                let new_string = args["new_string"].as_str()?;
                let existing   = std::fs::read_to_string(path).ok()?;
                let offset = existing.find(old_string)
                    .map(|byte| existing[..byte].lines().count())
                    .unwrap_or(0);
                let mut out: Vec<RenderLine> = vec![
                    RenderLine::DimMsg(format!("--- {path}")),
                ];
                for (i, ln) in old_string.lines().enumerate() {
                    out.push(RenderLine::ErrorMsg(
                        format!("- {ln}  (L{})", offset + i + 1)
                    ));
                }
                for ln in new_string.lines() {
                    out.push(RenderLine::SuccessMsg(format!("+ {ln}")));
                }
                Some(out)
            }
            "write_file" | "create_file" => {
                let path    = args["path"].as_str()?;
                let content = args["content"].as_str()?;
                let is_new  = !std::path::Path::new(path).exists();
                let lines: Vec<&str> = content.lines().collect();
                let show    = lines.len().min(12);
                let mut out: Vec<RenderLine> = vec![
                    RenderLine::DimMsg(format!("{} {path}",
                        if is_new { "new file:" } else { "overwrite:" })),
                ];
                for ln in &lines[..show] {
                    out.push(RenderLine::SuccessMsg(format!("+ {ln}")));
                }
                if lines.len() > show {
                    out.push(RenderLine::DimMsg(
                        format!("  … ({} more lines)", lines.len() - show)
                    ));
                }
                Some(out)
            }
            "apply_patch" => {
                let patch = args["patch"].as_str()?;
                let mut out: Vec<RenderLine> = vec![RenderLine::DimMsg("(patch)".to_string())];
                for ln in patch.lines().take(20) {
                    if ln.starts_with('-') && !ln.starts_with("---") {
                        out.push(RenderLine::ErrorMsg(ln.to_string()));
                    } else if ln.starts_with('+') && !ln.starts_with("+++") {
                        out.push(RenderLine::SuccessMsg(ln.to_string()));
                    } else {
                        out.push(RenderLine::DimMsg(ln.to_string()));
                    }
                }
                if patch.lines().count() > 20 {
                    out.push(RenderLine::DimMsg(
                        format!("… ({} more lines)", patch.lines().count() - 20)
                    ));
                }
                Some(out)
            }
            _ => None,
        }
    }

    fn prompt_approval(
        &self,
        _stdout: &mut io::Stdout,
        tool_name: &str,
        args: &serde_json::Value,
    ) -> Result<bool> {
        use crate::ui::question::{Question, QuestionOption, QuestionWidget};

        // Show diff preview for file-mutation tools before the approval prompt.
        if let Some(diff_lines) = Self::build_diff_preview(tool_name, args) {
            let mut app = self.app.lock().unwrap();
            for line in diff_lines {
                let _ = app.push(line);
            }
            let _ = app.draw();
        }

        // One-line preview of what is being requested
        let preview: String = if let Some(cmd) = args["command"].as_str() {
            truncate(cmd, 100).to_string()
        } else if let Some(fp) = args["file_path"].as_str().or(args["path"].as_str()) {
            fp.to_string()
        } else if let Some(pat) = args["pattern"].as_str() {
            format!("\"{}\"", truncate(pat, 60))
        } else {
            String::new()
        };

        // Header chip — tool name, max 12 chars
        let header_raw = tool_name.replace('_', " ");
        let header: String = header_raw.chars().take(12).collect();

        let question_text = if preview.is_empty() {
            format!("Run {tool_name}?")
        } else {
            format!("{preview}")
        };

        let opts = vec![
            QuestionOption {
                label: "Yes".to_string(),
                description: "Run this tool once".to_string(),
            },
            QuestionOption {
                label: "Yes, don't ask again".to_string(),
                description: "Allow this tool for the rest of the session".to_string(),
            },
            QuestionOption {
                label: "No".to_string(),
                description: "Deny this tool call".to_string(),
            },
        ];

        let q = Question {
            header: &header,
            text: &question_text,
            options: &opts,
            multi_select: false,
            allow_other: false,
            progress: None,
        };

        // Render the question through TuiApp's terminal (already in alternate screen + raw mode).
        let qa = {
            let mut app = self.app.lock().unwrap();
            let result = QuestionWidget::ask(&mut app.terminal, &q)?;
            // Redraw normal CADE UI after the question widget.
            let _ = app.draw();
            result
        };

        match qa {
            None => Ok(false), // Esc / Ctrl+C = deny
            Some(answer) => {
                let label = answer.as_str();
                if label.starts_with("Yes, don't") {
                    self.permissions.add_session_allow(tool_name);
                    Ok(true)
                } else if label.starts_with("Yes") {
                    Ok(true)
                } else {
                    Ok(false)
                }
            }
        }
    }

    /// Handle the agent's `update_memory` tool call natively.
    async fn handle_update_memory(
        &self,
        call_id: &str,
        args: &serde_json::Value,
    ) -> Result<crate::tools::ToolResult> {
        let label = args["label"].as_str().unwrap_or("").trim().to_string();
        let value = args["value"].as_str().unwrap_or("").to_string();
        let operation = args["operation"].as_str().unwrap_or("set");

        if label.is_empty() {
            return Ok(crate::tools::ToolResult {
                tool_call_id: call_id.to_string(),
                tool_name: "update_memory".to_string(),
                output: "error: 'label' is required".to_string(),
                is_error: true,
            });
        }

        let agent_id = self.agent_id();
        let final_value = if operation == "append" {
            let existing = self.client.get_memory(&agent_id).await
                .unwrap_or_default()
                .into_iter()
                .find(|b| b.label == label)
                .map(|b| b.value)
                .unwrap_or_default();
            // S3: deduplication — skip append if content is already present
            let normalised_new = value.split_whitespace().collect::<String>();
            let normalised_existing = existing.split_whitespace().collect::<String>();
            if !normalised_new.is_empty() && normalised_existing.contains(&normalised_new) {
                return Ok(crate::tools::ToolResult {
                    tool_call_id: call_id.to_string(),
                    tool_name: "update_memory".to_string(),
                    output: format!("Memory block '{label}' already contains this information — no change."),
                    is_error: false,
                });
            }
            if existing.is_empty() { value } else { format!("{existing}\n{value}") }
        } else {
            value
        };

        let description = args["description"].as_str();
        match self.client.upsert_memory(&agent_id, &label, &final_value, description).await {
            Ok(_) => {
                tracing::info!("Agent updated memory [{label}]");
                Ok(crate::tools::ToolResult {
                    tool_call_id: call_id.to_string(),
                    tool_name: "update_memory".to_string(),
                    output: format!("Memory block '{label}' updated"),
                    is_error: false,
                })
            }
            Err(e) => Ok(crate::tools::ToolResult {
                tool_call_id: call_id.to_string(),
                tool_name: "update_memory".to_string(),
                output: format!("Failed to update '{label}': {e}"),
                is_error: true,
            }),
        }
    }

    /// Interactive `ask_user_question` tool intercept.
    ///
    /// Parses the LLM's structured questions, shows the `QuestionWidget` for
    /// each one sequentially, then returns a formatted result string to the agent.
    async fn handle_ask_user_question(
        &self,
        call_id: &str,
        args: &serde_json::Value,
    ) -> Result<crate::tools::ToolResult> {
        use crate::tools::AskUserQuestionTool;
        use crate::ui::question::{Question, QuestionOption, QuestionWidget};
        use std::collections::HashMap;

        // Parse and validate
        let ask_questions = match AskUserQuestionTool::parse_questions(args) {
            Ok(q) => q,
            Err(e) => {
                let msg = format!("Invalid ask_user_question args: {e}");
                let _ = self.app.lock().unwrap().push(RenderLine::ToolResult { is_error: true, content: msg.clone() });
                return Ok(crate::tools::ToolResult {
                    tool_call_id: call_id.to_string(),
                    tool_name: "ask_user_question".to_string(),
                    output: msg,
                    is_error: true,
                });
            }
        };

        let total = ask_questions.len();
        let _ = self.app.lock().unwrap().commit_streaming();

        let mut answers: HashMap<String, String> = HashMap::new();

        for (i, aq) in ask_questions.iter().enumerate() {
            let opts: Vec<QuestionOption> = aq.options.iter()
                .map(|o| QuestionOption {
                    label: o.label.clone(),
                    description: o.description.clone(),
                })
                .collect();

            let q = Question {
                header: &aq.header,
                text: &aq.question,
                options: &opts,
                multi_select: aq.multi_select,
                allow_other: true,
                progress: if total > 1 { Some((i + 1, total)) } else { None },
            };

            let qa = {
                let mut app = self.app.lock().unwrap();
                let res = QuestionWidget::ask(&mut app.terminal, &q)?;
                let _ = app.draw();
                res
            };

            match qa {
                None => {
                    let msg = "User cancelled the question prompt.".to_string();
                    let _ = self.app.lock().unwrap().push(RenderLine::ToolResult { is_error: true, content: msg.clone() });
                    return Ok(crate::tools::ToolResult {
                        tool_call_id: call_id.to_string(),
                        tool_name: "ask_user_question".to_string(),
                        output: msg,
                        is_error: true,
                    });
                }
                Some(answer) => {
                    let _ = self.app.lock().unwrap().push(RenderLine::SystemMsg(
                        format!("  {}: {}", aq.header, answer.as_str())
                    ));
                    answers.insert(aq.question.clone(), answer.as_str());
                }
            }
        }

        let result = AskUserQuestionTool::format_result(&answers);
        let _ = self.app.lock().unwrap().push(RenderLine::ToolResult {
            is_error: false,
            content: format!("{} answer{} collected", total, if total == 1 { "" } else { "s" }),
        });

        Ok(crate::tools::ToolResult {
            tool_call_id: call_id.to_string(),
            tool_name: "ask_user_question".to_string(),
            output: result,
            is_error: false,
        })
    }

    /// Return the full body of a skill by ID — `load_skill` tool intercept.
    async fn handle_load_skill(
        &self,
        call_id: &str,
        args: &serde_json::Value,
    ) -> Result<crate::tools::ToolResult> {
        let id = args["id"].as_str().unwrap_or("").trim().to_string();
        let skills = self.skills.lock().unwrap();
        match skills.iter().find(|s| s.id == id) {
            Some(skill) => {
                let content = skill.to_context_block();
                tracing::info!("Agent loaded skill: {id}");
                Ok(crate::tools::ToolResult {
                    tool_call_id: call_id.to_string(),
                    tool_name: "load_skill".to_string(),
                    output: content,
                    is_error: false,
                })
            }
            None => {
                let available: Vec<&str> = skills.iter().map(|s| s.id.as_str()).collect();
                Ok(crate::tools::ToolResult {
                    tool_call_id: call_id.to_string(),
                    tool_name: "load_skill".to_string(),
                    output: format!(
                        "Skill '{}' not found. Available: {}",
                        id,
                        available.join(", ")
                    ),
                    is_error: true,
                })
            }
        }
    }

    /// Download and install a skill from a URL — `install_skill` tool intercept.
    async fn handle_install_skill(
        &self,
        call_id: &str,
        args: &serde_json::Value,
        stdout: &mut io::Stdout,
    ) -> Result<crate::tools::ToolResult> {
        let url   = args["url"].as_str().unwrap_or("").trim().to_string();
        let scope = args["scope"].as_str().unwrap_or("project");

        if url.is_empty() {
            return Ok(crate::tools::ToolResult {
                tool_call_id: call_id.to_string(),
                tool_name: "install_skill".to_string(),
                output: "error: 'url' is required".to_string(),
                is_error: true,
            });
        }

        // Resolve target directory based on scope
        let target_dir = if scope == "global" {
            dirs::home_dir()
                .map(|h| h.join(".cade").join("skills"))
                .unwrap_or_else(|| self.skills_dir.clone())
        } else {
            self.skills_dir.clone()
        };

        self.tui_dim(format!("  Downloading skill from {}…", url));

        match crate::skills::install_skill_from_url(&url, &target_dir).await {
            Ok(skill) => {
                let name = skill.name.clone();
                let id   = skill.id.clone();
                self.skills.lock().unwrap().push(skill);
                let agent_id = self.agent_id();
                let skills   = self.skills.lock().unwrap().clone();
                let listing  = crate::skills::skills_listing(&skills);
                let _ = self.client.upsert_memory(
                    &agent_id, "skills",
                    listing.as_deref().unwrap_or(""), None
                ).await;
                drop(skills);
                self.tui_ok(format!("  ✓ Installed: {name} [{id}]"));
                Ok(crate::tools::ToolResult {
                    tool_call_id: call_id.to_string(),
                    tool_name: "install_skill".to_string(),
                    output: format!("Skill '{name}' installed as [{id}] in {scope} scope. It is now available via load_skill(\"{id}\")."),
                    is_error: false,
                })
            }
            Err(e) => {
                self.tui_err(format!("  ✗ Install failed: {e}"));
                Ok(crate::tools::ToolResult {
                    tool_call_id: call_id.to_string(),
                    tool_name: "install_skill".to_string(),
                    output: format!("Failed to install skill: {e}"),
                    is_error: true,
                })
            }
        }
    }

    /// Interactive /connect flow — guided provider setup.
    async fn handle_connect(
        &self,
        preset: Option<String>,
        _stdout: &mut io::Stdout,
    ) -> Result<()> {
        use crate::ui::question::{Question, QuestionOption, QuestionWidget};

        const BUILTIN: &[(&str, &str)] = &[
            ("anthropic", "Anthropic (Claude models)"),
            ("openai",    "OpenAI (GPT / Codex models)"),
            ("gemini",    "Google Gemini"),
            ("ollama",    "Ollama (local models, no key needed)"),
        ];

        let presets = self.client.list_provider_presets().await;

        let (name, kind, default_base_url) = if let Some(p) = preset {
            if let Some(&(n, _)) = BUILTIN.iter().find(|(n, _)| *n == p.as_str()) {
                (n.to_string(), n.to_string(), None)
            } else if let Some(preset_val) = presets.iter().find(|v| v["name"].as_str() == Some(&p)) {
                let base = preset_val["base_url"].as_str().map(String::from);
                (p.clone(), "openai-compatible".to_string(), base)
            } else {
                (p.clone(), "openai-compatible".to_string(), None)
            }
        } else {
            // Interactive picker via QuestionWidget
            let mut all_options: Vec<(String, String, Option<String>)> = BUILTIN.iter()
                .map(|(n, label)| (n.to_string(), label.to_string(), None))
                .collect();
            for p in &presets {
                let n = p["name"].as_str().unwrap_or("?").to_string();
                let u = p["base_url"].as_str().map(String::from);
                all_options.push((n.clone(), format!("{n} (OpenAI-compatible)"), u));
            }
            all_options.push(("custom".to_string(), "Custom OpenAI-compatible URL…".to_string(), None));

            let opts: Vec<QuestionOption> = all_options.iter()
                .map(|(_, label, _)| QuestionOption { label: label.clone(), description: String::new() })
                .collect();
            let q = Question {
                header: "Connect provider", text: "Choose a provider to connect:",
                options: &opts, multi_select: false, allow_other: false, progress: None,
            };
            let ans = {
                let mut app = self.app.lock().unwrap();
                let r = QuestionWidget::ask(&mut app.terminal, &q)?;
                let _ = app.draw();
                r
            };
            let Some(chosen) = ans else { return Ok(()); };
            let label = chosen.as_str();
            let idx = all_options.iter().position(|(_, l, _)| l.as_str() == label).unwrap_or(0);
            let (n, _, base) = all_options.remove(idx);
            let k = if BUILTIN.iter().any(|(bn, _)| *bn == n.as_str()) { n.clone() }
                    else { "openai-compatible".to_string() };
            (n, k, base)
        };

        // Ask for API key
        let needs_key = kind != "ollama";
        let api_key = if needs_key {
            let key_opts = vec![QuestionOption { label: "Skip (no key)".to_string(), description: String::new() }];
            let kq = Question {
                header: "API Key",
                text: &format!("API key for '{name}' (type key or select Skip):"),
                options: &key_opts, multi_select: false, allow_other: true, progress: None,
            };
            let ans = {
                let mut app = self.app.lock().unwrap();
                let r = QuestionWidget::ask(&mut app.terminal, &kq)?;
                let _ = app.draw();
                r
            };
            match ans {
                Some(ref a) if a.as_str() != "Skip (no key)" && !a.as_str().is_empty() => Some(a.as_str().to_string()),
                _ => None,
            }
        } else { None };

        // Ask for base URL if needed
        let base_url = if kind == "openai-compatible" && default_base_url.is_none() {
            let url_opts = vec![QuestionOption { label: "Cancel".to_string(), description: String::new() }];
            let uq = Question {
                header: "Base URL",
                text: "Base URL (e.g. https://api.example.com/v1):",
                options: &url_opts, multi_select: false, allow_other: true, progress: None,
            };
            let ans = {
                let mut app = self.app.lock().unwrap();
                let r = QuestionWidget::ask(&mut app.terminal, &uq)?;
                let _ = app.draw();
                r
            };
            match ans {
                Some(ref a) if a.as_str() != "Cancel" && !a.as_str().is_empty() => Some(a.as_str().to_string()),
                _ => None,
            }
        } else { default_base_url };

        self.tui_dim(format!("  Connecting to '{name}'…"));
        match self.client.add_provider(&name, &kind, api_key.as_deref(), base_url.as_deref()).await {
            Ok(_) => {
                self.tui_ok(format!("  ✓ Provider '{name}' connected and hot-loaded"));
                self.tui_dim(format!("    Use: /model {name}/<model-name>"));
            }
            Err(e) => self.tui_err(e.to_string()),
        }
        Ok(())
    }

    /// `/resume` conversation picker — full-screen on TuiApp terminal.
    ///
    /// Keys: ↑/↓ move · Enter select · d delete · Esc/q cancel.
    /// Returns the picked conversation JSON, or None if cancelled.
    async fn conversation_picker(
        &self,
        app_arc: std::sync::Arc<std::sync::Mutex<crate::ui::TuiApp>>,
        convs: &[serde_json::Value],
        agent_id: &str,
    ) -> Result<Option<serde_json::Value>> {
        use crossterm::event::{self, Event, KeyCode, KeyModifiers};
        use ratatui::{
            style::{Color as RC, Modifier, Style},
            text::{Line, Span},
            widgets::{Block, Borders, List, ListItem, ListState},
        };

        if convs.is_empty() {
            return Ok(None);
        }

        let mut sel:    usize = 0;
        let mut result: Option<serde_json::Value> = None;

        let build_items = |sel: usize| -> Vec<ListItem<'static>> {
            convs.iter().enumerate().map(|(i, c)| {
                let title = c["title"].as_str().unwrap_or("(untitled)").to_string();
                let cnt   = c["message_count"].as_i64().unwrap_or(0);
                let ts    = c["updated_at"].as_i64().unwrap_or(0);
                let date  = if ts > 0 {
                    let dt = chrono::DateTime::from_timestamp(ts, 0)
                        .unwrap_or_default()
                        .with_timezone(&chrono::Local);
                    dt.format("%m/%d %H:%M").to_string()
                } else { String::new() };
                let label = format!("  {title}  ({cnt} msgs)  {date}");
                let style = if i == sel {
                    Style::default().fg(RC::Black).bg(RC::Cyan).add_modifier(Modifier::BOLD)
                } else {
                    Style::default().fg(RC::White)
                };
                ListItem::new(Line::from(vec![Span::styled(label, style)]))
            }).collect()
        };

        // Initial draw
        {
            let mut app = app_arc.lock().unwrap();
            let items = build_items(sel);
            let n     = convs.len();
            let mut ls = ListState::default().with_selected(Some(sel));
            app.terminal.draw(|f| {
                let area  = f.area();
                let block = Block::default()
                    .borders(Borders::ALL)
                    .title(format!(" Conversations [{}/{}]  ↑↓ navigate · Enter select · d delete · Esc cancel ", sel + 1, n))
                    .border_style(Style::default().fg(RC::Cyan));
                let list = List::new(items).block(block);
                f.render_stateful_widget(list, area, &mut ls);
            })?;
        }

        loop {
            if !event::poll(std::time::Duration::from_millis(200))? { continue; }
            match event::read()? {
                Event::Key(k) => match (k.code, k.modifiers) {
                    (KeyCode::Char('q') | KeyCode::Esc, _) => break,
                    (KeyCode::Up   | KeyCode::Char('k'), _) => { if sel > 0 { sel -= 1; } }
                    (KeyCode::Down | KeyCode::Char('j'), _) => {
                        if sel + 1 < convs.len() { sel += 1; }
                    }
                    (KeyCode::Enter, _) => {
                        result = convs.get(sel).cloned();
                        break;
                    }
                    (KeyCode::Char('d') | KeyCode::Delete, _) => {
                        let conv_id = convs[sel]["id"].as_str().unwrap_or("").to_string();
                        let title   = convs[sel]["title"].as_str().unwrap_or("(untitled)").to_string();
                        // Use QuestionWidget for confirmation
                        use crate::ui::question::{Question, QuestionOption, QuestionWidget};
                        let opts = vec![
                            QuestionOption { label: "Yes — delete".to_string(), description: String::new() },
                            QuestionOption { label: "No — keep".to_string(),    description: String::new() },
                        ];
                        let q = Question {
                            header: "Delete?", text: &format!("Delete conversation \"{title}\"?"),
                            options: &opts, multi_select: false, allow_other: false, progress: None,
                        };
                        let ans = {
                            let mut app = app_arc.lock().unwrap();
                            let r = QuestionWidget::ask(&mut app.terminal, &q)?;
                            r
                        };
                        if matches!(ans, Some(ref a) if a.as_str().starts_with("Yes")) {
                            let _ = self.client.delete_conversation(agent_id, &conv_id).await;
                        }
                        return Ok(None);
                    }
                    (KeyCode::Char('c'), KeyModifiers::CONTROL) => break,
                    _ => {}
                },
                _ => {}
            }
            // Redraw after state change
            let mut app = app_arc.lock().unwrap();
            let items = build_items(sel);
            let n     = convs.len();
            let mut ls = ListState::default().with_selected(Some(sel));
            app.terminal.draw(|f| {
                let area  = f.area();
                let block = Block::default()
                    .borders(Borders::ALL)
                    .title(format!(" Conversations [{}/{}]  ↑↓ navigate · Enter select · d delete · Esc cancel ", sel + 1, n))
                    .border_style(Style::default().fg(RC::Cyan));
                let list = List::new(items).block(block);
                f.render_stateful_widget(list, area, &mut ls);
            })?;
        }

        Ok(result)
    }

    /// `/agents` TUI picker — full-screen on TuiApp terminal.
    ///
    /// Keys:
    ///   ↑/↓  j/k  — move cursor
    ///   Space      — toggle mark for deletion
    ///   d / Delete — confirm delete of all marked (or current if none marked)
    ///   r          — rename highlighted agent
    ///   Enter      — switch to highlighted agent (only when no marks)
    ///   Esc / q    — cancel
    async fn agent_picker(
        &self,
        app_arc: std::sync::Arc<std::sync::Mutex<crate::ui::TuiApp>>,
        agents: &mut Vec<AgentState>,
    ) -> Result<Option<AgentPickerResult>> {
        use crossterm::event::{self, Event, KeyCode};
        use std::collections::HashSet;
        use ratatui::{
            widgets::{List, ListItem, ListState, Block, Borders},
            style::{Style, Color as RC, Modifier},
            text::{Line, Span},
        };

        if agents.is_empty() {
            return Ok(None);
        }

        let current = self.agent_id();
        let total   = agents.len();
        let mut selected: usize = agents.iter()
            .position(|a| a.id == current)
            .unwrap_or(0);
        let mut marked: HashSet<usize> = HashSet::new();

        let build_items = |agents: &[AgentState], sel: usize,
                           marked: &HashSet<usize>, current: &str| -> Vec<ListItem<'static>> {
            agents.iter().enumerate().map(|(i, a)| {
                let is_sel    = i == sel;
                let is_marked = marked.contains(&i);
                let is_active = a.id == current;
                let short_id  = if a.id.len() > 22 { a.id[..22].to_string() + "…" }
                                else { a.id.clone() };
                ListItem::new(Line::from(vec![
                    Span::styled(if is_sel { " ▶ " } else { "   " }.to_string(),
                        Style::default().fg(if is_sel { RC::Green } else { RC::DarkGray })),
                    Span::styled(if is_marked { "☑ " } else { "☐ " }.to_string(),
                        Style::default().fg(if is_marked { RC::Yellow } else { RC::DarkGray })),
                    Span::styled(format!("{:<32}", a.name),
                        Style::default()
                            .fg(if is_sel { RC::White } else { RC::DarkGray })
                            .add_modifier(if is_sel { Modifier::BOLD } else { Modifier::empty() })),
                    Span::styled(short_id, Style::default().fg(RC::DarkGray)),
                    Span::styled(
                        if is_active { "  ← active".to_string() } else { String::new() },
                        Style::default().fg(RC::Cyan),
                    ),
                ]))
            }).collect()
        };

        let do_draw = |app_arc: &std::sync::Arc<std::sync::Mutex<crate::ui::TuiApp>>,
                       agents: &[AgentState], sel: usize,
                       marked: &HashSet<usize>, current: &str| -> Result<()> {
            let mut app = app_arc.lock().unwrap();
            let items  = build_items(agents, sel, marked, current);
            let n      = marked.len();
            let hint   = if n == 0 {
                " ↑↓/jk  Space mark  r rename  d delete  Enter switch  q cancel ".to_string()
            } else {
                format!(" [{n} marked]  d delete all  q cancel ")
            };
            let mut ls = ListState::default().with_selected(Some(sel));
            app.terminal.draw(|f| {
                let area  = f.area();
                let block = Block::default()
                    .borders(Borders::ALL)
                    .title(format!(" Agents {hint}"))
                    .border_style(Style::default().fg(RC::Cyan));
                let list  = List::new(items).block(block);
                f.render_stateful_widget(list, area, &mut ls);
            })?;
            Ok(())
        };

        do_draw(&app_arc, agents, selected, &marked, &current)?;

        let result = loop {
            if !event::poll(std::time::Duration::from_millis(200))? { continue; }
            if let Ok(Event::Key(key)) = event::read() {
                match (key.code, key.modifiers) {
                    (KeyCode::Esc, _) | (KeyCode::Char('q'), _) => break None,

                    (KeyCode::Enter, _) => {
                        if marked.is_empty() {
                            let a = agents[selected].clone();
                            if a.id != current { break Some(AgentPickerResult::Switch(a)); }
                        }
                    }

                    (KeyCode::Char(' '), _) => {
                        if marked.contains(&selected) { marked.remove(&selected); }
                        else                          { marked.insert(selected);  }
                    }

                    (KeyCode::Char('d'), _) | (KeyCode::Delete, _) => {
                        let targets: Vec<usize> = if marked.is_empty() { vec![selected] }
                            else { let mut v: Vec<usize> = marked.iter().copied().collect(); v.sort_unstable(); v };
                        let names: Vec<String> = targets.iter().map(|&i| agents[i].name.clone()).collect();
                        let label = if targets.len() == 1 { format!("Delete '{}'?", names[0]) }
                            else { format!("Delete {} agents ({})?", targets.len(), names.join(", ")) };
                        use crate::ui::question::{Question, QuestionOption, QuestionWidget};
                        let opts = vec![
                            QuestionOption { label: "Yes — delete".to_string(), description: String::new() },
                            QuestionOption { label: "No — cancel".to_string(),  description: String::new() },
                        ];
                        let q = Question { header: "Confirm", text: &label, options: &opts,
                            multi_select: false, allow_other: false, progress: None };
                        let confirmed = {
                            let mut app = app_arc.lock().unwrap();
                            let r = QuestionWidget::ask(&mut app.terminal, &q)?;
                            matches!(r, Some(ref a) if a.as_str().starts_with("Yes"))
                        };
                        if confirmed {
                            let to_delete: Vec<AgentState> = targets.iter().map(|&i| agents[i].clone()).collect();
                            break Some(AgentPickerResult::DeleteMany(to_delete));
                        }
                    }

                    (KeyCode::Char('r'), _) => {
                        let a = agents[selected].clone();
                        // Collect new name via QuestionWidget (allow_other = freetext)
                        use crate::ui::question::{Question, QuestionOption, QuestionWidget};
                        let opts = vec![
                            QuestionOption { label: "Keep current name".to_string(), description: String::new() },
                        ];
                        let q = Question {
                            header: "Rename agent",
                            text: &format!("New name for '{}':", a.name),
                            options: &opts, multi_select: false, allow_other: true, progress: None,
                        };
                        let ans = {
                            let mut app = app_arc.lock().unwrap();
                            QuestionWidget::ask(&mut app.terminal, &q)?
                        };
                        if let Some(ref answer) = ans {
                            let new_name = answer.as_str();
                            if !new_name.is_empty() && new_name != "Keep current name" {
                                break Some(AgentPickerResult::Rename { agent: a, new_name });
                            }
                        }
                    }

                    (KeyCode::Up, _) | (KeyCode::Char('k'), _) => {
                        selected = if selected == 0 { total - 1 } else { selected - 1 };
                    }
                    (KeyCode::Down, _) | (KeyCode::Char('j'), _) => {
                        selected = (selected + 1) % total;
                    }
                    _ => {}
                }
                do_draw(&app_arc, agents, selected, &marked, &current)?;
            }
        };

        Ok(result)
    }

    /// Interactive model picker — full-screen on TuiApp terminal.
    /// Returns the selected model string or None if cancelled.
    async fn interactive_model_picker(
        &self,
        app_arc: std::sync::Arc<std::sync::Mutex<crate::ui::TuiApp>>,
    ) -> Result<Option<String>> {
        use crossterm::event::{self, Event, KeyCode};
        use ratatui::{
            widgets::{List, ListItem, ListState, Block, Borders,
                      Scrollbar, ScrollbarOrientation, ScrollbarState},
            layout::{Constraint, Layout, Direction},
            style::{Style, Color as RC, Modifier},
            text::{Line, Span},
        };

        {
            let mut app = app_arc.lock().unwrap();
            let _ = app.push(crate::ui::RenderLine::DimMsg("  Fetching models…".to_string()));
        }

        let current = self.model();

        // ── Fetch model list ──────────────────────────────────────────────────
        // (provider, display_name, model_id, toolset, is_dynamic)
        let mut models: Vec<(String, String, String, String, bool)> = Vec::new();
        let mut custom_providers: Vec<String> = Vec::new();

        match self.client.list_models().await {
            Ok(body) => {
                if let Some(arr) = body["supported"].as_array() {
                    for m in arr {
                        models.push((
                            m["provider"].as_str().unwrap_or("?").to_string(),
                            m["display_name"].as_str().unwrap_or("?").to_string(),
                            m["id"].as_str().unwrap_or("?").to_string(),
                            m["toolset"].as_str().unwrap_or("default").to_string(),
                            false,
                        ));
                    }
                }
                if let Some(arr) = body["dynamic"].as_array() {
                    for m in arr {
                        let id       = m["id"].as_str().unwrap_or("?").to_string();
                        let provider = m["provider"].as_str().unwrap_or("?").to_string();
                        if !models.iter().any(|(_, _, mid, _, _)| mid == &id) {
                            models.push((
                                provider,
                                m["display_name"].as_str().unwrap_or(&id).to_string(),
                                id,
                                m["toolset"].as_str().unwrap_or("default").to_string(),
                                true,
                            ));
                        }
                    }
                }
                if let Some(arr) = body["custom_providers"].as_array() {
                    custom_providers = arr.iter()
                        .filter_map(|v| v.as_str().map(String::from))
                        .collect();
                }
            }
            Err(_) => {
                let mut app = app_arc.lock().unwrap();
                let _ = app.push(crate::ui::RenderLine::ErrorMsg("Could not fetch models. Specify directly: /model provider/model-name".to_string()));
                return Ok(None);
            }
        }

        for cp in &custom_providers {
            models.push((cp.clone(), format!("Enter model for {cp}…"),
                         format!("{cp}/"), "default".to_string(), false));
        }
        // Sentinel: always-last "Enter custom model ID" entry
        models.push(("__custom__".to_string(), "Enter custom model ID…".to_string(),
                     String::new(), String::new(), false));

        if models.len() == 1 {
            let mut app = app_arc.lock().unwrap();
            let _ = app.push(crate::ui::RenderLine::DimMsg("  No models available. Connect a provider: /connect".to_string()));
            return Ok(None);
        }

        let n_models = models.len();

        // ── Flat display-item list (provider headers + model rows) ────────────
        #[derive(Clone)]
        enum DisplayItem { Header(String, bool), ModelRow(usize) }

        let display_items: Vec<DisplayItem> = {
            let mut items = Vec::new();
            let mut last_p = String::new();
            for (i, (provider, _, _, _, dynamic)) in models.iter().enumerate() {
                if *provider != last_p {
                    items.push(DisplayItem::Header(provider.clone(), *dynamic));
                    last_p = provider.clone();
                }
                items.push(DisplayItem::ModelRow(i));
            }
            items
        };
        let disp_len = display_items.len();

        // list_pos = position in display_items (never on a Header)
        let initial_list_pos = display_items.iter()
            .position(|d| matches!(d, DisplayItem::ModelRow(i) if models[*i].2 == current))
            .or_else(|| display_items.iter().position(|d| matches!(d, DisplayItem::ModelRow(_))))
            .unwrap_or(0);
        let mut list_pos = initial_list_pos;

        // Navigate display_items, skipping Header items
        let next_pos = |mut p: usize| -> usize {
            loop {
                p = (p + 1) % disp_len;
                if !matches!(display_items.get(p), Some(DisplayItem::Header(..))) { return p; }
            }
        };
        let prev_pos = |mut p: usize| -> usize {
            loop {
                p = if p == 0 { disp_len - 1 } else { p - 1 };
                if !matches!(display_items.get(p), Some(DisplayItem::Header(..))) { return p; }
            }
        };
        // Derive selected model index from list_pos
        let model_at = |p: usize| -> usize {
            if let Some(DisplayItem::ModelRow(i)) = display_items.get(p) { *i } else { 0 }
        };

        // ── Build ratatui ListItems ───────────────────────────────────────────
        let build_items = |list_pos: usize, current: &str| -> Vec<ListItem<'static>> {
            display_items.iter().map(|item| match item {
                DisplayItem::Header(provider, dynamic) => {
                    if provider == "__custom__" {
                        ListItem::new(Line::from(Span::styled(
                            "  ─────────────────────────────────────────".to_string(),
                            Style::default().fg(RC::DarkGray),
                        )))
                    } else {
                        let suffix = if *dynamic {
                            if provider == "ollama" { " (local)" } else { " (live)" }
                        } else { "" };
                        ListItem::new(Line::from(Span::styled(
                            format!("  {}{}", provider.to_uppercase(), suffix),
                            Style::default().fg(RC::Yellow).add_modifier(Modifier::BOLD),
                        )))
                    }
                }
                DisplayItem::ModelRow(i) => {
                    let (provider, name, id, toolset, _) = &models[*i];
                    let is_sel     = *i == model_at(list_pos);
                    let is_current = !id.is_empty() && id == current;

                    if provider == "__custom__" {
                        ListItem::new(Line::from(vec![
                            Span::styled(
                                if is_sel { "  ▶ " } else { "    " }.to_string(),
                                Style::default().fg(RC::Cyan),
                            ),
                            Span::styled(name.clone(),
                                Style::default().fg(if is_sel { RC::Cyan } else { RC::DarkGray })),
                        ]))
                    } else {
                        let name_trunc = if name.len() > 44 {
                            format!("{}…", &name[..43])
                        } else {
                            format!("{:<44}", name)
                        };
                        let toolset_tag = if toolset.is_empty() { String::new() }
                                          else { format!(" [{toolset}]") };
                        let current_tag = if is_current { " ← current".to_string() }
                                          else { String::new() };
                        ListItem::new(Line::from(vec![
                            Span::styled(
                                if is_sel { "  ▶ " } else { "    " }.to_string(),
                                Style::default().fg(if is_sel { RC::Green } else { RC::DarkGray }),
                            ),
                            Span::styled(name_trunc,
                                Style::default()
                                    .fg(if is_sel { RC::White } else { RC::DarkGray })
                                    .add_modifier(if is_sel { Modifier::BOLD } else { Modifier::empty() })),
                            Span::styled(toolset_tag,
                                Style::default().fg(RC::DarkGray)),
                            Span::styled(current_tag,
                                Style::default().fg(RC::Cyan)),
                        ]))
                    }
                }
            }).collect()
        };

        // ── Draw helper ───────────────────────────────────────────────────────
        let do_draw_model = |app_arc: &std::sync::Arc<std::sync::Mutex<crate::ui::TuiApp>>,
                             list_pos: usize| -> Result<()> {
            let sel_model = model_at(list_pos);
            let title = format!(
                " Models  ↑↓/jk/PgUp/PgDn  Enter select  q cancel  [{}/{}] ",
                sel_model + 1, n_models
            );
            let items = build_items(list_pos, &current);
            let list = List::new(items)
                .block(Block::default()
                    .borders(Borders::ALL)
                    .title(title)
                    .border_style(Style::default().fg(RC::Cyan)));
            let mut ls = ListState::default().with_selected(Some(list_pos));
            let mut sb = ScrollbarState::new(disp_len).position(list_pos);
            let mut app = app_arc.lock().unwrap();
            app.terminal.draw(|f| {
                let area   = f.area();
                let chunks = Layout::default()
                    .direction(Direction::Horizontal)
                    .constraints([Constraint::Fill(1), Constraint::Length(1)])
                    .split(area);
                f.render_stateful_widget(list, chunks[0], &mut ls);
                f.render_stateful_widget(
                    Scrollbar::new(ScrollbarOrientation::VerticalRight),
                    chunks[1], &mut sb,
                );
            })?;
            Ok(())
        };
        do_draw_model(&app_arc, list_pos)?;

        // ── Event loop ────────────────────────────────────────────────────────
        let result = loop {
            if !event::poll(std::time::Duration::from_millis(200))? { continue; }
            if let Ok(Event::Key(key)) = event::read() {
                match (key.code, key.modifiers) {
                    (KeyCode::Esc, _) | (KeyCode::Char('q'), _) => break None,

                    (KeyCode::Enter, _) => {
                        let sel = model_at(list_pos);
                        let (provider, _, id, _, _) = &models[sel];
                        if provider == "__custom__" || id.ends_with('/') {
                            // Freetext input via QuestionWidget
                            let prefix = if id.ends_with('/') && id.len() > 1 { id.as_str() } else { "" };
                            use crate::ui::question::{Question, QuestionOption, QuestionWidget};
                            let opts = vec![QuestionOption { label: "Cancel".to_string(), description: String::new() }];
                            let prompt = if prefix.is_empty() { "Enter model ID (e.g. provider/model-name):".to_string() }
                                         else { format!("Enter model for {prefix}") };
                            let q = Question { header: "Custom model", text: &prompt, options: &opts,
                                multi_select: false, allow_other: true, progress: None };
                            let ans = {
                                let mut app = app_arc.lock().unwrap();
                                QuestionWidget::ask(&mut app.terminal, &q)?
                            };
                            if let Some(ref a) = ans {
                                let typed = a.as_str();
                                if !typed.is_empty() && typed != "Cancel" {
                                    let full = if prefix.is_empty() || typed.starts_with(prefix) { typed }
                                               else { format!("{prefix}{typed}") };
                                    break Some(full);
                                }
                            }
                            break None;
                        } else {
                            break Some(id.clone());
                        }
                    }

                    (KeyCode::Up, _) | (KeyCode::Char('k'), _) => {
                        list_pos = prev_pos(list_pos);
                        do_draw_model(&app_arc, list_pos)?;
                    }
                    (KeyCode::Down, _) | (KeyCode::Char('j'), _) => {
                        list_pos = next_pos(list_pos);
                        do_draw_model(&app_arc, list_pos)?;
                    }
                    (KeyCode::PageDown, _) => {
                        for _ in 0..10 { list_pos = next_pos(list_pos); }
                        do_draw_model(&app_arc, list_pos)?;
                    }
                    (KeyCode::PageUp, _) => {
                        for _ in 0..10 { list_pos = prev_pos(list_pos); }
                        do_draw_model(&app_arc, list_pos)?;
                    }
                    _ => {}
                }
            }
        };

        Ok(result)
    }

    /// Handle the `run_subagent` tool call — spawn a subagent and return its result.
    async fn handle_run_subagent(
        &self,
        call_id: &str,
        args: &serde_json::Value,
        stdout: &mut io::Stdout,
    ) -> Result<crate::tools::ToolResult> {
        let subagent_type = args["subagent_type"].as_str().unwrap_or("general-purpose").trim().to_string();
        let prompt        = args["prompt"].as_str().unwrap_or("").trim().to_string();
        let background    = args["background"].as_bool().unwrap_or(false);
        let agent_id_arg  = args["agent_id"].as_str().map(|s| s.trim().to_string());
        let model_override= args["model"].as_str().map(|s| s.trim().to_string());

        if prompt.is_empty() {
            return Ok(crate::tools::ToolResult {
                tool_call_id: call_id.to_string(),
                tool_name: "run_subagent".to_string(),
                output: "error: 'prompt' is required".to_string(),
                is_error: true,
            });
        }

        // Resolve subagent definition
        let all_defs = discover_all_subagents(&self.cwd);
        let def_opt  = find_subagent(&subagent_type, &all_defs).cloned();

        // Determine if using existing stateful agent or ephemeral
        let _use_existing_agent = agent_id_arg.is_some();

        // Show progress
        self.tui_dim(format!("  Launching subagent [{}]{}…", subagent_type,
            if background { " (background)" } else { "" }));

        // Clone what we need for the async task
        let client     = self.client.clone();
        let main_model = self.model();
        let permissions = crate::permissions::PermissionManager::default();
        let call_id_owned = call_id.to_string();
        let bg_results = Arc::clone(&self.background_results);
        let mcp_ref    = std::sync::Arc::clone(&self.mcp);

        let task_id = uuid::Uuid::new_v4().to_string()[..8].to_string();
        let task_id_c = task_id.clone();
        let prompt_preview: String = prompt.chars().take(60).collect();

        let run_task = {
            let subagent_type_c = subagent_type.clone();
            let task_id_c = task_id.clone();
            let _prompt_preview_c = prompt_preview.clone();
            async move {
                // Determine agent to use
                let (sub_agent_id, ephemeral) = if let Some(existing_id) = agent_id_arg {
                    (existing_id, false)
                } else {
                    // Create ephemeral agent
                    let _system_prompt = def_opt.as_ref()
                        .map(|d| d.system_prompt.clone())
                        .unwrap_or_else(|| "You are a helpful coding assistant. Complete the task and report back.".to_string());

                    let model = model_override.clone()
                        .or_else(|| def_opt.as_ref().and_then(|d| d.model.clone()))
                        .unwrap_or(main_model);

                    let req = crate::agent::client::CreateAgentRequest {
                        name: Some(format!("subagent-{}-{}", subagent_type_c, task_id_c)),
                        model,
                        description: Some(format!("Ephemeral subagent: {}", subagent_type_c)),
                        system_prompt: None,
                        memory_blocks: vec![],
                        tool_ids: vec![],
                    };
                    match client.create_agent(req).await {
                        Ok(a)  => (a.id, true),
                        Err(e) => return (format!("Failed to create subagent: {e}"), true),
                    }
                };

                // Run headless
                let result = crate::cli::headless::run_headless(
                    &client, &sub_agent_id, &prompt, &permissions, &mcp_ref
                ).await;

                // Delete ephemeral agent
                if ephemeral {
                    let _ = client.delete_agent(&sub_agent_id).await;
                }

                match result {
                    Ok((output, _)) => (output, false),
                    Err(e)          => (format!("Subagent error: {e}"), true),
                }
            }
        };

        if background {
            // Spawn and return immediately
            let bg = bg_results;
            let st = subagent_type.clone();
            tokio::spawn(async move {
                let (result, is_error) = run_task.await;
                bg.lock().unwrap().push(BackgroundResult {
                    task_id: task_id.clone(),
                    subagent: st,
                    prompt_preview,
                    result,
                    is_error,
                });
            });

            Ok(crate::tools::ToolResult {
                tool_call_id: call_id_owned,
                tool_name: "run_subagent".to_string(),
                output: format!(
                    "Background subagent [{subagent_type}] launched (task ID: {}). \
                     You will be notified when it completes.", task_id_c
                ),
                is_error: false,
            })
        } else {
            // Run synchronously — wait for result
            let (output, is_error) = run_task.await;

            // SubagentStop hook — can block (exit 2 continues the agent)
            let hook_outcome = self.hooks.subagent_stop(&subagent_type, &output, is_error).await;

            if !is_error {
                self.tui_ok(format!("  ✓ Subagent [{}] complete", subagent_type));
            }

            // If hook blocked, append its reason to the output so the agent sees it
            let final_output = match hook_outcome {
                crate::hooks::HookOutcome::Block { reason } =>
                    format!("{output}\n\n[SubagentStop hook: {reason}]"),
                crate::hooks::HookOutcome::Allow => output,
            };

            Ok(crate::tools::ToolResult {
                tool_call_id: call_id_owned,
                tool_name: "run_subagent".to_string(),
                output: final_output,
                is_error,
            })
        }
    }

    /// Push a success line (green) to the TUI.
    fn tui_ok(&self, msg: impl Into<String>) {
        let _ = self.app.lock().unwrap().push(crate::ui::RenderLine::SuccessMsg(msg.into()));
    }
    /// Push an error line (red) to the TUI.
    fn tui_err(&self, msg: impl Into<String>) {
        let _ = self.app.lock().unwrap().push(crate::ui::RenderLine::ErrorMsg(msg.into()));
    }
    /// Push a section header (cyan bold) to the TUI.
    fn tui_hdr(&self, msg: impl Into<String>) {
        let _ = self.app.lock().unwrap().push(crate::ui::RenderLine::InfoHeader(msg.into()));
    }
    /// Push a dim hint / secondary text to the TUI.
    fn tui_dim(&self, msg: impl Into<String>) {
        let _ = self.app.lock().unwrap().push(crate::ui::RenderLine::DimMsg(msg.into()));
    }
    /// Push a plain system message (gray) to the TUI.
    fn tui_sys(&self, msg: impl Into<String>) {
        let _ = self.app.lock().unwrap().push(crate::ui::RenderLine::SystemMsg(msg.into()));
    }
    /// Push a blank line to the TUI.
    fn tui_blank(&self) {
        let _ = self.app.lock().unwrap().push(crate::ui::RenderLine::Blank);
    }

    #[allow(dead_code)]
    fn print_error(&self, _stdout: &mut io::Stdout, msg: &str) -> Result<()> {
        self.tui_err(format!("Error: {msg}"));
        Ok(())
    }

    /// Return the full help text as a string (used by /help via print_block).
    fn help_text() -> String {
        let mut s = String::new();
        s.push_str("\nCommands:\n");
        s.push_str("\n  Session:\n");
        s.push_str("    /info           - agent, model, mode, cwd\n");
        s.push_str("    /agent          - show current agent ID\n");
        s.push_str("    /agents         - list all agents (Space mark, r rename, d delete, Enter switch)\n");
        s.push_str("    /new            - start a new conversation on the current agent\n");
        s.push_str("    /new-agent      - create a brand-new agent\n");
        s.push_str("    /resume         - browse past conversations and switch to one\n");
        s.push_str("    /rename [name]  - rename current agent\n");
        s.push_str("    /delete <name>  - delete an agent by name or ID\n");
        s.push_str("    /pin            - pin current agent for quick access\n");
        s.push_str("    /clear          - clear screen + context window\n");
        s.push_str("\n  Hooks:\n");
        s.push_str("    /mcp            - list active MCP servers and their tools\n");
        s.push_str("    /link           - (re-)attach CADE tools to current agent\n");
        s.push_str("    /unlink         - remove all CADE tools from current agent\n");
        s.push_str("    /hooks                    - show active hook configuration\n");
        s.push_str("    (configure in ~/.cade/settings.json or .cade/settings.json)\n");
        s.push_str("    events: PreToolUse PostToolUse Stop UserPromptSubmit SessionStart ...\n");
        s.push_str("\n  Permissions:\n");
        s.push_str("    /permissions              - show mode + active allow/deny rules\n");
        s.push_str("    /approve-always <pattern> - always allow a tool pattern\n");
        s.push_str("    /deny-always <pattern>    - always deny a tool pattern\n");
        s.push_str("    pattern syntax: Bash(cargo test)  Read(src/**)  Bash(rm -rf:*)\n");
        s.push_str("\n  Providers:\n");
        s.push_str("    /providers           - list configured providers\n");
        s.push_str("    /connect             - interactive provider setup\n");
        s.push_str("    /connect <name>      - connect: anthropic, openai, gemini, openrouter, groq...\n");
        s.push_str("    /disconnect <name>   - remove a provider (persisted + live)\n");
        s.push_str("\n  Subagents:\n");
        s.push_str("    /subagents      - list available subagents (built-in + custom)\n");
        s.push_str("    ask agent to    - run_subagent(type, task) -- spawns subagent\n");
        s.push_str("    custom def      - .cade/agents/<name>.md in project or ~/.cade/agents/\n");
        s.push_str("\n  Skills:\n");
        s.push_str("    /skills                - list loaded skills\n");
        s.push_str("    /skills create <name>  - scaffold a new SKILL.MD file\n");
        s.push_str("    /skills show <id>      - show full skill content\n");
        s.push_str("    /skills reload         - re-discover skills + update agent memory\n");
        s.push_str("\n  Memory:\n");
        s.push_str("    /memory                    - list memory blocks (label + description + preview)\n");
        s.push_str("    /memory view <label>       - show full block content\n");
        s.push_str("    /memory set <label> <val>  - set a block directly\n");
        s.push_str("    /memory delete <label>              - delete a block\n");
        s.push_str("    /memory edit <label>                - multi-line inline editor\n");
        s.push_str("    /memory history <label>             - show last 5 revisions\n");
        s.push_str("    /memory restore <label> <rev_id>    - restore a revision\n");
        s.push_str("    /remember [text]           - ask agent to store something in memory\n");
        s.push_str("    /init                      - analyse project + init memory\n");
        s.push_str("    /search <q>                - search past messages\n");
        s.push_str("\n  Model / Toolset:\n");
        s.push_str("    /model [name]   - show current or switch model\n");
        s.push_str("    /toolset [name] - show current or switch toolset: default | codex | gemini\n");
        s.push_str("\n  Permission modes:\n");
        s.push_str("    /default        - ask before each tool  [Shift+Tab to cycle]\n");
        s.push_str("    /plan           - read-only tools; write ops blocked\n");
        s.push_str("    /yolo           - auto-approve all tools (bypassPermissions)\n");
        s.push_str("    /mode [name]    - show/set: default | plan | yolo | acceptEdits | bypassPermissions\n");
        s.push_str("\n  Direct bash (bypasses agent):\n");
        s.push_str("    ! <cmd>         - e.g.  ! git status\n");
        s.push_str("\n  Keyboard shortcuts:\n");
        s.push_str("    Shift+Enter    - insert newline (multi-line input)\n");
        s.push_str("    Esc            - clear current input line\n");
        s.push_str("    Shift+Tab      - cycle permission mode\n");
        s.push_str("    Up / Down      - navigate command history\n");
        s.push_str("    Ctrl+C         - clear line / cancel current input / interrupt running turn\n");
        s.push_str("    Ctrl+D         - exit (on empty line)\n");
        s.push_str("\n    /stream         - toggle token streaming on/off\n");
        s.push_str("    /usage          - show token usage for this session\n");
        s.push_str("    /logout         - clear API credentials and exit\n");
        s.push_str("    /feedback       - report issues\n");
        s.push_str("    /help  /?       - this message\n");
        s.push_str("    exit  /exit     - quit CADE\n");
        s
    }
}

fn truncate(s: &str, max: usize) -> String {
    super::truncate(s, max)
}

/// Return a display path relative to the current working directory if possible.
/// Delegates to the ui module's implementation.
fn make_relative_path(path: &str) -> String {
    crate::ui::make_relative_path(path)
}

/// Read a line from stdin with no echo (for API key input).
/// Falls back to normal readline if raw mode can't be set.
/// Read a password from stdin with no echo. Caller must ensure raw mode is OFF before
/// calling; this function enables and then disables raw mode internally.
fn rpassword_read() -> anyhow::Result<String> {
    use crossterm::event::{self, Event, KeyCode};
    let mut buf = String::new();
    let _raw = crate::ui::RawModeGuard::enable()?;
    loop {
        if let Ok(Event::Key(k)) = event::read() {
            match k.code {
                KeyCode::Enter     => break,
                KeyCode::Backspace => { buf.pop(); }
                KeyCode::Char(c)   => buf.push(c),
                KeyCode::Esc       => { buf.clear(); break; }
                _ => {}
            }
        }
    }
    drop(_raw); // always leave raw mode OFF on exit
    Ok(buf)
}


/// Returns (icon, label, hint) for the current permission mode.
fn mode_display(mode: PermissionMode) -> (&'static str, &'static str, &'static str) {
    match mode {
        PermissionMode::Plan               => ("📖", "plan (read-only)", "— Use /default to resume"),
        PermissionMode::BypassPermissions  => ("⚡",  "yolo",             "— All tools auto-approved"),
        PermissionMode::AcceptEdits        => ("📝",  "acceptEdits",       "— File edits auto-approved"),
        PermissionMode::Default            => ("✅",  "default",           "— Tools require approval"),
    }
}
