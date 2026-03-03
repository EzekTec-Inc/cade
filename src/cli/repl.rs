use anyhow::Result;
use std::io::{self, Write};
use crossterm::{execute, style::{Color, Print, ResetColor, SetForegroundColor}, terminal::{self, ClearType}, cursor};
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
    let trimmed = input.trim();
    if !trimmed.starts_with('/') {
        return None;
    }
    let parts: Vec<&str> = trimmed[1..].splitn(2, ' ').collect();
    let arg = parts.get(1).map(|s| s.trim().to_string()).filter(|s| !s.is_empty());
    match parts[0] {
        "help" | "?"             => Some(SlashCmd::Help),
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
        // "toolset" now handled as SlashCmd::Toolset above
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

            // Read input via TuiApp (fullscreen input widget at bottom of screen).
            let input = match self.app.lock().unwrap().read_input(&mut history, &mut hist_idx)? {
                Some(s) => s,
                None => break,
            };
            let input = input.trim().to_string();

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

            // Slash commands
            if let Some(cmd) = parse_slash(&input) {
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
                    SlashCmd::Help => {
                        let text = Self::help_text();
                        let _ = self.app.lock().unwrap().push(RenderLine::SystemMsg(text));
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
                        execute!(stdout, Print("\n"), SetForegroundColor(Color::Cyan),
                            Print("  MCP Servers\n\n"), ResetColor)?;
                        if statuses.is_empty() {
                            execute!(stdout,
                                SetForegroundColor(Color::DarkGrey),
                                Print("  No MCP servers configured.\n\n"),
                                ResetColor,
                                Print("  Add servers to ~/.cade/settings.json:\n"),
                                SetForegroundColor(Color::DarkGrey),
                                Print("  {\n    \"mcpServers\": {\n"),
                                Print("      \"git\": { \"command\": \"/path/to/git-mcp-server\" }\n"),
                                Print("    }\n  }\n\n"),
                                ResetColor,
                            )?;
                        } else {
                            for s in &statuses {
                                execute!(stdout,
                                    SetForegroundColor(Color::Green), Print("  ● "), ResetColor,
                                    SetForegroundColor(Color::White), Print(format!("{:<16}", s.key)), ResetColor,
                                    SetForegroundColor(Color::DarkGrey),
                                    Print(format!("{} tool(s)\n", s.tools.len())),
                                    ResetColor,
                                )?;
                                // Show tool names (strip prefix for clarity)
                                for chunk in s.tools.chunks(4) {
                                    let names: Vec<&str> = chunk.iter()
                                        .map(|t| t.splitn(2, "__").nth(1).unwrap_or(t))
                                        .collect();
                                    execute!(stdout,
                                        SetForegroundColor(Color::DarkGrey),
                                        Print(format!("    {}\n", names.join("  "))),
                                        ResetColor,
                                    )?;
                                }
                                let _ = self.app.lock().unwrap().push(RenderLine::Blank);
                            }
                        }
                    }
                    SlashCmd::Link => {
                        execute!(stdout, SetForegroundColor(Color::DarkGrey),
                            Print("\n  Linking tools…\n"), ResetColor)?;
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
                        execute!(stdout, SetForegroundColor(Color::Green),
                            Print(format!("  ✓ Linked {n_native} native + {n_mcp} MCP tool(s)\n")),
                            ResetColor)?;
                    }
                    SlashCmd::Unlink => {
                        let agent_id = self.agent_id();
                        match self.client.detach_agent_tools(&agent_id).await {
                            Ok(n) => {
                                execute!(stdout, SetForegroundColor(Color::Green),
                                    Print(format!("\n  ✓ Detached {n} tool(s) from agent\n")),
                                    ResetColor)?;
                            }
                            Err(e) => {
                                execute!(stdout, SetForegroundColor(Color::Red),
                                    Print(format!("\n  ✗ {e}\n")), ResetColor)?;
                            }
                        }
                    }
                    SlashCmd::Stream => {
                        use std::sync::atomic::Ordering;
                        let current = self.streaming_enabled.load(Ordering::SeqCst);
                        self.streaming_enabled.store(!current, Ordering::SeqCst);
                        let label = if !current { "on" } else { "off" };
                        execute!(stdout, SetForegroundColor(Color::Cyan),
                            Print(format!("\n  Streaming: {label}\n")), ResetColor)?;
                    }
                    SlashCmd::Usage => {
                        use std::sync::atomic::Ordering;
                        let in_tok  = self.session_input_tokens.load(Ordering::SeqCst);
                        let out_tok = self.session_output_tokens.load(Ordering::SeqCst);
                        let total   = in_tok + out_tok;
                        execute!(stdout, SetForegroundColor(Color::Cyan), Print("\n  Token usage this session:\n"), ResetColor)?;
                        execute!(stdout, SetForegroundColor(Color::DarkGrey),
                            Print(format!("    Input  : {:>8}\n", in_tok)),
                            Print(format!("    Output : {:>8}\n", out_tok)),
                            Print(format!("    Total  : {:>8}\n", total)),
                            ResetColor)?;
                        if total == 0 {
                            execute!(stdout, SetForegroundColor(Color::DarkGrey),
                                Print("    (no usage recorded yet — requires Anthropic/OpenAI)\n"),
                                ResetColor)?;
                        }
                    }
                    SlashCmd::Logout => {
                        if let Ok(mut s) = self.settings.lock() {
                            s.clear_api_key();
                        }
                        execute!(stdout, SetForegroundColor(Color::Green),
                            Print("\n  ✓ API key cleared from ~/.cade/settings.json\n"),
                            Print("    Restart CADE to re-authenticate.\n"),
                            ResetColor)?;
                        return Ok(());
                    }
                    SlashCmd::Plan => {
                        self.permissions.set_mode(PermissionMode::Plan);
                        execute!(stdout,
                            SetForegroundColor(Color::Cyan),
                            Print("\n📖 Permission mode: plan (read-only) — write/exec tools blocked\n"),
                            Print("   Use /default to resume normal mode\n"),
                            ResetColor,
                        )?;
                    }
                    SlashCmd::Default => {
                        self.permissions.set_mode(PermissionMode::Default);
                        execute!(stdout,
                            SetForegroundColor(Color::Green),
                            Print("\n✅ Permission mode: default — tools require approval\n"),
                            ResetColor,
                        )?;
                    }
                    SlashCmd::Mode(arg) => {
                        match arg.as_deref() {
                            None | Some("") => {
                                // Show current mode
                                let (icon, label, hint) = mode_display(self.permissions.mode());
                                execute!(stdout,
                                    Print(format!("\n{icon} Current mode: {label}  {hint}\n")),
                                )?;
                            }
                            Some(name) => {
                                // Switch to named mode
                                match name.to_lowercase().as_str() {
                                    "default" | "normal" => {
                                        self.permissions.set_mode(PermissionMode::Default);
                                        execute!(stdout, SetForegroundColor(Color::Green),
                                            Print("\n✅ Permission mode: default\n"), ResetColor)?;
                                    }
                                    "plan" | "readonly" | "read-only" => {
                                        self.permissions.set_mode(PermissionMode::Plan);
                                        execute!(stdout, SetForegroundColor(Color::Cyan),
                                            Print("\n📖 Permission mode: plan (read-only)\n"),
                                            Print("   Use /default to resume normal mode\n"),
                                            ResetColor)?;
                                    }
                                    "yolo" | "bypass" | "bypasspermissions" => {
                                        self.permissions.set_mode(PermissionMode::BypassPermissions);
                                        execute!(stdout, SetForegroundColor(Color::Yellow),
                                            Print("\n⚡ Permission mode: bypassPermissions\n"), ResetColor)?;
                                    }
                                    "acceptedits" | "accept-edits" | "edits" => {
                                        self.permissions.set_mode(PermissionMode::AcceptEdits);
                                        execute!(stdout, SetForegroundColor(Color::Green),
                                            Print("\n📝 Permission mode: acceptEdits — file edits auto-approved\n"), ResetColor)?;
                                    }
                                    other => {
                                        execute!(stdout, SetForegroundColor(Color::Red),
                                            Print(format!("\n  Unknown mode '{other}'\n  Valid: default | plan | yolo | acceptEdits\n")),
                                            ResetColor)?;
                                    }
                                }
                            }
                        }
                    }
                    // SlashCmd::New is handled below (hot-swap)
                    SlashCmd::Model(m) => {
                        // Empty arg → open interactive picker
                        let m = if m.is_empty() {
                            match self.interactive_model_picker(&mut stdout).await? {
                                Some(picked) => picked,
                                None => { stdout.flush()?; continue; }
                            }
                        } else {
                            m
                        };
                        let new_toolset = Toolset::for_model(&m);
                        let old_toolset = *self.current_toolset.lock().unwrap();
                        execute!(stdout, SetForegroundColor(Color::DarkGrey),
                            Print(format!("\n  Switching model → {m}…\n")), ResetColor)?;
                        match self.client.patch_agent_model(&self.agent_id(), &m).await {
                            Ok(new_model) => {
                                *self.current_model.lock().unwrap() = new_model.clone();
                                // Auto-switch toolset if model family changed
                                if new_toolset != old_toolset {
                                    *self.current_toolset.lock().unwrap() = new_toolset;
                                    // Re-register + re-attach tools for the new toolset
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
                                    execute!(stdout, SetForegroundColor(Color::Cyan),
                                        Print(format!("  Toolset → {}\n", new_toolset.display_name())),
                                        ResetColor)?;
                                }
                                execute!(stdout, SetForegroundColor(Color::Green),
                                    Print(format!("  ✓ Model: {new_model}\n")), ResetColor)?;
                            }
                            Err(e) => {
                                execute!(stdout, SetForegroundColor(Color::Red),
                                    Print(format!("  ✗ {e}\n")), ResetColor)?;
                            }
                        }
                    }

                    // ── New commands ──────────────────────────────────────────

                    SlashCmd::Clear => {
                        // Clear terminal (no MoveTo(0,0): output anchors to bottom via insert_before)
                        execute!(stdout, terminal::Clear(ClearType::All))?;
                        // Clear context window on server
                        match self.client.clear_messages(&self.agent_id()).await {
                            Ok(n) => {
                                execute!(stdout, SetForegroundColor(Color::Green),
                                    Print(format!("✓ Context window cleared ({n} messages deleted)\n")),
                                    ResetColor)?;
                            }
                            Err(e) => {
                                execute!(stdout, SetForegroundColor(Color::Yellow),
                                    Print(format!("⚠ Screen cleared (context clear failed: {e})\n")),
                                    ResetColor)?;
                            }
                        }
                    }

                    SlashCmd::New => {
                        // Start a fresh conversation on the current agent
                        let agent_id = self.agent_id();
                        match self.client.create_conversation(&agent_id, "").await {
                            Ok(conv) => {
                                let cid = conv["id"].as_str().unwrap_or("").to_string();
                                *self.conversation_id.lock().unwrap() = Some(cid.clone());
                                if let Ok(mut s) = self.session.lock() {
                                    let _ = s.set_conversation(Some(cid.clone()));
                                }
                                self.first_turn.store(true, std::sync::atomic::Ordering::SeqCst);
                                execute!(stdout, SetForegroundColor(Color::Green),
                                    Print(format!("\n  ✓ New conversation started  ({})\n", &cid[..cid.len().min(20)])),
                                    ResetColor)?;
                            }
                            Err(e) => {
                                execute!(stdout, SetForegroundColor(Color::Red),
                                    Print(format!("\n  ✗ {e}\n")), ResetColor)?;
                            }
                        }
                    }

                    SlashCmd::NewAgent => {
                        execute!(stdout, SetForegroundColor(Color::DarkGrey),
                            Print("\n  Creating new agent…\n"), ResetColor)?;
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
                                execute!(stdout, SetForegroundColor(Color::Green),
                                    Print(format!("  ✓ New agent: {} ({})\n", a.name, a.id)),
                                    ResetColor)?;

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
                            Err(e) => {
                                execute!(stdout, SetForegroundColor(Color::Red),
                                    Print(format!("  ✗ {e}\n")), ResetColor)?;
                            }
                        }
                    }

                    SlashCmd::Resume => {
                        execute!(stdout, SetForegroundColor(Color::DarkGrey),
                            Print("\n  Fetching conversations…\n"), ResetColor)?;
                        let agent_id = self.agent_id();
                        match self.client.list_conversations(&agent_id).await {
                            Ok(convs) => {
                                if convs.is_empty() {
                                    execute!(stdout, SetForegroundColor(Color::DarkGrey),
                                        Print("  No saved conversations yet. Use /new to start one.\n\n"),
                                        ResetColor)?;
                                } else if let Some(picked) = self.conversation_picker(&mut stdout, &convs, &agent_id).await? {
                                    let cid = picked["id"].as_str().unwrap_or("").to_string();
                                    *self.conversation_id.lock().unwrap() = Some(cid.clone());
                                    if let Ok(mut s) = self.session.lock() {
                                        let _ = s.set_conversation(Some(cid));
                                    }
                                    self.first_turn.store(false, std::sync::atomic::Ordering::SeqCst);
                                    execute!(stdout, SetForegroundColor(Color::Green),
                                        Print(format!("  ✓ Switched to: {}\n",
                                            picked["title"].as_str().unwrap_or("(untitled)"))),
                                        ResetColor)?;
                                }
                            }
                            Err(e) => self.print_error(&mut stdout, &e.to_string())?,
                        }
                    }

                    SlashCmd::Pin => {
                        let id   = self.agent_id();
                        let name = self.agent_name();
                        match self.settings.lock() {
                            Ok(mut s) => match s.pin_agent(&id, &name) {
                                Ok(_) => {
                                    execute!(stdout, SetForegroundColor(Color::Green),
                                        Print(format!("\n  ✓ Pinned: {name} ({id})\n")), ResetColor)?;
                                }
                                Err(e) => {
                                    execute!(stdout, SetForegroundColor(Color::Red),
                                        Print(format!("\n  ✗ Pin failed: {e}\n")), ResetColor)?;
                                }
                            },
                            Err(_) => {}
                        }
                    }

                    SlashCmd::Agents => {
                        execute!(stdout, SetForegroundColor(Color::DarkGrey),
                            Print("\n  Fetching agents…\n"), ResetColor)?;
                        match self.client.list_agents().await {
                            Ok(agents) if agents.is_empty() => {
                                execute!(stdout, Print("  (no agents found)\n"))?;
                            }
                            Ok(mut agents) => {
                                if let Some(result) = self.agent_picker(&mut stdout, &mut agents).await? {
                                    match result {
                                        AgentPickerResult::Switch(a) => {
                                            *self.agent_id.lock().unwrap()   = a.id.clone();
                                            *self.agent_name.lock().unwrap() = a.name.clone();
                                            if let Ok(mut s) = self.settings.lock() {
                                                let _ = s.set_last_agent(&a.id);
                                            }
                                            execute!(stdout, SetForegroundColor(Color::Green),
                                                Print(format!("  ✓ Switched to: {} ({})\n", a.name, a.id)),
                                                ResetColor)?;
                                        }
                                        AgentPickerResult::Rename { agent, new_name } => {
                                            match self.client.rename_agent(&agent.id, &new_name).await {
                                                Ok(_) => {
                                                    // Update live state if active agent was renamed
                                                    if agent.id == self.agent_id() {
                                                        *self.agent_name.lock().unwrap() = new_name.clone();
                                                    }
                                                    execute!(stdout, SetForegroundColor(Color::Green),
                                                        Print(format!("  ✓ Renamed '{}' → '{new_name}'\n", agent.name)),
                                                        ResetColor)?;
                                                }
                                                Err(e) => self.print_error(&mut stdout, &e.to_string())?,
                                            }
                                        }
                                        AgentPickerResult::DeleteMany(to_delete) => {
                                            let current_id = self.agent_id();
                                            let mut deleted_active = false;
                                            for a in &to_delete {
                                                match self.client.delete_agent(&a.id).await {
                                                    Ok(_) => {
                                                        execute!(stdout, SetForegroundColor(Color::Green),
                                                            Print(format!("  ✓ Deleted: {}\n", a.name)), ResetColor)?;
                                                        if a.id == current_id { deleted_active = true; }
                                                    }
                                                    Err(e) => self.print_error(&mut stdout, &e.to_string())?,
                                                }
                                            }
                                            // If the active agent was deleted, auto-switch
                                            if deleted_active {
                                                match self.client.list_agents().await {
                                                    Ok(remaining) if !remaining.is_empty() => {
                                                        let first = &remaining[0];
                                                        *self.agent_id.lock().unwrap()   = first.id.clone();
                                                        *self.agent_name.lock().unwrap() = first.name.clone();
                                                        if let Ok(mut s) = self.settings.lock() {
                                                            let _ = s.set_last_agent(&first.id);
                                                        }
                                                        execute!(stdout, SetForegroundColor(Color::DarkGrey),
                                                            Print(format!("  → Now using: {}\n", first.name)), ResetColor)?;
                                                    }
                                                    _ => {
                                                        execute!(stdout, SetForegroundColor(Color::DarkGrey),
                                                            Print("  No remaining agents — run /new to create one\n"), ResetColor)?;
                                                    }
                                                }
                                            }
                                        }
                                    }
                                }
                            }
                            Err(e) => self.print_error(&mut stdout, &e.to_string())?,
                        }
                    }

                    SlashCmd::Delete(target) => {
                        // /delete [name-or-id] — delete a specific agent by name/id prefix
                        let agents = match self.client.list_agents().await {
                            Ok(a) => a,
                            Err(e) => { self.print_error(&mut stdout, &e.to_string())?; vec![] }
                        };
                        if agents.is_empty() { execute!(stdout, Print("  (no agents)\n"))?; }
                        else if let Some(query) = target {
                            let q = query.to_lowercase();
                            let matched: Vec<_> = agents.iter()
                                .filter(|a| a.name.to_lowercase().contains(&q) || a.id.starts_with(&q))
                                .collect();
                            match matched.len() {
                                0 => self.print_error(&mut stdout, &format!("No agent matching '{query}'"))?,
                                1 => {
                                    let a = matched[0];
                                    execute!(stdout, SetForegroundColor(Color::Yellow),
                                        Print(format!("\n  Delete '{}'? [y/N]: ", a.name)), ResetColor)?;
                                    let _raw = crate::ui::RawModeGuard::enable()?;
                                    let confirmed = loop {
                                        if let Ok(crossterm::event::Event::Key(k)) = crossterm::event::read() {
                                            break matches!(k.code, KeyCode::Char('y') | KeyCode::Char('Y'));
                                        }
                                    };
                                    drop(_raw);
                                    execute!(stdout, Print("\n"))?;
                                    if confirmed {
                                        match self.client.delete_agent(&a.id).await {
                                            Ok(_) => {
                                                execute!(stdout, SetForegroundColor(Color::Green),
                                                    Print(format!("  ✓ Deleted: {}\n", a.name)), ResetColor)?;
                                                if a.id == self.agent_id() {
                                                    execute!(stdout, SetForegroundColor(Color::DarkGrey),
                                                        Print("  Active agent deleted — use /new or /agents to continue\n"), ResetColor)?;
                                                }
                                            }
                                            Err(e) => self.print_error(&mut stdout, &e.to_string())?,
                                        }
                                    } else {
                                        execute!(stdout, SetForegroundColor(Color::DarkGrey),
                                            Print("  (cancelled)\n"), ResetColor)?;
                                    }
                                }
                                n => self.print_error(&mut stdout, &format!("{n} agents match '{query}' — be more specific"))?,
                            }
                        } else {
                            // No arg → open the agents TUI picker directly in delete mode
                            execute!(stdout, SetForegroundColor(Color::DarkGrey),
                                Print("\n  /delete <name-or-id>  or  /agents then press d\n"), ResetColor)?;
                        }
                    }

                    SlashCmd::Init => {
                        // Spawn an explore subagent to do the analysis — keeps main
                        // agent's context clean, only the summary comes back.
                        execute!(stdout, SetForegroundColor(Color::DarkGrey),
                            Print(format!("  Analysing project at {}…\n", self.cwd.display())),
                            ResetColor)?;

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
                                            let _ = self.app.lock().unwrap().push(RenderLine::Blank);
                                            execute!(stdout, SetForegroundColor(Color::Cyan),
                                                Print(format!("  [{label}]")), ResetColor)?;
                                            if let Some(desc) = &b.description {
                                                if !desc.is_empty() {
                                                    execute!(stdout, SetForegroundColor(Color::DarkGrey),
                                                        Print(format!("  {desc}")), ResetColor)?;
                                                }
                                            }
                                            let _ = self.app.lock().unwrap().push(RenderLine::Blank);
                                            if b.value.is_empty() {
                                                execute!(stdout, SetForegroundColor(Color::DarkGrey),
                                                    Print("  (empty)\n"), ResetColor)?;
                                            } else {
                                                println!("{}\n", b.value);
                                            }
                                        } else {
                                            println!("\n  ✗ Block '{label}' not found");
                                        }
                                    }
                                    Err(e) => println!("\n  ✗ {e}"),
                                }
                            }
                            // /memory set <label> <value>
                            "set" if parts.len() >= 3 => {
                                let label = parts[1];
                                let value = parts[2..].join(" ");
                                let id = self.agent_id();
                                match self.client.upsert_memory(&id, label, &value, None).await {
                                    Ok(_) => println!("\n  ✓ [{label}] updated"),
                                    Err(e) => println!("\n  ✗ {e}"),
                                }
                            }
                            // /memory delete <label>
                            "delete" | "del" | "rm" if parts.len() >= 2 => {
                                let label = parts[1];
                                let id = self.agent_id();
                                match self.client.delete_memory(&id, label).await {
                                    Ok(_) => println!("\n  ✓ [{label}] deleted"),
                                    Err(e) => println!("\n  ✗ {e}"),
                                }
                            }
                            // /memory edit <label> — inline multi-line editor
                            "edit" if parts.len() >= 2 => {
                                let label = parts[1];
                                let id = self.agent_id();
                                // Fetch current value
                                let current = self.client.get_memory(&id).await
                                    .unwrap_or_default()
                                    .into_iter()
                                    .find(|b| b.label == label)
                                    .map(|b| b.value)
                                    .unwrap_or_default();

                                execute!(stdout, ResetColor)?;
                                println!("\n  Editing [{label}]");
                                let _ = self.app.lock().unwrap().push(RenderLine::SystemMsg("Current value:  ---".to_string()));
                                for line in current.lines() { println!("  {line}"); }
                                let _ = self.app.lock().unwrap().push(RenderLine::SystemMsg("---".to_string()));
                                let _ = self.app.lock().unwrap().push(RenderLine::SystemMsg("Enter new content (empty line = done, .clear = erase):".to_string()));

                                terminal::disable_raw_mode()?;
                                let mut lines: Vec<String> = Vec::new();
                                let mut clear_mode = false;
                                loop {
                                    let mut buf = String::new();
                                    std::io::stdin().read_line(&mut buf).unwrap_or(0);
                                    let line = buf.trim_end_matches('\n').trim_end_matches('\r');
                                    if line == ".clear" { clear_mode = true; break; }
                                    if line.is_empty() { break; }
                                    lines.push(line.to_string());
                                }
                                terminal::enable_raw_mode()?;

                                let new_value = if clear_mode { String::new() } else { lines.join("\n") };
                                if new_value.is_empty() && !clear_mode {
                                    let _ = self.app.lock().unwrap().push(RenderLine::SystemMsg("(cancelled — no changes)".to_string()));
                                } else {
                                    match self.client.upsert_memory(&id, label, &new_value, None).await {
                                        Ok(_) => println!("  ✓ [{label}] updated"),
                                        Err(e) => println!("  ✗ {e}"),
                                    }
                                }
                            }
                            // /memory (list)
                            _ => {
                                match self.client.get_memory(&self.agent_id()).await {
                                    Ok(blocks) if blocks.is_empty() => {
                                        execute!(stdout, ResetColor)?;
                                        let _ = self.app.lock().unwrap().push(RenderLine::SystemMsg("(no memory blocks)".to_string()));
                                        let _ = self.app.lock().unwrap().push(RenderLine::SystemMsg("Run /init to populate, or use update_memory tool".to_string()));
                                    }
                                    Ok(blocks) => {
                                        execute!(stdout, ResetColor)?;
                                        let _ = self.app.lock().unwrap().push(RenderLine::Blank);
                                        for b in &blocks {
                                            // Label + description
                                            execute!(stdout, SetForegroundColor(Color::Cyan),
                                                Print(format!("  [{}]", b.label)), ResetColor)?;
                                            if let Some(desc) = &b.description {
                                                if !desc.is_empty() {
                                                    execute!(stdout, SetForegroundColor(Color::DarkGrey),
                                                        Print(format!("  {desc}")), ResetColor)?;
                                                }
                                            }
                                            let _ = self.app.lock().unwrap().push(RenderLine::Blank);
                                            // Value preview
                                            if b.value.is_empty() {
                                                execute!(stdout, SetForegroundColor(Color::DarkGrey),
                                                    Print("  (empty)\n\n"), ResetColor)?;
                                            } else {
                                                let preview: String = b.value.chars().take(300).collect();
                                                let ellipsis = if b.value.len() > 300 { "…  (/memory view to see all)" } else { "" };
                                                println!("  {preview}{ellipsis}\n");
                                            }
                                        }
                                    }
                                    Err(e) => println!("\n  ✗ {e}"),
                                }
                            }
                        }
                    }

                    SlashCmd::Search(query) => {
                        match self.client.search_messages(&self.agent_id(), &query).await {
                            Ok(msgs) if msgs.is_empty() => {
                                execute!(stdout, SetForegroundColor(Color::DarkGrey),
                                    Print(format!("\n  No results for '{query}'\n")), ResetColor)?;
                            }
                            Ok(msgs) => {
                                execute!(stdout, Print(format!("\n  {} result(s) for '{query}':\n\n", msgs.len())))?;
                                for m in msgs.iter().take(10) {
                                    let role = m["role"].as_str().unwrap_or("?");
                                    let content = m["content"]["content"].as_str()
                                        .or_else(|| m["content"].as_str())
                                        .unwrap_or("");
                                    let preview: String = content.chars().take(120).collect();
                                    let ellipsis = if content.len() > 120 { "…" } else { "" };
                                    execute!(stdout,
                                        SetForegroundColor(Color::DarkGrey),
                                        Print(format!("  [{role}] ")),
                                        SetForegroundColor(Color::White),
                                        Print(format!("{preview}{ellipsis}\n")),
                                        ResetColor)?;
                                }
                            }
                            Err(e) => {
                                execute!(stdout, SetForegroundColor(Color::Red),
                                    Print(format!("\n  ✗ {e}\n")), ResetColor)?;
                            }
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
                                execute!(stdout, ResetColor)?;
                                if skills.is_empty() {
                                    let _ = self.app.lock().unwrap().push(RenderLine::SystemMsg("No skills loaded.".to_string()));
                                    let _ = self.app.lock().unwrap().push(RenderLine::SystemMsg("Create one: /skills create <name>".to_string()));
                                    let _ = self.app.lock().unwrap().push(RenderLine::SystemMsg("Skills dirs: .skills/  ~/.cade/skills/  ~/.cade/agents/<id>/skills/".to_string()));
                                } else {
                                    println!("\n  Skills ({} loaded):\n", skills.len());
                                    for s in skills.iter() {
                                        let cat = s.category.as_deref()
                                            .map(|c| format!("[{}]", c))
                                            .unwrap_or_default();
                                        println!("  {:<10} {:<28} {:<12} {}",
                                            format!("[{}]", s.scope), s.id, cat, s.description);
                                    }
                                    let _ = self.app.lock().unwrap().push(RenderLine::Blank);
                                    let _ = self.app.lock().unwrap().push(RenderLine::SystemMsg("Agent uses load_skill(<id>) to load full content on-demand.".to_string()));
                                }
                            }

                            "create" => {
                                let name_raw = sub_arg.trim().to_string();
                                if name_raw.is_empty() {
                                    let _ = self.app.lock().unwrap().push(RenderLine::SystemMsg("Usage: /skills create <name>".to_string()));
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
                                        println!("\n  ✗ Skill '{}' already exists: {}",
                                            slug, skill_file.display());
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
                                                        println!("\n  ✓ Created: {}", skill_file.display());
                                                        let _ = self.app.lock().unwrap().push(RenderLine::SystemMsg("Edit the file, then run /skills reload to activate it.".to_string()));
                                                    }
                                                    Err(e) => println!("\n  ✗ Failed to write skill file: {e}"),
                                                }
                                            }
                                            Err(e) => println!("\n  ✗ Failed to create directory: {e}"),
                                        }
                                    }
                                }
                            }

                            "show" => {
                                let id = sub_arg.trim();
                                let skills = self.skills.lock().unwrap();
                                match skills.iter().find(|s| s.id == id) {
                                    None => {
                                        println!("\n  Skill '{}' not found. Run /skills to list.", id);
                                    }
                                    Some(s) => {
                                        execute!(stdout, ResetColor)?;
                                        println!("\n  [{id}]");
                                        println!("  Name       : {}", s.name);
                                        println!("  Description: {}", s.description);
                                        if let Some(cat) = &s.category {
                                            println!("  Category   : {cat}");
                                        }
                                        if !s.tags.is_empty() {
                                            println!("  Tags       : {}", s.tags.join(", "));
                                        }
                                        println!("\n---\n{}\n---", s.body);
                                    }
                                }
                            }

                            "reload" => {
                                {
                                        let new_skills = crate::skills::discover_all_skills(&self.cwd, None, None);
                                        let prev_count = self.skills.lock().unwrap().len();
                                        let new_count = new_skills.len();
                                        let agent_id = self.agent_id();

                                        // Clear old per-skill blocks, upsert new ones individually
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

                                        // Compact listing for system prompt
                                        let listing = crate::skills::skills_listing(&new_skills);
                                        let _ = self.client.upsert_memory(
                                            &agent_id, "skills", listing.as_deref().unwrap_or(""), None
                                        ).await;

                                        *self.skills.lock().unwrap() = new_skills;

                                        println!("\n  ✓ Reloaded: {new_count} skills (was {prev_count})");
                                        let _ = self.app.lock().unwrap().push(RenderLine::SystemMsg("✓ Skills listing updated (on-demand via load_skill)".to_string()));

                                        // Notify agent in current conversation
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
                                println!("\n  Unknown /skills subcommand: '{other}'");
                                let _ = self.app.lock().unwrap().push(RenderLine::SystemMsg("Usage: /skills [list | create <name> | show <id> | reload]".to_string()));
                            }
                        }
                    }

                    SlashCmd::Subagents => {
                        let all = discover_all_subagents(&self.cwd);
                        execute!(stdout, ResetColor)?;
                        println!("\n  Available subagents ({}):\n", all.len());
                        for def in &all {
                            println!("{}", def.summary());
                        }
                        let _ = self.app.lock().unwrap().push(RenderLine::Blank);
                        let _ = self.app.lock().unwrap().push(RenderLine::SystemMsg("Usage: ask the agent to run_subagent(type, task)".to_string()));
                        let _ = self.app.lock().unwrap().push(RenderLine::SystemMsg("Custom: create .cade/agents/<name>.md in this project".to_string()));
                        let _ = self.app.lock().unwrap().push(RenderLine::SystemMsg("Global: create ~/.cade/agents/<name>.md".to_string()));
                    }

                    SlashCmd::Providers => {
                        match self.client.list_providers().await {
                            Ok(body) => {
                                let empty = vec![];
                                let providers = body["providers"].as_array().unwrap_or(&empty);
                                println!("\n  Configured providers ({}):\n", providers.len());
                                for p in providers {
                                    let name    = p["name"].as_str().unwrap_or("?");
                                    let kind    = p["kind"].as_str().unwrap_or("?");
                                    let live    = p["live"].as_bool().unwrap_or(false);
                                    let source  = p["source"].as_str().unwrap_or("db");
                                    let enabled = p["enabled"].as_bool().unwrap_or(true);
                                    let status  = if live { "✓ live" } else { "✗ offline" };
                                    execute!(stdout,
                                        SetForegroundColor(if live { Color::Green } else { Color::Red }),
                                        Print(format!("  {status:<10}")),
                                        ResetColor,
                                        Print(format!("{:<18} [{kind}] ({source})\n",
                                            if enabled { name.to_string() } else { format!("{name} (disabled)") }
                                        ))
                                    )?;
                                }
                                let _ = self.app.lock().unwrap().push(RenderLine::Blank);
                                let _ = self.app.lock().unwrap().push(RenderLine::SystemMsg("/connect <name>    — add a provider".to_string()));
                                let _ = self.app.lock().unwrap().push(RenderLine::SystemMsg("/disconnect <name> — remove a provider".to_string()));
                                let presets = self.client.list_provider_presets().await;
                                if !presets.is_empty() {
                                    let _ = self.app.lock().unwrap().push(RenderLine::SystemMsg("OpenAI-compatible presets:".to_string()));
                                    for p in &presets {
                                        let n = p["name"].as_str().unwrap_or("?");
                                        let u = p["base_url"].as_str().unwrap_or("?");
                                        println!("    /connect {n:<14} — {u}");
                                    }
                                }
                                let _ = self.app.lock().unwrap().push(RenderLine::Blank);
                            }
                            Err(e) => self.print_error(&mut stdout, &e.to_string())?,
                        }
                    }

                    SlashCmd::Connect(preset) => {
                        self.handle_connect(preset, &mut stdout).await?;
                    }

                    SlashCmd::Disconnect(name) => {
                        if name.is_empty() {
                            self.print_error(&mut stdout, "/disconnect requires a provider name")?;
                        } else {
                            execute!(stdout, SetForegroundColor(Color::DarkGrey),
                                Print(format!("\n  Disconnecting provider '{name}'…\n")), ResetColor)?;
                            match self.client.remove_provider(&name).await {
                                Ok(_) => {
                                    execute!(stdout, SetForegroundColor(Color::Green),
                                        Print(format!("  ✓ Provider '{name}' removed\n")), ResetColor)?;
                                }
                                Err(e) => self.print_error(&mut stdout, &e.to_string())?,
                            }
                        }
                    }

                    SlashCmd::Permissions => {
                        let mode  = self.permissions.mode();
                        let allow = self.permissions.allow_rules();
                        let deny  = self.permissions.deny_rules();

                        // Mode header — icon + label in cyan, hint in grey
                        let (icon, label, _) = mode_display(mode);
                        let mode_hint = match mode {
                            crate::permissions::PermissionMode::Default           => "ask before each tool call",
                            crate::permissions::PermissionMode::AcceptEdits       => "file edits auto-approved; Bash still prompts",
                            crate::permissions::PermissionMode::Plan              => "read-only; write operations blocked",
                            crate::permissions::PermissionMode::BypassPermissions => "all tools auto-approved (deny rules still apply)",
                        };
                        execute!(stdout,
                            Print("\n"),
                            Print("  Mode: "),
                            SetForegroundColor(Color::Cyan),
                            Print(format!("{icon} {label}")),
                            ResetColor,
                            SetForegroundColor(Color::DarkGrey),
                            Print(format!("  —  {mode_hint}\n\n")),
                            ResetColor,
                        )?;

                        // Rules
                        if allow.is_empty() && deny.is_empty() {
                            execute!(stdout, SetForegroundColor(Color::DarkGrey),
                                Print("  No allow/deny rules active.\n\n"), ResetColor)?;
                        } else {
                            if !allow.is_empty() {
                                execute!(stdout, SetForegroundColor(Color::Green),
                                    Print(format!("  Allow rules ({}):\n", allow.len())), ResetColor)?;
                                for r in &allow {
                                    execute!(stdout,
                                        Print("    "),
                                        SetForegroundColor(Color::Cyan),
                                        Print(format!("{:<12}", r.tool())),
                                        ResetColor,
                                        SetForegroundColor(Color::DarkGrey),
                                        Print(r.arg_display()),
                                        ResetColor,
                                        Print("\n"),
                                    )?;
                                }
                                let _ = self.app.lock().unwrap().push(RenderLine::Blank);
                            }
                            if !deny.is_empty() {
                                execute!(stdout, SetForegroundColor(Color::Red),
                                    Print(format!("  Deny rules ({}):\n", deny.len())), ResetColor)?;
                                for r in &deny {
                                    execute!(stdout,
                                        Print("    "),
                                        SetForegroundColor(Color::Cyan),
                                        Print(format!("{:<12}", r.tool())),
                                        ResetColor,
                                        SetForegroundColor(Color::DarkGrey),
                                        Print(r.arg_display()),
                                        ResetColor,
                                        Print("\n"),
                                    )?;
                                }
                                let _ = self.app.lock().unwrap().push(RenderLine::Blank);
                            }
                        }

                        // Usage hints with color-coded command names
                        execute!(stdout,
                            SetForegroundColor(Color::Green),  Print("  /approve-always"),
                            ResetColor,                        Print(" <pattern>    "),
                            SetForegroundColor(Color::Red),    Print("/deny-always"),
                            ResetColor,                        Print(" <pattern>\n"),
                            SetForegroundColor(Color::DarkGrey),
                            Print("  Pattern:  Bash(cargo test)  ·  Read(src/**)  ·  Bash(rm -rf:*)\n\n"),
                            ResetColor,
                        )?;
                    }

                    SlashCmd::ApproveAlways(pattern) => {
                        if pattern.is_empty() {
                            execute!(stdout,
                                Print("\n  "),
                                SetForegroundColor(Color::Green), Print("/approve-always"), ResetColor,
                                SetForegroundColor(Color::DarkGrey), Print(" <pattern>\n"), ResetColor,
                                SetForegroundColor(Color::DarkGrey),
                                Print("  Pattern examples:\n"),
                                Print("    Bash(cargo test)    — exact command\n"),
                                Print("    Read(src/**)        — path prefix\n"),
                                Print("    Bash(git commit:*)  — command prefix\n"),
                                Print("    Bash                — all bash calls\n\n"),
                                ResetColor,
                            )?;
                        } else if let Some(rule) = crate::permissions::PermissionRule::parse(&pattern) {
                            self.permissions.add_allow_rule(rule.clone());
                            execute!(stdout,
                                Print("\n  "),
                                SetForegroundColor(Color::Green), Print("✓ Allow  "), ResetColor,
                                SetForegroundColor(Color::Cyan),  Print(format!("{:<12}", rule.tool())), ResetColor,
                                SetForegroundColor(Color::DarkGrey), Print(rule.arg_display()), ResetColor,
                                Print("\n"),
                            )?;
                            execute!(stdout, SetForegroundColor(Color::DarkGrey),
                                Print("  Save to settings.json? [y/N] "), ResetColor)?;
                            let _raw = crate::ui::RawModeGuard::enable()?;
                            let save = loop {
                                if let Ok(crossterm::event::Event::Key(k)) = crossterm::event::read() {
                                    match k.code {
                                        crossterm::event::KeyCode::Char('y') | crossterm::event::KeyCode::Char('Y') => break true,
                                        _ => break false,
                                    }
                                }
                            };
                            drop(_raw);
                            execute!(stdout, SetForegroundColor(if save { Color::Green } else { Color::DarkGrey }),
                                Print(if save { "y\n" } else { "N\n" }), ResetColor)?;
                            if save {
                                let mut settings = self.settings.lock().unwrap();
                                match settings.save_allow_rule(&pattern) {
                                    Ok(_) => execute!(stdout, SetForegroundColor(Color::Green),
                                        Print("  ✓ Saved\n"), ResetColor)?,
                                    Err(e) => self.print_error(&mut stdout, &e.to_string())?,
                                }
                            }
                            let _ = self.app.lock().unwrap().push(RenderLine::Blank);
                        } else {
                            self.print_error(&mut stdout, &format!("invalid pattern: {pattern:?}\n  Expected: Tool  or  Tool(arg)  or  Tool(prefix:*)"))?;
                        }
                    }

                    SlashCmd::DenyAlways(pattern) => {
                        if pattern.is_empty() {
                            execute!(stdout,
                                Print("\n  "),
                                SetForegroundColor(Color::Red), Print("/deny-always"), ResetColor,
                                SetForegroundColor(Color::DarkGrey), Print(" <pattern>\n"), ResetColor,
                                SetForegroundColor(Color::DarkGrey),
                                Print("  Pattern examples:\n"),
                                Print("    Bash(rm -rf:*)          — prefix wildcard\n"),
                                Print("    Bash(git push --force)  — exact command\n"),
                                Print("    Bash                    — all bash calls\n\n"),
                                ResetColor,
                            )?;
                        } else if let Some(rule) = crate::permissions::PermissionRule::parse(&pattern) {
                            self.permissions.add_deny_rule(rule.clone());
                            execute!(stdout,
                                Print("\n  "),
                                SetForegroundColor(Color::Red),  Print("✗ Deny   "), ResetColor,
                                SetForegroundColor(Color::Cyan), Print(format!("{:<12}", rule.tool())), ResetColor,
                                SetForegroundColor(Color::DarkGrey), Print(rule.arg_display()), ResetColor,
                                Print("\n"),
                            )?;
                            execute!(stdout, SetForegroundColor(Color::DarkGrey),
                                Print("  Save to settings.json? [y/N] "), ResetColor)?;
                            let _raw = crate::ui::RawModeGuard::enable()?;
                            let save = loop {
                                if let Ok(crossterm::event::Event::Key(k)) = crossterm::event::read() {
                                    match k.code {
                                        crossterm::event::KeyCode::Char('y') | crossterm::event::KeyCode::Char('Y') => break true,
                                        _ => break false,
                                    }
                                }
                            };
                            drop(_raw);
                            execute!(stdout, SetForegroundColor(if save { Color::Red } else { Color::DarkGrey }),
                                Print(if save { "y\n" } else { "N\n" }), ResetColor)?;
                            if save {
                                let mut settings = self.settings.lock().unwrap();
                                match settings.save_deny_rule(&pattern) {
                                    Ok(_) => execute!(stdout, SetForegroundColor(Color::Red),
                                        Print("  ✓ Saved\n"), ResetColor)?,
                                    Err(e) => self.print_error(&mut stdout, &e.to_string())?,
                                }
                            }
                            let _ = self.app.lock().unwrap().push(RenderLine::Blank);
                        } else {
                            self.print_error(&mut stdout, &format!("invalid pattern: {pattern:?}\n  Expected: Tool  or  Tool(arg)  or  Tool(prefix:*)"))?;
                        }
                    }

                    SlashCmd::Hooks => {
                        let merged = self.settings.lock().unwrap().merged_hooks();
                        if merged.is_empty() {
                            execute!(stdout,
                                Print("\n"),
                                SetForegroundColor(Color::DarkGrey),
                                Print("  No hooks configured.\n\n"),
                                ResetColor,
                                Print("  Configure in "),
                                SetForegroundColor(Color::Cyan), Print("~/.cade/settings.json"), ResetColor,
                                Print(" or "),
                                SetForegroundColor(Color::Cyan), Print(".cade/settings.json\n\n"), ResetColor,
                                SetForegroundColor(Color::DarkGrey),
                                Print("  Example:\n"),
                                Print("  {\n"),
                                Print("    \"hooks\": {\n"),
                                Print("      \"PreToolUse\": [{\n"),
                                Print("        \"matcher\": \"Bash\",\n"),
                                Print("        \"hooks\": [{ \"type\": \"command\", \"command\": \"./hooks/validate.sh\" }]\n"),
                                Print("      }],\n"),
                                Print("      \"Stop\": [{\n"),
                                Print("        \"hooks\": [{ \"type\": \"command\", \"command\": \"./hooks/run-tests.sh\" }]\n"),
                                Print("      }]\n"),
                                Print("    }\n"),
                                Print("  }\n\n"),
                                Print("  Exit codes:  0=allow  1=log+continue  2=block (stderr→agent)\n\n"),
                                ResetColor,
                            )?;
                        } else {
                            execute!(stdout,
                                Print("\n  "),
                                SetForegroundColor(Color::Cyan), Print("Hooks"), ResetColor,
                                Print("\n\n"),
                            )?;
                            let stdout_ref = &mut stdout;
                            macro_rules! show_hook_section {
                                ($name:expr, $entries:expr, $color:expr) => {
                                    if !$entries.is_empty() {
                                        execute!(stdout_ref,
                                            Print("  "),
                                            SetForegroundColor($color), Print($name), ResetColor,
                                            SetForegroundColor(Color::DarkGrey),
                                            Print(format!("  ({})\n", $entries.len())),
                                            ResetColor,
                                        )?;
                                        for entry in $entries {
                                            let m = entry.matcher.as_deref().unwrap_or("*");
                                            execute!(stdout_ref,
                                                SetForegroundColor(Color::DarkGrey),
                                                Print(format!("    matcher: {m}\n")),
                                                ResetColor,
                                            )?;
                                            for hook in &entry.hooks {
                                                execute!(stdout_ref,
                                                    SetForegroundColor(Color::DarkGrey),
                                                    Print(format!("      {hook}\n")),
                                                    ResetColor,
                                                )?;
                                            }
                                        }
                                        let _ = self.app.lock().unwrap().push(RenderLine::Blank);
                                    }
                                };
                            }
                            show_hook_section!("PreToolUse",         &merged.pre_tool_use,          Color::Yellow);
                            show_hook_section!("PostToolUse",        &merged.post_tool_use,         Color::Green);
                            show_hook_section!("PostToolUseFailure", &merged.post_tool_use_failure,  Color::Red);
                            show_hook_section!("PermissionRequest",  &merged.permission_request,    Color::Yellow);
                            show_hook_section!("UserPromptSubmit",   &merged.user_prompt_submit,    Color::Cyan);
                            show_hook_section!("Stop",               &merged.stop,                  Color::Cyan);
                            show_hook_section!("SubagentStop",       &merged.subagent_stop,         Color::Cyan);
                            show_hook_section!("SessionStart",       &merged.session_start,         Color::DarkGrey);
                            show_hook_section!("SessionEnd",         &merged.session_end,           Color::DarkGrey);
                            show_hook_section!("Notification",       &merged.notification,          Color::DarkGrey);
                            execute!(stdout, SetForegroundColor(Color::DarkGrey),
                                Print("  Config: ~/.cade/settings.json  ·  .cade/settings.json  ·  .cade/settings.local.json\n\n"),
                                ResetColor)?;
                        }
                    }

                    SlashCmd::Rename(new_name) => {
                        let id = self.agent_id();
                        let new_name = new_name.trim().to_string();
                        let name = if new_name.is_empty() {
                            // Prompt for name interactively
                            execute!(stdout, SetForegroundColor(Color::DarkGrey),
                                Print("\n  New name: "), ResetColor)?;
                            terminal::disable_raw_mode()?;
                            let mut buf = String::new();
                            std::io::stdin().read_line(&mut buf).unwrap_or(0);
                            terminal::enable_raw_mode()?;
                            buf.trim().to_string()
                        } else {
                            new_name
                        };
                        if name.is_empty() {
                            execute!(stdout, SetForegroundColor(Color::DarkGrey),
                                Print("  (cancelled)\n"), ResetColor)?;
                        } else {
                            match self.client.rename_agent(&id, &name).await {
                                Ok(_) => {
                                    *self.agent_name.lock().unwrap() = name.clone();
                                    execute!(stdout, SetForegroundColor(Color::Green),
                                        Print(format!("\n  ✓ Renamed to: {name}\n")), ResetColor)?;
                                }
                                Err(e) => self.print_error(&mut stdout, &e.to_string())?,
                            }
                        }
                    }

                    SlashCmd::Toolset(arg) => {
                        // Reuse the model command's toolset-switching logic
                        let fake_arg = format!("__toolset__{}", arg.as_deref().unwrap_or(""));
                        // Delegate by recursing into the Model handler logic inline
                        let old_toolset = *self.current_toolset.lock().unwrap();
                        let new_toolset = if let Some(name) = arg.as_deref() {
                            match crate::toolsets::Toolset::from_str(name) {
                                Some(t) => t,
                                None => {
                                    execute!(stdout, SetForegroundColor(Color::DarkGrey),
                                        Print("\n  Toolsets: default | codex | gemini\n\n"), ResetColor)?;
                                    continue;
                                }
                            }
                        } else {
                            // No arg — show current and options
                            execute!(stdout,
                                Print("\n  Current: "),
                                SetForegroundColor(Color::Cyan),
                                Print(format!("{old_toolset:?}")),
                                ResetColor,
                                Print("\n"),
                                SetForegroundColor(Color::DarkGrey),
                                Print("  /toolset default | codex | gemini\n\n"),
                                ResetColor,
                            )?;
                            continue;
                        };
                        let _ = fake_arg; // silence unused warning
                        if new_toolset != old_toolset {
                            *self.current_toolset.lock().unwrap() = new_toolset;
                            execute!(stdout, SetForegroundColor(Color::Green),
                                Print(format!("\n  ✓ Toolset: {new_toolset:?}\n")), ResetColor)?;
                        } else {
                            execute!(stdout, SetForegroundColor(Color::DarkGrey),
                                Print(format!("\n  Toolset already: {new_toolset:?}\n")), ResetColor)?;
                        }
                    }

                    SlashCmd::Feedback => {
                        execute!(stdout,
                            SetForegroundColor(Color::Cyan),
                            Print("\n  Report issues or give feedback:\n"),
                            SetForegroundColor(Color::White),
                            Print("  https://github.com/EzekTec-Inc/CADE/issues\n"),
                            ResetColor)?;
                    }
                }
                continue;
            }

            // UserPromptSubmit hook — can block the turn
            if let crate::hooks::HookOutcome::Block { reason } =
                self.hooks.user_prompt_submit(&input).await
            {
                execute!(stdout, SetForegroundColor(Color::Yellow),
                    Print(format!("\n  ⚠ Hook blocked prompt: {reason}\n")), ResetColor)?;
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
            loop {
                tokio::time::sleep(tokio::time::Duration::from_millis(120)).await;
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
                // Try to redraw; skip if the lock is contended (brief miss is OK).
                if let Ok(mut app) = tick_app.try_lock() {
                    let _ = app.draw();
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
    fn prompt_approval(
        &self,
        _stdout: &mut io::Stdout,
        tool_name: &str,
        args: &serde_json::Value,
    ) -> Result<bool> {
        use crate::ui::question::{Question, QuestionOption, QuestionWidget};

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

        execute!(stdout, SetForegroundColor(Color::DarkGrey),
            Print(format!("  Downloading skill from {}…\n", url)), ResetColor)?;

        match crate::skills::install_skill_from_url(&url, &target_dir).await {
            Ok(skill) => {
                let name = skill.name.clone();
                let id   = skill.id.clone();
                // Add to in-memory list
                self.skills.lock().unwrap().push(skill);
                // Update agent memory listing
                let agent_id = self.agent_id();
                let skills   = self.skills.lock().unwrap().clone();
                let listing  = crate::skills::skills_listing(&skills);
                let _ = self.client.upsert_memory(
                    &agent_id, "skills",
                    listing.as_deref().unwrap_or(""), None
                ).await;
                drop(skills);

                execute!(stdout, SetForegroundColor(Color::Green),
                    Print(format!("  ✓ Installed: {name} [{id}]\n")), ResetColor)?;

                Ok(crate::tools::ToolResult {
                    tool_call_id: call_id.to_string(),
                    tool_name: "install_skill".to_string(),
                    output: format!("Skill '{name}' installed as [{id}] in {scope} scope. It is now available via load_skill(\"{id}\")."),
                    is_error: false,
                })
            }
            Err(e) => {
                execute!(stdout, SetForegroundColor(Color::Red),
                    Print(format!("  ✗ Install failed: {e}\n")), ResetColor)?;
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
        stdout: &mut io::Stdout,
    ) -> Result<()> {
        use crossterm::event::{self, Event, KeyCode};

        // Known built-in provider types (non-OpenAI-compatible)
        const BUILTIN: &[(&str, &str)] = &[
            ("anthropic", "Anthropic (Claude models)"),
            ("openai",    "OpenAI (GPT / Codex models)"),
            ("gemini",    "Google Gemini"),
            ("ollama",    "Ollama (local models, no key needed)"),
        ];

        // Fetch presets from server
        let presets = self.client.list_provider_presets().await;

        // If a preset name was given (e.g. /connect openrouter) skip the picker
        let (name, kind, default_base_url) = if let Some(p) = preset {
            // Check built-in first
            if let Some(&(n, _)) = BUILTIN.iter().find(|(n, _)| *n == p.as_str()) {
                (n.to_string(), n.to_string(), None)
            } else if let Some(preset_val) = presets.iter().find(|v| v["name"].as_str() == Some(&p)) {
                let base = preset_val["base_url"].as_str().map(String::from);
                (p.clone(), "openai-compatible".to_string(), base)
            } else {
                // Treat as custom openai-compatible
                (p.clone(), "openai-compatible".to_string(), None)
            }
        } else {
            // Interactive picker
            let _ = self.app.lock().unwrap().push(RenderLine::Blank);
            execute!(stdout, SetForegroundColor(Color::Cyan),
                Print("  /connect — Choose a provider\n\n"), ResetColor)?;

            let mut all_options: Vec<(String, String, Option<String>)> = BUILTIN.iter()
                .map(|(n, label)| (n.to_string(), label.to_string(), None))
                .collect();
            for p in &presets {
                let n = p["name"].as_str().unwrap_or("?").to_string();
                let u = p["base_url"].as_str().map(String::from);
                all_options.push((n.clone(), format!("{n} (OpenAI-compatible)"), u));
            }
            all_options.push(("custom".to_string(), "Custom OpenAI-compatible URL…".to_string(), None));

            let total = all_options.len();
            let mut sel = 0usize;

            // Lines per render: total_options + 1 blank + 1 hint = total + 2
            let connect_draw_lines = (all_options.len() + 2) as u16;

            // Draw list — \r\n required in raw mode
            let draw_list = |stdout: &mut io::Stdout, sel: usize| -> Result<()> {
                execute!(stdout, cursor::MoveToColumn(0),
                    terminal::Clear(terminal::ClearType::FromCursorDown))?;
                for (i, (_, label, _)) in all_options.iter().enumerate() {
                    let arrow = if i == sel { "  ▶ " } else { "    " };
                    let color = if i == sel { Color::White } else { Color::DarkGrey };
                    execute!(stdout,
                        SetForegroundColor(if i == sel { Color::Green } else { Color::DarkGrey }),
                        Print(arrow),
                        SetForegroundColor(color),
                        Print(format!("{label}\r\n")),
                        ResetColor,
                    )?;
                }
                execute!(stdout, SetForegroundColor(Color::DarkGrey),
                    Print("\r\n  ↑↓ / j/k navigate  Enter select  Esc cancel\r\n"), ResetColor)?;
                Ok(())
            };

            let _raw = crate::ui::RawModeGuard::enable()?;
            execute!(stdout, cursor::Hide)?;
            draw_list(stdout, sel)?;

            let chosen = loop {
                if let Ok(Event::Key(key)) = event::read() {
                    match key.code {
                        KeyCode::Esc | KeyCode::Char('q') => { break None; }
                        KeyCode::Enter => { break Some(sel); }
                        KeyCode::Up | KeyCode::Char('k') => {
                            sel = if sel == 0 { total - 1 } else { sel - 1 };
                            execute!(stdout, cursor::MoveToPreviousLine(connect_draw_lines))?;
                            draw_list(stdout, sel)?;
                        }
                        KeyCode::Down | KeyCode::Char('j') => {
                            sel = (sel + 1) % total;
                            execute!(stdout, cursor::MoveToPreviousLine(connect_draw_lines))?;
                            draw_list(stdout, sel)?;
                        }
                        _ => {}
                    }
                }
            };

            drop(_raw);
            execute!(stdout, cursor::Show, ResetColor, Print("\r\n"))?;

            let Some(idx) = chosen else { return Ok(()); };
            let (n, _, base) = all_options.remove(idx);
            let k = if BUILTIN.iter().any(|(bn, _)| *bn == n.as_str()) {
                n.clone()
            } else {
                "openai-compatible".to_string()
            };
            (n, k, base)
        };

        // Prompt for API key (masked input)
        let needs_key = kind != "ollama";
        let api_key = if needs_key {
            execute!(stdout, SetForegroundColor(Color::DarkGrey),
                Print(format!("\n  API key for '{name}' (input hidden, Enter to skip): ")),
                ResetColor)?;
            terminal::disable_raw_mode()?;
            let key = rpassword_read()?;
            // rpassword_read leaves raw mode off — add newline in normal mode
            execute!(stdout, Print("\r\n"))?;
            if key.trim().is_empty() { None } else { Some(key.trim().to_string()) }
        } else {
            None
        };

        // For custom: prompt for base URL
        let base_url = if kind == "openai-compatible" && default_base_url.is_none() {
            execute!(stdout, SetForegroundColor(Color::DarkGrey),
                Print("  Base URL (e.g. https://api.example.com/v1/chat/completions): "),
                ResetColor)?;
            let mut line = String::new();
            terminal::disable_raw_mode()?;
            std::io::stdin().read_line(&mut line).ok();
            terminal::enable_raw_mode()?;
            let u = line.trim().to_string();
            if u.is_empty() { None } else { Some(u) }
        } else {
            default_base_url
        };

        execute!(stdout, SetForegroundColor(Color::DarkGrey),
            Print(format!("\n  Connecting to '{name}'…\n")), ResetColor)?;

        match self.client.add_provider(
            &name,
            &kind,
            api_key.as_deref(),
            base_url.as_deref(),
        ).await {
            Ok(_) => {
                execute!(stdout, SetForegroundColor(Color::Green),
                    Print(format!("  ✓ Provider '{name}' connected and hot-loaded\n")),
                    ResetColor)?;
                execute!(stdout, SetForegroundColor(Color::DarkGrey),
                    Print(format!("    Use: /model {name}/<model-name>\n")),
                    ResetColor)?;
            }
            Err(e) => self.print_error(stdout, &e.to_string())?,
        }
        Ok(())
    }

    /// `/resume` conversation picker — ratatui Viewport::Inline.
    ///
    /// Keys: ↑/↓ move · Enter select · d delete · Esc/q cancel.
    /// Returns the picked conversation JSON, or None if cancelled.
    async fn conversation_picker(
        &self,
        stdout: &mut io::Stdout,
        convs: &[serde_json::Value],
        agent_id: &str,
    ) -> Result<Option<serde_json::Value>> {
        use crossterm::event::{self, Event, KeyCode, KeyModifiers};
        use ratatui::{
            Terminal,
            backend::CrosstermBackend,

            style::{Color as RC, Modifier, Style},
            text::{Line, Span},
            widgets::{Block, Borders, List, ListItem, ListState},
            Viewport,
        };

        if convs.is_empty() {
            return Ok(None);
        }

        let term_h = crossterm::terminal::size().map(|(_, h)| h as u16).unwrap_or(24);
        let view_h = (convs.len() as u16 + 2).min(term_h.saturating_sub(2)).max(4);

        let backend  = CrosstermBackend::new(io::stdout());
        let mut term = Terminal::with_options(
            backend,
            ratatui::TerminalOptions { viewport: Viewport::Inline(view_h) },
        )?;

        let _raw = crate::ui::RawModeGuard::enable()?;
        let mut sel:    usize = 0;
        let mut result: Option<serde_json::Value> = None;
        let mut list_state = ListState::default().with_selected(Some(0));

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
                let label = format!("{title}  ({cnt} msgs)  {date}");
                let style = if i == sel {
                    Style::default().fg(RC::Black).bg(RC::Cyan).add_modifier(Modifier::BOLD)
                } else {
                    Style::default().fg(RC::White)
                };
                ListItem::new(Line::from(vec![Span::styled(label, style)]))
            }).collect()
        };

        macro_rules! redraw {
            () => {{
                let items = build_items(sel);
                let n     = convs.len();
                term.draw(|f| {
                    let area = f.area();
                    let block = Block::default()
                        .borders(Borders::ALL)
                        .title(format!(" Conversations [{}/{}] · Enter select · d delete · Esc cancel ", sel + 1, n))
                        .border_style(Style::default().fg(RC::Cyan));
                    let list = List::new(items).block(block)
                        .highlight_style(Style::default().fg(RC::Black).bg(RC::Cyan));
                    f.render_stateful_widget(list, area, &mut list_state);
                })?;
                list_state = ListState::default().with_selected(Some(sel));
            }};
        }

        redraw!();

        loop {
            if !event::poll(std::time::Duration::from_millis(200))? { continue; }
            match event::read()? {
                Event::Key(k) => match (k.code, k.modifiers) {
                    (KeyCode::Char('q') | KeyCode::Esc, _) => break,
                    (KeyCode::Up   | KeyCode::Char('k'), _) => { if sel > 0 { sel -= 1; } redraw!(); }
                    (KeyCode::Down | KeyCode::Char('j'), _) => {
                        if sel + 1 < convs.len() { sel += 1; }
                        redraw!();
                    }
                    (KeyCode::Enter, _) => {
                        result = convs.get(sel).cloned();
                        break;
                    }
                    (KeyCode::Char('d') | KeyCode::Delete, _) => {
                        // Confirm + delete
                        let conv_id = convs[sel]["id"].as_str().unwrap_or("").to_string();
                        let title   = convs[sel]["title"].as_str().unwrap_or("(untitled)").to_string();
                        term.clear()?;
                        drop(term);
                        crossterm::terminal::disable_raw_mode()?;
                        execute!(stdout,
                            SetForegroundColor(Color::Yellow),
                            Print(format!("\n  Delete conversation \"{title}\"? [y/N] ")),
                            ResetColor)?;
                        crossterm::terminal::enable_raw_mode()?;
                        if let Ok(Event::Key(k2)) = event::read() {
                            crossterm::terminal::disable_raw_mode()?;
                            if matches!(k2.code, KeyCode::Char('y') | KeyCode::Char('Y')) {
                                let _ = self.client.delete_conversation(agent_id, &conv_id).await;
                                execute!(stdout, SetForegroundColor(Color::Green),
                                    Print("  Deleted.\n"), ResetColor)?;
                            } else {
                                execute!(stdout, Print("\n"))?;
                            }
                        } else {
                            crossterm::terminal::disable_raw_mode()?;
                        }
                        return Ok(None); // close picker after delete
                    }
                    (KeyCode::Char('c'), KeyModifiers::CONTROL) => break,
                    _ => {}
                },
                _ => {}
            }
        }

        term.clear()?;
        drop(term);
        drop(_raw);
        Ok(result)
    }

    /// `/agents` TUI picker — rendered via ratatui `Viewport::Inline`.
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
        stdout: &mut io::Stdout,
        agents: &mut Vec<AgentState>,
    ) -> Result<Option<AgentPickerResult>> {
        use crossterm::event::{self, Event, KeyCode};
        use std::collections::HashSet;
        use ratatui::{
            Terminal, TerminalOptions, Viewport,
            backend::CrosstermBackend,
            widgets::{List, ListItem, ListState, Block},
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

        // ── Build list items ─────────────────────────────────────────────────
        // Returns owned ListItems so the ratatui draw closure is 'static-safe.
        let build_items = |agents: &[AgentState], sel: usize,
                           marked: &HashSet<usize>, current: &str| -> Vec<ListItem<'static>> {
            agents.iter().enumerate().map(|(i, a)| {
                let is_sel    = i == sel;
                let is_marked = marked.contains(&i);
                let is_active = a.id == current;
                let short_id  = if a.id.len() > 16 { a.id[..16].to_string() + "…" }
                                else { a.id.clone() };

                let arrow_style = Style::default().fg(if is_sel { RC::Green } else { RC::DarkGray });
                let name_style  = Style::default()
                    .fg(if is_sel { RC::White } else { RC::DarkGray })
                    .add_modifier(if is_sel { Modifier::BOLD } else { Modifier::empty() });
                let check_style = Style::default()
                    .fg(if is_marked { RC::Yellow } else { RC::DarkGray });

                ListItem::new(Line::from(vec![
                    Span::styled(if is_sel { "▶ " } else { "  " }.to_string(), arrow_style),
                    Span::styled(if is_marked { "☑ " } else { "☐ " }.to_string(), check_style),
                    Span::styled(format!("{:<30}", a.name), name_style),
                    Span::styled(short_id, Style::default().fg(RC::DarkGray)),
                    Span::styled(
                        if is_active { "  ← active".to_string() } else { String::new() },
                        Style::default().fg(RC::Cyan),
                    ),
                ]))
            }).collect()
        };

        // ── Terminal setup ───────────────────────────────────────────────────
        let term_rows = terminal::size().map(|(_, r)| r as usize).unwrap_or(24);
        let view_h    = (total + 3).min(term_rows.saturating_sub(2)).max(3) as u16;

        let _raw = crate::ui::RawModeGuard::enable()?;
        execute!(stdout, cursor::Hide)?;
        let mut term = Terminal::with_options(
            CrosstermBackend::new(&mut *stdout),
            TerminalOptions { viewport: Viewport::Inline(view_h) },
        )?;

        // ── Draw helper ──────────────────────────────────────────────────────
        macro_rules! redraw {
            () => {{
                let items = build_items(agents, selected, &marked, &current);
                let n     = marked.len();
                let title = if n == 0 {
                    " Agents  ↑↓/jk navigate  Space mark  r rename  d delete  Enter switch  q cancel "
                        .to_string()
                } else {
                    format!(" Agents  [{n} marked]  d delete all marked  Esc/q cancel ")
                };
                let list = List::new(items)
                    .block(Block::default().title(title));
                let mut ls = ListState::default().with_selected(Some(selected));
                term.draw(|f| f.render_stateful_widget(list, f.area(), &mut ls))?;
            }};
        }
        redraw!();

        // ── Event loop ───────────────────────────────────────────────────────
        let result = loop {
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
                        redraw!();
                    }

                    (KeyCode::Char('d'), _) | (KeyCode::Delete, _) => {
                        let targets: Vec<usize> = if marked.is_empty() {
                            vec![selected]
                        } else {
                            let mut v: Vec<usize> = marked.iter().copied().collect();
                            v.sort_unstable(); v
                        };
                        let names: Vec<String> = targets.iter()
                            .map(|&i| agents[i].name.clone())
                            .collect();
                        let prompt = if targets.len() == 1 {
                            format!("\n  Delete '{}'? [y/N]: ", names[0])
                        } else {
                            format!("\n  Delete {} agents ({})? [y/N]: ",
                                targets.len(), names.join(", "))
                        };
                        execute!(term.backend_mut(),
                            cursor::Show,
                            SetForegroundColor(Color::Yellow),
                            Print(&prompt), ResetColor)?;
                        term.backend_mut().flush()?;

                        let confirmed = loop {
                            if let Ok(Event::Key(k)) = event::read() {
                                break matches!(k.code, KeyCode::Char('y') | KeyCode::Char('Y'));
                            }
                        };
                        execute!(term.backend_mut(), cursor::Hide, Print("\n"))?;

                        if confirmed {
                            let to_delete: Vec<AgentState> = targets.iter()
                                .map(|&i| agents[i].clone())
                                .collect();
                            break Some(AgentPickerResult::DeleteMany(to_delete));
                        } else {
                            redraw!();
                        }
                    }

                    (KeyCode::Char('r'), _) => {
                        let a = agents[selected].clone();
                        terminal::disable_raw_mode()?;
                        execute!(term.backend_mut(),
                            cursor::Show,
                            SetForegroundColor(Color::Yellow),
                            Print(format!("\n  Rename '{}' → new name: ", a.name)),
                            ResetColor)?;
                        term.backend_mut().flush()?;
                        let mut buf = String::new();
                        std::io::stdin().read_line(&mut buf).unwrap_or(0);
                        let new_name = buf.trim().to_string();
                        terminal::enable_raw_mode()?;
                        execute!(term.backend_mut(), cursor::Hide)?;
                        if new_name.is_empty() { redraw!(); }
                        else { break Some(AgentPickerResult::Rename { agent: a, new_name }); }
                    }

                    (KeyCode::Up, _) | (KeyCode::Char('k'), _) => {
                        selected = if selected == 0 { total - 1 } else { selected - 1 };
                        redraw!();
                    }
                    (KeyCode::Down, _) | (KeyCode::Char('j'), _) => {
                        selected = (selected + 1) % total;
                        redraw!();
                    }
                    _ => {}
                }
            }
        };

        term.clear()?;
        drop(term);
        drop(_raw);
        execute!(stdout, cursor::Show, ResetColor)?;
        Ok(result)
    }

    /// Interactive model picker — rendered via ratatui `Viewport::Inline`.
    /// Returns the selected model string or None if cancelled.
    async fn interactive_model_picker(&self, stdout: &mut io::Stdout) -> Result<Option<String>> {
        use crossterm::event::{self, Event, KeyCode};
        use ratatui::{
            Terminal, TerminalOptions, Viewport,
            backend::CrosstermBackend,
            widgets::{List, ListItem, ListState, Block,
                      Scrollbar, ScrollbarOrientation, ScrollbarState},
            layout::{Constraint, Layout, Direction},
            style::{Style, Color as RC, Modifier},
            text::{Line, Span},
        };

        execute!(stdout, SetForegroundColor(Color::DarkGrey),
            Print("\n  Fetching models…\r\n"), ResetColor)?;

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
                execute!(stdout, SetForegroundColor(Color::DarkGrey),
                    Print("  Could not fetch models from server.\r\n"),
                    Print("  Specify model directly: /model provider/model-name\r\n\n"),
                    ResetColor)?;
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
            execute!(stdout, Print("\n  No models available.\r\n"),
                SetForegroundColor(Color::DarkGrey),
                Print("  Connect a provider: /connect\r\n\n"), ResetColor)?;
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

        // ── Terminal setup ────────────────────────────────────────────────────
        let term_rows = terminal::size().map(|(_, r)| r as usize).unwrap_or(24);
        let view_h    = term_rows.saturating_sub(3).max(5) as u16;

        let _raw = crate::ui::RawModeGuard::enable()?;
        execute!(stdout, cursor::Hide)?;
        let mut term = Terminal::with_options(
            CrosstermBackend::new(&mut *stdout),
            TerminalOptions { viewport: Viewport::Inline(view_h) },
        )?;

        // ── Draw macro ────────────────────────────────────────────────────────
        macro_rules! redraw {
            () => {{
                let sel_model = model_at(list_pos);
                let title = format!(
                    " Models  ↑↓/jk/PgUp/PgDn  Enter select  q cancel  [{}/{}] ",
                    sel_model + 1, n_models
                );
                let items = build_items(list_pos, &current);
                let list = List::new(items)
                    .block(Block::default().title(title));
                let mut ls = ListState::default().with_selected(Some(list_pos));
                let mut sb = ScrollbarState::new(disp_len).position(list_pos);
                term.draw(|f| {
                    let area = f.area();
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
            }};
        }
        redraw!();

        // ── Event loop ────────────────────────────────────────────────────────
        let result = loop {
            if let Ok(Event::Key(key)) = event::read() {
                match (key.code, key.modifiers) {
                    (KeyCode::Esc, _) | (KeyCode::Char('q'), _) => break None,

                    (KeyCode::Enter, _) => {
                        let sel = model_at(list_pos);
                        let (provider, _, id, _, _) = &models[sel];
                        if provider == "__custom__" || id.ends_with('/') {
                            // Text input — write directly to backend
                            terminal::disable_raw_mode()?;
                            let prefix = if id.ends_with('/') && id.len() > 1 {
                                id.as_str()
                            } else { "" };
                            execute!(term.backend_mut(),
                                cursor::Show, Print("\n"),
                                SetForegroundColor(Color::DarkGrey),
                                Print(format!("  Model ID: {prefix}")), ResetColor)?;
                            term.backend_mut().flush()?;
                            let mut input = String::new();
                            std::io::stdin().read_line(&mut input).unwrap_or(0);
                            let typed = input.trim().to_string();
                            if typed.is_empty() { break None; }
                            let full = if prefix.is_empty() || typed.starts_with(prefix) {
                                typed
                            } else { format!("{prefix}{typed}") };
                            break Some(full);
                        } else {
                            break Some(id.clone());
                        }
                    }

                    (KeyCode::Up, _) | (KeyCode::Char('k'), _) => {
                        list_pos = prev_pos(list_pos);
                        redraw!();
                    }
                    (KeyCode::Down, _) | (KeyCode::Char('j'), _) => {
                        list_pos = next_pos(list_pos);
                        redraw!();
                    }
                    (KeyCode::PageDown, _) => {
                        for _ in 0..view_h { list_pos = next_pos(list_pos); }
                        redraw!();
                    }
                    (KeyCode::PageUp, _) => {
                        for _ in 0..view_h { list_pos = prev_pos(list_pos); }
                        redraw!();
                    }
                    _ => {}
                }
            }
        };

        term.clear()?;
        drop(term);
        drop(_raw);
        execute!(stdout, cursor::Show, ResetColor)?;
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
        execute!(stdout, SetForegroundColor(Color::DarkGrey),
            Print(format!("  Launching subagent [{}]{}…\n",
                subagent_type,
                if background { " (background)" } else { "" }
            )),
            ResetColor)?;

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
                execute!(stdout, SetForegroundColor(Color::Green),
                    Print(format!("  ✓ Subagent [{}] complete\n", subagent_type)),
                    ResetColor)?;
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

    fn print_error(&self, stdout: &mut io::Stdout, msg: &str) -> Result<()> {
        execute!(
            stdout,
            SetForegroundColor(Color::Red),
            Print(format!("\nError: {msg}\n")),
            ResetColor
        )?;
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
        s.push_str("    /memory delete <label>     - delete a block\n");
        s.push_str("    /memory edit <label>       - multi-line inline editor\n");
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
