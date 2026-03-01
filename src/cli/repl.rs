use anyhow::Result;
use crossterm::{
    cursor,
    event::{self, Event, KeyCode, KeyEvent, KeyModifiers},
    execute,
    style::{Attribute, Color, Print, ResetColor, SetAttribute, SetForegroundColor},
    terminal::{self, ClearType},
};
use std::io::{self, Write};

use std::sync::{Arc, Mutex};

use crate::agent::{CadeClient, client::CadeMessage};
use crate::agent::session::SessionStore;
use crate::permissions::{PermissionManager, PermissionMode};
use crate::settings::SettingsManager;
use crate::skills::Skill;
use crate::subagents::{BackgroundResult, discover_all_subagents, find_subagent};
use crate::toolsets::Toolset;
use crate::tools::{dispatch, is_write_tool};

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

#[derive(Debug)]
enum SlashCmd {
    Help,
    Exit,
    Clear,
    Agent,
    Info,
    Model(String),
    New,
    Pin,
    Agents,
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
    Yolo,
    Plan,
    Default,
    Mode(Option<String>),
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
        "pin"                    => Some(SlashCmd::Pin),
        "agents"                 => Some(SlashCmd::Agents),
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
        "yolo"                   => Some(SlashCmd::Yolo),
        "plan"                   => Some(SlashCmd::Plan),
        "default" | "normal" | "resume" => Some(SlashCmd::Default),
        "mode"                   => Some(SlashCmd::Mode(arg)),
        "model"  => Some(SlashCmd::Model(arg.unwrap_or_default())),
        "toolset" => Some(SlashCmd::Model(format!("__toolset__{}", arg.unwrap_or_default()))),
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
    ) -> Self {
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
        }
    }

    fn agent_id(&self)   -> String { self.agent_id.lock().unwrap().clone() }
    fn agent_name(&self) -> String { self.agent_name.lock().unwrap().clone() }
    fn model(&self)      -> String { self.current_model.lock().unwrap().clone() }

    pub async fn run(self) -> Result<()> {
        let mut stdout = io::stdout();

        execute!(stdout, SetForegroundColor(Color::Cyan), Print(BANNER), ResetColor)?;
        execute!(
            stdout,
            SetForegroundColor(Color::DarkGrey),
            Print(format!(
                " Agent : {} ({})\n Mode  : {}\n\n",
                self.agent_name(), self.agent_id(), self.permissions.mode()
            )),
            ResetColor
        )?;

        // SessionStart hook (non-blocking)
        self.hooks.session_start(&self.agent_id()).await;

        let mut history: Vec<String> = Vec::new();
        let mut hist_idx: Option<usize> = None;

        loop {
            // Check for completed background subagent results
            {
                let mut results = self.background_results.lock().unwrap();
                for r in results.drain(..) {
                    execute!(stdout, Print("\n"), SetForegroundColor(Color::Cyan),
                        Print(format!("  [background subagent: {}]", r.subagent)), ResetColor,
                        Print(format!("\n  Task: {}\n  Result: {}\n", r.prompt_preview, r.result)))?;
                    stdout.flush()?;
                    // Inject result into main agent's conversation
                    let notify = format!(
                        "[Background subagent '{}' completed (task ID: {})]:\n{}",
                        r.subagent, r.task_id, r.result
                    );
                    let _ = self.client.send_message(&self.agent_id(), &notify).await;
                }
            }

            // Prompt — show mode indicator when not in default mode
            let mode_tag = mode_prompt_tag(self.permissions.mode());
            execute!(
                stdout,
                SetForegroundColor(Color::Green),
                Print(format!("\ncade{mode_tag}> ")),
                ResetColor,
            )?;
            stdout.flush()?;

            let input = match self.read_line(&mut history, &mut hist_idx)? {
                Some(s) => s,
                None => break,
            };
            let input = input.trim().to_string();
            if input.is_empty() {
                continue;
            }
            history.push(input.clone());
            hist_idx = None;

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
                            execute!(stdout, SetForegroundColor(Color::White), Print(text), ResetColor)?;
                        }
                        Err(e) => {
                            execute!(stdout, SetForegroundColor(Color::Red),
                                Print(format!("  bash error: {e}\n")), ResetColor)?;
                        }
                    }
                    stdout.flush()?;
                }
                continue;
            }

            // Slash commands
            if let Some(cmd) = parse_slash(&input) {
                match cmd {
                    SlashCmd::Exit => {
                        execute!(stdout, Print("\nBye!\n"))?;
                        break;
                    }
                    // SlashCmd::Clear is handled below (with context clearing)
                    SlashCmd::Help => self.print_help(&mut stdout)?,
                    SlashCmd::Agent => {
                        println!("\nAgent: {} ({})", self.agent_name(), self.agent_id());
                    }
                    SlashCmd::Info => {
                        execute!(stdout, ResetColor)?;
                        stdout.flush()?;
                        println!();
                        println!("  Agent   : {} ({})", self.agent_name(), self.agent_id());
                        println!("  Model   : {}", self.model());
                        println!("  Mode    : {}", self.permissions.mode());
                        println!("  CWD     : {}", self.cwd.display());
                        println!("  Version : {}", env!("CARGO_PKG_VERSION"));
                    }
                    SlashCmd::Yolo => {
                        self.permissions.set_mode(PermissionMode::BypassPermissions);
                        execute!(stdout,
                            SetForegroundColor(Color::Yellow),
                            Print("\n⚡ Permission mode: bypassPermissions — all tools auto-approved\n"),
                            ResetColor,
                        )?;
                        stdout.flush()?;
                    }
                    SlashCmd::Plan => {
                        self.permissions.set_mode(PermissionMode::Plan);
                        execute!(stdout,
                            SetForegroundColor(Color::Cyan),
                            Print("\n📖 Permission mode: plan (read-only) — write/exec tools blocked\n"),
                            Print("   Use /default to resume normal mode\n"),
                            ResetColor,
                        )?;
                        stdout.flush()?;
                    }
                    SlashCmd::Default => {
                        self.permissions.set_mode(PermissionMode::Default);
                        execute!(stdout,
                            SetForegroundColor(Color::Green),
                            Print("\n✅ Permission mode: default — tools require approval\n"),
                            ResetColor,
                        )?;
                        stdout.flush()?;
                    }
                    SlashCmd::Mode(arg) => {
                        match arg.as_deref() {
                            None | Some("") => {
                                // Show current mode
                                let (icon, label, hint) = mode_display(self.permissions.mode());
                                execute!(stdout,
                                    Print(format!("\n{icon} Current mode: {label}  {hint}\n")),
                                )?;
                                stdout.flush()?;
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
                                stdout.flush()?;
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
                        stdout.flush()?;
                        match self.client.patch_agent_model(&self.agent_id(), &m).await {
                            Ok(new_model) => {
                                *self.current_model.lock().unwrap() = new_model.clone();
                                // Auto-switch toolset if model family changed
                                if new_toolset != old_toolset {
                                    *self.current_toolset.lock().unwrap() = new_toolset;
                                    // Re-register + re-attach tools for the new toolset
                                    let agent_id = self.agent_id();
                                    let client = self.client.clone();
                                    tokio::spawn(async move {
                                        use crate::agent::tools::register_cade_tools;
                                        let tools = register_cade_tools(&client, new_toolset)
                                            .await.unwrap_or_default();
                                        let ids: Vec<String> = tools.iter().map(|t| t.id.clone()).collect();
                                        if !ids.is_empty() {
                                            let _ = client.attach_agent_tools(&agent_id, &ids).await;
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
                        stdout.flush()?;
                    }

                    // ── New commands ──────────────────────────────────────────

                    SlashCmd::Clear => {
                        // Clear terminal
                        execute!(stdout, terminal::Clear(ClearType::All), cursor::MoveTo(0, 0))?;
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
                        stdout.flush()?;
                    }

                    SlashCmd::New => {
                        execute!(stdout, SetForegroundColor(Color::DarkGrey),
                            Print("\n  Creating new agent…\n"), ResetColor)?;
                        stdout.flush()?;
                        let model = self.model();
                        let req = crate::agent::client::CreateAgentRequest {
                            name: Some(format!("CADE-{}", chrono::Local::now().format("%Y%m%d-%H%M%S"))),
                            model,
                            description: Some("CADE coding agent".to_string()),
                            memory_blocks: vec![],
                            tool_ids: vec![],
                        };
                        match self.client.create_agent(req).await {
                            Ok(a) => {
                                *self.agent_id.lock().unwrap()   = a.id.clone();
                                *self.agent_name.lock().unwrap() = a.name.clone();
                                if let Ok(mut s) = self.settings.lock() {
                                    let _ = s.set_last_agent(&a.id);
                                }
                                if let Ok(mut s) = self.session.lock() {
                                    let _ = s.set_agent(a.id.clone(), Some(a.name.clone()));
                                }
                                execute!(stdout, SetForegroundColor(Color::Green),
                                    Print(format!("  ✓ New agent: {} ({})\n", a.name, a.id)),
                                    ResetColor)?;
                            }
                            Err(e) => {
                                execute!(stdout, SetForegroundColor(Color::Red),
                                    Print(format!("  ✗ {e}\n")), ResetColor)?;
                            }
                        }
                        stdout.flush()?;
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
                        stdout.flush()?;
                    }

                    SlashCmd::Agents => {
                        execute!(stdout, SetForegroundColor(Color::DarkGrey),
                            Print("\n  Fetching agents…\n"), ResetColor)?;
                        stdout.flush()?;
                        match self.client.list_agents().await {
                            Ok(agents) if agents.is_empty() => {
                                execute!(stdout, Print("  (no agents found)\n"))?;
                            }
                            Ok(agents) => {
                                let current = self.agent_id();
                                for (i, a) in agents.iter().enumerate() {
                                    let marker = if a.id == current { " ←" } else { "" };
                                    execute!(stdout,
                                        SetForegroundColor(if a.id == current { Color::Green } else { Color::White }),
                                        Print(format!("  [{}] {}{}\n      {}\n", i + 1, a.name, marker, a.id)),
                                        ResetColor)?;
                                }
                                execute!(stdout, SetForegroundColor(Color::DarkGrey),
                                    Print("\n  Switch to [number] or Enter to cancel: "), ResetColor)?;
                                stdout.flush()?;
                                // Read input (need cooked mode for this)
                                terminal::enable_raw_mode()?;
                                let choice = {
                                    let mut buf = String::new();
                                    loop {
                                        if let Ok(crossterm::event::Event::Key(k)) = crossterm::event::read() {
                                            match k.code {
                                                KeyCode::Enter => break,
                                                KeyCode::Char(c) => {
                                                    buf.push(c);
                                                    execute!(stdout, Print(c))?;
                                                }
                                                KeyCode::Backspace if !buf.is_empty() => {
                                                    buf.pop();
                                                    execute!(stdout, cursor::MoveLeft(1), Print(" "), cursor::MoveLeft(1))?;
                                                }
                                                _ => {}
                                            }
                                        }
                                    }
                                    buf
                                };
                                terminal::disable_raw_mode()?;
                                execute!(stdout, Print("\n"))?;
                                if let Ok(n) = choice.trim().parse::<usize>() {
                                    if n >= 1 && n <= agents.len() {
                                        let a = &agents[n - 1];
                                        *self.agent_id.lock().unwrap()   = a.id.clone();
                                        *self.agent_name.lock().unwrap() = a.name.clone();
                                        if let Ok(mut s) = self.settings.lock() {
                                            let _ = s.set_last_agent(&a.id);
                                        }
                                        execute!(stdout, SetForegroundColor(Color::Green),
                                            Print(format!("  ✓ Switched to: {} ({})\n", a.name, a.id)),
                                            ResetColor)?;
                                    }
                                }
                            }
                            Err(e) => {
                                execute!(stdout, SetForegroundColor(Color::Red),
                                    Print(format!("  ✗ {e}\n")), ResetColor)?;
                            }
                        }
                        stdout.flush()?;
                    }

                    SlashCmd::Init => {
                        // Spawn an explore subagent to do the analysis — keeps main
                        // agent's context clean, only the summary comes back.
                        execute!(stdout, SetForegroundColor(Color::DarkGrey),
                            Print(format!("  Analysing project at {}…\n", self.cwd.display())),
                            ResetColor)?;
                        stdout.flush()?;

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
                                memory_blocks: vec![],
                                tool_ids: vec![],
                            };
                            match client.create_agent(req).await {
                                Ok(sub) => {
                                    let perm = PermissionManager::default();
                                    let result = run_headless(&client, &sub.id, &explore_prompt, &perm).await;
                                    let _ = client.delete_agent(&sub.id).await;
                                    result.unwrap_or_else(|e| format!("Analysis failed: {e}"))
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
                                            println!();
                                            execute!(stdout, SetForegroundColor(Color::Cyan),
                                                Print(format!("  [{label}]")), ResetColor)?;
                                            if let Some(desc) = &b.description {
                                                if !desc.is_empty() {
                                                    execute!(stdout, SetForegroundColor(Color::DarkGrey),
                                                        Print(format!("  {desc}")), ResetColor)?;
                                                }
                                            }
                                            println!();
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
                                stdout.flush()?;
                                println!("\n  Editing [{label}]");
                                println!("  Current value:\n  ---");
                                for line in current.lines() { println!("  {line}"); }
                                println!("  ---");
                                println!("  Enter new content (empty line = done, .clear = erase):");

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
                                    println!("  (cancelled — no changes)");
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
                                        stdout.flush()?;
                                        println!("\n  (no memory blocks)");
                                        println!("  Run /init to populate, or use update_memory tool");
                                    }
                                    Ok(blocks) => {
                                        execute!(stdout, ResetColor)?;
                                        stdout.flush()?;
                                        println!();
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
                                            println!();
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
                        stdout.flush()?;
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
                        stdout.flush()?;
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
                                stdout.flush()?;
                                if skills.is_empty() {
                                    println!("\n  No skills loaded.");
                                    println!("  Create one: /skills create <name>");
                                    println!("  Skills dirs: .skills/  ~/.cade/skills/  ~/.cade/agents/<id>/skills/");
                                } else {
                                    println!("\n  Skills ({} loaded):\n", skills.len());
                                    for s in skills.iter() {
                                        let cat = s.category.as_deref()
                                            .map(|c| format!("[{}]", c))
                                            .unwrap_or_default();
                                        println!("  {:<10} {:<28} {:<12} {}",
                                            format!("[{}]", s.scope), s.id, cat, s.description);
                                    }
                                    println!();
                                    println!("  Agent uses load_skill(<id>) to load full content on-demand.");
                                }
                            }

                            "create" => {
                                let name_raw = sub_arg.trim().to_string();
                                if name_raw.is_empty() {
                                    println!("\n  Usage: /skills create <name>");
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
                                                        println!("    Edit the file, then run /skills reload to activate it.");
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
                                        stdout.flush()?;
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
                                        println!("  ✓ Skills listing updated (on-demand via load_skill)");

                                        // Notify agent in current conversation
                                        if new_count > 0 {
                                            let list = names.join(", ");
                                            let notify = format!(
                                                "[System: Skills reloaded. Now active: {list}. \
                                                 Use load_skill(id) to load any skill's full content.]"
                                            );
                                            self.agent_turn(&mut stdout, &notify).await?;
                                        }
                                }
                            }

                            other => {
                                println!("\n  Unknown /skills subcommand: '{other}'");
                                println!("  Usage: /skills [list | create <name> | show <id> | reload]");
                            }
                        }
                        stdout.flush()?;
                    }

                    SlashCmd::Subagents => {
                        let all = discover_all_subagents(&self.cwd);
                        execute!(stdout, ResetColor)?;
                        stdout.flush()?;
                        println!("\n  Available subagents ({}):\n", all.len());
                        for def in &all {
                            println!("{}", def.summary());
                        }
                        println!();
                        println!("  Usage: ask the agent to run_subagent(type, task)");
                        println!("  Custom: create .cade/agents/<name>.md in this project");
                        println!("  Global: create ~/.cade/agents/<name>.md");
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
                                println!();
                                println!("  /connect <name>    — add a provider");
                                println!("  /disconnect <name> — remove a provider");
                                let presets = self.client.list_provider_presets().await;
                                if !presets.is_empty() {
                                    println!("\n  OpenAI-compatible presets:");
                                    for p in &presets {
                                        let n = p["name"].as_str().unwrap_or("?");
                                        let u = p["base_url"].as_str().unwrap_or("?");
                                        println!("    /connect {n:<14} — {u}");
                                    }
                                }
                                println!();
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
                            stdout.flush()?;
                        }
                    }

                    SlashCmd::Permissions => {
                        let mode = self.permissions.mode();
                        let allow = self.permissions.allow_rules();
                        let deny  = self.permissions.deny_rules();
                        println!("\n  Mode: {}\n", mode);
                        if allow.is_empty() && deny.is_empty() {
                            execute!(stdout, SetForegroundColor(Color::DarkGrey),
                                Print("  No allow/deny rules active.\n"), ResetColor)?;
                        } else {
                            if !allow.is_empty() {
                                execute!(stdout, SetForegroundColor(Color::Green),
                                    Print(format!("  Allow rules ({}):\n", allow.len())), ResetColor)?;
                                for r in &allow {
                                    println!("    {r}");
                                }
                                println!();
                            }
                            if !deny.is_empty() {
                                execute!(stdout, SetForegroundColor(Color::Red),
                                    Print(format!("  Deny rules ({}):\n", deny.len())), ResetColor)?;
                                for r in &deny {
                                    println!("    {r}");
                                }
                                println!();
                            }
                        }
                        println!("  /approve-always <pattern>  · /deny-always <pattern>");
                        println!("  e.g.  /approve-always Bash(cargo test)");
                        println!("        /deny-always Bash(rm -rf:*)\n");
                    }

                    SlashCmd::ApproveAlways(pattern) => {
                        if pattern.is_empty() {
                            execute!(stdout, SetForegroundColor(Color::DarkGrey),
                                Print("  Usage: /approve-always <pattern>\n"),
                                Print("  e.g.  /approve-always Bash(cargo test)\n"),
                                Print("        /approve-always Read(src/**)\n"),
                                ResetColor)?;
                        } else if let Some(rule) = crate::permissions::PermissionRule::parse(&pattern) {
                            self.permissions.add_allow_rule(rule.clone());
                            execute!(stdout, SetForegroundColor(Color::Green),
                                Print(format!("  ✓ Added allow rule: {rule}\n")), ResetColor)?;
                            // Offer to persist
                            execute!(stdout, SetForegroundColor(Color::DarkGrey),
                                Print("  Save to settings.json? [y/N] "), ResetColor)?;
                            stdout.flush()?;
                            terminal::enable_raw_mode()?;
                            let save = loop {
                                if let Ok(crossterm::event::Event::Key(k)) = crossterm::event::read() {
                                    match k.code {
                                        crossterm::event::KeyCode::Char('y') | crossterm::event::KeyCode::Char('Y') => break true,
                                        _ => break false,
                                    }
                                }
                            };
                            terminal::disable_raw_mode()?;
                            execute!(stdout, Print(if save { "y\n" } else { "N\n" }))?;
                            if save {
                                let mut settings = self.settings.lock().unwrap();
                                match settings.save_allow_rule(&pattern) {
                                    Ok(_) => execute!(stdout, SetForegroundColor(Color::Green),
                                        Print("  ✓ Saved to ~/.cade/settings.json\n"), ResetColor)?,
                                    Err(e) => self.print_error(&mut stdout, &e.to_string())?,
                                }
                            }
                            stdout.flush()?;
                        } else {
                            self.print_error(&mut stdout, "invalid pattern")?;
                        }
                    }

                    SlashCmd::DenyAlways(pattern) => {
                        if pattern.is_empty() {
                            execute!(stdout, SetForegroundColor(Color::DarkGrey),
                                Print("  Usage: /deny-always <pattern>\n"),
                                Print("  e.g.  /deny-always Bash(rm -rf:*)\n"),
                                Print("        /deny-always Bash(git push --force:*)\n"),
                                ResetColor)?;
                        } else if let Some(rule) = crate::permissions::PermissionRule::parse(&pattern) {
                            self.permissions.add_deny_rule(rule.clone());
                            execute!(stdout, SetForegroundColor(Color::Red),
                                Print(format!("  ✓ Added deny rule: {rule}\n")), ResetColor)?;
                            execute!(stdout, SetForegroundColor(Color::DarkGrey),
                                Print("  Save to settings.json? [y/N] "), ResetColor)?;
                            stdout.flush()?;
                            terminal::enable_raw_mode()?;
                            let save = loop {
                                if let Ok(crossterm::event::Event::Key(k)) = crossterm::event::read() {
                                    match k.code {
                                        crossterm::event::KeyCode::Char('y') | crossterm::event::KeyCode::Char('Y') => break true,
                                        _ => break false,
                                    }
                                }
                            };
                            terminal::disable_raw_mode()?;
                            execute!(stdout, Print(if save { "y\n" } else { "N\n" }))?;
                            if save {
                                let mut settings = self.settings.lock().unwrap();
                                match settings.save_deny_rule(&pattern) {
                                    Ok(_) => execute!(stdout, SetForegroundColor(Color::Red),
                                        Print("  ✓ Saved to ~/.cade/settings.json\n"), ResetColor)?,
                                    Err(e) => self.print_error(&mut stdout, &e.to_string())?,
                                }
                            }
                            stdout.flush()?;
                        } else {
                            self.print_error(&mut stdout, "invalid pattern")?;
                        }
                    }

                    SlashCmd::Hooks => {
                        use crate::settings::manager::HookDef;
                        let merged = self.settings.lock().unwrap().merged_hooks();
                        if merged.is_empty() {
                            execute!(stdout, SetForegroundColor(Color::DarkGrey),
                                Print("\n  No hooks configured.\n\n"),
                                Print("  Add hooks to ~/.cade/settings.json or .cade/settings.json:\n"),
                                Print("  {\n    \"hooks\": {\n      \"PreToolUse\": [{ \"matcher\": \"Bash\", \"hooks\": [{ \"type\": \"command\", \"command\": \"./hooks/check.sh\" }] }]\n    }\n  }\n\n"),
                                ResetColor)?;
                        } else {
                            println!("\n  Hooks configuration\n");
                            let show_entries = |name: &str, entries: &[crate::settings::manager::HookEntry]| {
                                if entries.is_empty() { return; }
                                println!("  {} ({} entr{}):", name, entries.len(),
                                    if entries.len() == 1 { "y" } else { "ies" });
                                for entry in entries {
                                    let m = entry.matcher.as_deref().unwrap_or("*");
                                    println!("    matcher: {m}");
                                    for hook in &entry.hooks {
                                        println!("      {hook}");
                                    }
                                }
                                println!();
                            };
                            show_entries("PreToolUse",          &merged.pre_tool_use);
                            show_entries("PostToolUse",         &merged.post_tool_use);
                            show_entries("PostToolUseFailure",  &merged.post_tool_use_failure);
                            show_entries("PermissionRequest",   &merged.permission_request);
                            show_entries("UserPromptSubmit",    &merged.user_prompt_submit);
                            show_entries("Stop",                &merged.stop);
                            show_entries("SubagentStop",        &merged.subagent_stop);
                            show_entries("SessionStart",        &merged.session_start);
                            show_entries("SessionEnd",          &merged.session_end);
                            show_entries("Notification",        &merged.notification);
                            execute!(stdout, SetForegroundColor(Color::DarkGrey),
                                Print("  Config: ~/.cade/settings.json  ·  .cade/settings.json  ·  .cade/settings.local.json\n\n"),
                                ResetColor)?;
                        }
                        stdout.flush()?;
                        let _ = HookDef::Command { command: String::new(), timeout: 0 }; // silence unused import
                    }

                    SlashCmd::Feedback => {
                        execute!(stdout,
                            SetForegroundColor(Color::Cyan),
                            Print("\n  Report issues or give feedback:\n"),
                            SetForegroundColor(Color::White),
                            Print("  https://github.com/EzekTec-Inc/CADE/issues\n"),
                            ResetColor)?;
                        stdout.flush()?;
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
                stdout.flush()?;
                continue;
            }

            // Send to agent and handle tool loop
            self.agent_turn(&mut stdout, &input).await?;
        }

        // SessionEnd hook (non-blocking)
        self.hooks.session_end(&self.agent_id()).await;

        Ok(())
    }

    /// Send a user message and drive the tool-call loop with live SSE streaming.
    async fn agent_turn(&self, stdout: &mut io::Stdout, input: &str) -> Result<()> {
        let messages = self.stream_turn(stdout, input, false, "", "").await?;
        self.dispatch_tool_calls(stdout, messages, input).await
    }

    /// Stream one turn (user message or tool return) and render live.
    /// Returns the complete collected message list.
    async fn stream_turn(
        &self,
        stdout: &mut io::Stdout,
        input: &str,
        is_tool_return: bool,
        tool_call_id: &str,
        tool_output: &str,
    ) -> Result<Vec<CadeMessage>> {
        // Shared mutable render state (needs interior mutability across the closure)
        let stdout_ptr = stdout as *mut io::Stdout;
        let in_reasoning = std::sync::Arc::new(std::sync::Mutex::new(false));
        let in_assistant = std::sync::Arc::new(std::sync::Mutex::new(false));

        let in_reasoning2 = in_reasoning.clone();
        let in_assistant2 = in_assistant.clone();

        let on_event = move |msg: &CadeMessage| {
            // SAFETY: closure is called synchronously within the async function body,
            // stdout outlives the closure, and we never alias it.
            let out = unsafe { &mut *stdout_ptr };
            match msg.msg_type() {
                "reasoning_message" => {
                    let mut flag = in_reasoning2.lock().unwrap();
                    if let Some(text) = msg.reasoning_text() {
                        if !*flag {
                            let _ = execute!(out,
                                SetForegroundColor(Color::DarkGrey),
                                Print("\n  💭 "),
                            );
                            *flag = true;
                        }
                        let _ = execute!(out, Print(text));
                    }
                    let _ = out.flush();
                }
                "assistant_message" => {
                    let mut rflag = in_reasoning2.lock().unwrap();
                    if *rflag {
                        let _ = execute!(out, Print("\n"), ResetColor);
                        *rflag = false;
                    }
                    drop(rflag);

                    let mut aflag = in_assistant2.lock().unwrap();
                    if let Some(text) = msg.assistant_text() {
                        if !text.is_empty() {
                            if !*aflag {
                                let _ = execute!(out,
                                    SetForegroundColor(Color::White),
                                    Print("\n"),
                                );
                                *aflag = true;
                            }
                            let _ = execute!(out, Print(text));
                        }
                    }
                    let _ = out.flush();
                }
                "tool_call_message" => {
                    // Close any open reasoning/assistant block
                    let mut rflag = in_reasoning2.lock().unwrap();
                    if *rflag {
                        let _ = execute!(out, Print("\n"), ResetColor);
                        *rflag = false;
                    }
                    drop(rflag);
                    let mut aflag = in_assistant2.lock().unwrap();
                    if *aflag {
                        let _ = execute!(out, Print("\n"), ResetColor);
                        *aflag = false;
                    }
                }
                _ => {}
            }
        };

        let agent_id = self.agent_id();
        let messages = if is_tool_return {
            match self
                .client
                .stream_tool_return(&agent_id, tool_call_id, tool_output, false, on_event)
                .await
            {
                Ok(m) => m,
                Err(e) => {
                    self.print_error(stdout, &e.to_string())?;
                    return Ok(vec![]);
                }
            }
        } else {
            match self.client.stream_message(&agent_id, input, on_event).await {
                Ok(m) => m,
                Err(e) => {
                    self.print_error(stdout, &e.to_string())?;
                    return Ok(vec![]);
                }
            }
        };

        // Ensure final newline + colour reset after streaming
        execute!(stdout, Print("\n"), ResetColor)?;
        stdout.flush()?;

        Ok(messages)
    }

    /// Collect tool calls from messages and execute them one by one.
    async fn dispatch_tool_calls(
        &self,
        stdout: &mut io::Stdout,
        messages: Vec<CadeMessage>,
        user_input: &str,
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
                execute!(stdout, SetForegroundColor(Color::DarkGrey),
                    Print(format!("\n  ↩ Hook continuing turn: {reason}\n")), ResetColor)?;
                stdout.flush()?;
                // Feed the hook's stderr back to the agent as a new turn
                let follow_msgs = self.stream_turn(stdout, &reason, false, "", "").await?;
                Box::pin(self.dispatch_tool_calls(stdout, follow_msgs, user_input)).await?;
            }
            return Ok(());
        }

        for (call_id, tool_name, args) in tool_calls {
            let result = self.execute_tool(stdout, &call_id, &tool_name, &args).await?;

            // Stream the tool return and process any chained tool calls
            let follow = self
                .stream_turn(stdout, "", true, &call_id, &result.output)
                .await?;

            Box::pin(self.dispatch_tool_calls(stdout, follow, user_input)).await?;
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
        // Print what the agent wants to do
        execute!(
            stdout,
            SetForegroundColor(Color::Yellow),
            SetAttribute(Attribute::Bold),
            Print(format!("\n  🔧 {tool_name}")),
            SetAttribute(Attribute::Reset),
            ResetColor,
        )?;

        // Show compact args preview
        if let Some(cmd) = args.get("command").and_then(|v| v.as_str()) {
            let preview: String = cmd.chars().take(80).collect();
            let ellipsis = if cmd.len() > 80 { "…" } else { "" };
            execute!(
                stdout,
                SetForegroundColor(Color::DarkGrey),
                Print(format!("({preview}{ellipsis})")),
                ResetColor
            )?;
        } else if let Some(path) = args.get("path").and_then(|v| v.as_str()) {
            execute!(
                stdout,
                SetForegroundColor(Color::DarkGrey),
                Print(format!("({path})")),
                ResetColor
            )?;
        }
        execute!(stdout, Print("\n"))?;
        stdout.flush()?;

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

        // Permission check — plan mode / deny rules
        if self.permissions.is_blocked(tool_name, args) {
            let msg = self.permissions.block_reason(tool_name, args);
            execute!(stdout, SetForegroundColor(Color::Red),
                Print(format!("  ✗ {msg}\n")), ResetColor)?;
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
                execute!(stdout, SetForegroundColor(Color::Red),
                    Print(format!("  ✗ Hook denied: {reason}\n")), ResetColor)?;
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
            execute!(stdout, SetForegroundColor(Color::Red),
                Print(format!("  ✗ Hook blocked: {reason}\n")), ResetColor)?;
            return Ok(crate::tools::ToolResult {
                tool_call_id: call_id.to_string(),
                tool_name: tool_name.to_string(),
                output: format!("Blocked by hook: {reason}"),
                is_error: true,
            });
        }

        // Execute
        execute!(stdout, SetForegroundColor(Color::DarkGrey),
            Print("  ▶ running…\n"), ResetColor)?;
        stdout.flush()?;

        let mut result = dispatch(call_id.to_string(), tool_name, args).await;

        // PostToolUse / PostToolUseFailure hooks
        if result.is_error {
            self.hooks.post_tool_use_failure(tool_name, args, &result.output).await;
        } else {
            // PostToolUse may inject additionalContext into the tool output
            if let Some(extra) = self.hooks.post_tool_use(tool_name, args, &result.output).await {
                result.output = format!("{}\n\n[Hook context: {extra}]", result.output);
            }
        }

        // Show result summary
        if result.is_error {
            execute!(stdout, SetForegroundColor(Color::Red),
                Print(format!("  ✗ {}\n", truncate(&result.output, 120))), ResetColor)?;
        } else {
            let lines = result.output.lines().count();
            execute!(stdout, SetForegroundColor(Color::Green),
                Print(format!("  ✓ {} lines\n", lines)), ResetColor)?;
        }
        stdout.flush()?;

        Ok(result)
    }

    /// Prompt the user to approve/deny a tool call. Returns true = approved.
    fn prompt_approval(
        &self,
        stdout: &mut io::Stdout,
        tool_name: &str,
        args: &serde_json::Value,
    ) -> Result<bool> {
        execute!(
            stdout,
            SetForegroundColor(Color::Yellow),
            Print(format!("\n  Allow {tool_name}? [y/N] ")),
            ResetColor
        )?;

        // Show args for write tools
        if is_write_tool(tool_name) {
            if let Some(cmd) = args.get("command").and_then(|v| v.as_str()) {
                execute!(
                    stdout,
                    SetForegroundColor(Color::DarkGrey),
                    Print(format!("\n  > {}\n  Allow? [y/N] ", truncate(cmd, 120))),
                    ResetColor
                )?;
            }
        }
        stdout.flush()?;

        // Read single keypress
        terminal::enable_raw_mode()?;
        let approved = loop {
            if let Ok(Event::Key(KeyEvent { code, .. })) = event::read() {
                match code {
                    KeyCode::Char('y') | KeyCode::Char('Y') => break true,
                    _ => break false,
                }
            }
        };
        terminal::disable_raw_mode()?;
        execute!(stdout, Print(if approved { "y\n" } else { "N\n" }))?;

        // Show /approve-always hint on denial (default mode only, once per tool type)
        if !approved && self.permissions.mode() == crate::permissions::PermissionMode::Default {
            execute!(stdout, SetForegroundColor(Color::DarkGrey),
                Print(format!("  Tip: /approve-always {tool_name} to always allow\n")),
                ResetColor)?;
        }
        stdout.flush()?;
        Ok(approved)
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
        stdout.flush()?;

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
                stdout.flush()?;

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
                stdout.flush()?;
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
            println!();
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
                stdout.flush()?;
                Ok(())
            };

            terminal::enable_raw_mode()?;
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

            terminal::disable_raw_mode()?;
            execute!(stdout, cursor::Show, ResetColor, Print("\r\n"))?;
            stdout.flush()?;

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
            stdout.flush()?;
            terminal::disable_raw_mode()?;
            let key = rpassword_read()?;
            // rpassword_read leaves raw mode off — add newline in normal mode
            execute!(stdout, Print("\r\n"))?;
            stdout.flush()?;
            if key.trim().is_empty() { None } else { Some(key.trim().to_string()) }
        } else {
            None
        };

        // For custom: prompt for base URL
        let base_url = if kind == "openai-compatible" && default_base_url.is_none() {
            execute!(stdout, SetForegroundColor(Color::DarkGrey),
                Print("  Base URL (e.g. https://api.example.com/v1/chat/completions): "),
                ResetColor)?;
            stdout.flush()?;
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
        stdout.flush()?;

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
        stdout.flush()?;
        Ok(())
    }

    /// Interactive model picker — opens on `/model` with no argument.
    /// Returns the selected model string or None if cancelled.
    async fn interactive_model_picker(&self, stdout: &mut io::Stdout) -> Result<Option<String>> {
        use crossterm::event::{self, Event, KeyCode};

        // Fetch available providers from server
        let providers = self.client.available_providers().await;
        let current   = self.model();

        // Supported model catalogue — (provider, display_name, full_id, toolset_label)
        let catalogue: &[(&str, &str, &str, &str)] = &[
            // Anthropic
            ("anthropic", "Claude Opus 4.6",      "anthropic/claude-opus-4-6",              "default"),
            ("anthropic", "Claude Sonnet 4.5",    "anthropic/claude-sonnet-4-5-20250929",   "default"),
            ("anthropic", "Claude Haiku 4.5",     "anthropic/claude-haiku-4-5",             "default"),
            // OpenAI
            ("openai",    "GPT-4.1",              "openai/gpt-4.1",                         "codex"),
            ("openai",    "GPT-4o",               "openai/gpt-4o",                          "codex"),
            ("openai",    "GPT-4o Mini",          "openai/gpt-4o-mini",                     "codex"),
            ("openai",    "o3 Mini",              "openai/o3-mini",                         "codex"),
            // Google
            ("gemini",    "Gemini 2.5 Pro",       "gemini/gemini-2.5-pro",                  "gemini"),
            ("gemini",    "Gemini 2.0 Flash",     "gemini/gemini-2.0-flash",                "gemini"),
            // Ollama
            ("ollama",    "Llama 3",              "ollama/llama3",                          "default"),
            ("ollama",    "Mistral",              "ollama/mistral",                         "default"),
            ("ollama",    "Code Llama",           "ollama/codellama",                       "default"),
        ];

        // Filter to available providers only
        let models: Vec<(&str, &str, &str, &str)> = catalogue.iter()
            .filter(|(p, ..)| providers.contains(&p.to_string()))
            .copied()
            .collect();

        if models.is_empty() {
            execute!(stdout, ResetColor)?;
            stdout.flush()?;
            println!("\n  No models available. Check your API keys.");
            return Ok(None);
        }

        let total = models.len();
        let mut selected: usize = models.iter().position(|(_, _, id, _)| *id == current).unwrap_or(0);

        // Count provider headers once — needed for accurate line counting
        let provider_header_count = {
            let mut seen = std::collections::HashSet::new();
            models.iter().filter(|(p, ..)| seen.insert(*p)).count()
        };
        // Lines drawn per full render:
        // 1 (blank) + 1 (header) + 1 (blank) + provider_headers + total_models + 1 (blank footer)
        let draw_lines = (3 + provider_header_count + total + 1) as u16;

        // Draw the picker — must use \r\n everywhere; we are in raw mode.
        let draw = |stdout: &mut io::Stdout, sel: usize| -> Result<()> {
            execute!(stdout,
                cursor::MoveToColumn(0),
                terminal::Clear(terminal::ClearType::FromCursorDown),
                Print("\r\n"),
                SetForegroundColor(Color::Cyan),  Print("  Select model  "),
                ResetColor,
                SetForegroundColor(Color::DarkGrey),
                Print("↑↓ / j/k navigate  Enter select  Esc cancel\r\n"),
                ResetColor,
                Print("\r\n"),
            )?;

            let mut last_provider = "";
            for (i, (provider, name, id, toolset)) in models.iter().enumerate() {
                if *provider != last_provider {
                    execute!(stdout, SetForegroundColor(Color::Yellow),
                        Print(format!("  {}\r\n", provider.to_uppercase())), ResetColor)?;
                    last_provider = provider;
                }
                let is_current = *id == current;
                let is_sel     = i == sel;
                let arrow      = if is_sel { "  ▶ " } else { "    " };
                let name_color = if is_sel { Color::White } else { Color::DarkGrey };
                execute!(stdout,
                    SetForegroundColor(if is_sel { Color::Green } else { Color::DarkGrey }),
                    Print(arrow),
                    SetForegroundColor(name_color),
                    Print(format!("{:<28}", name)),
                    SetForegroundColor(Color::DarkGrey),
                    Print(format!("[{}]", toolset)),
                    ResetColor,
                )?;
                if is_current {
                    execute!(stdout, SetForegroundColor(Color::Cyan), Print(" ← current"), ResetColor)?;
                }
                execute!(stdout, Print("\r\n"))?;
            }
            execute!(stdout, Print("\r\n"))?;
            stdout.flush()?;
            Ok(())
        };

        // Enter raw mode and draw initial state
        terminal::enable_raw_mode()?;
        execute!(stdout, cursor::Hide)?;
        draw(stdout, selected)?;

        // Event loop
        let result = loop {
            if let Ok(Event::Key(key)) = event::read() {
                match (key.code, key.modifiers) {
                    (KeyCode::Esc, _) | (KeyCode::Char('q'), _) => break None,
                    (KeyCode::Enter, _) => break Some(models[selected].2.to_string()),
                    (KeyCode::Up, _) | (KeyCode::Char('k'), _) => {
                        selected = if selected == 0 { total - 1 } else { selected - 1 };
                        execute!(stdout, cursor::MoveToPreviousLine(draw_lines))?;
                        draw(stdout, selected)?;
                    }
                    (KeyCode::Down, _) | (KeyCode::Char('j'), _) => {
                        selected = (selected + 1) % total;
                        execute!(stdout, cursor::MoveToPreviousLine(draw_lines))?;
                        draw(stdout, selected)?;
                    }
                    _ => {}
                }
            }
        };

        terminal::disable_raw_mode()?;
        execute!(stdout, cursor::Show, ResetColor)?;
        stdout.flush()?;
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
        stdout.flush()?;

        // Clone what we need for the async task
        let client     = self.client.clone();
        let main_model = self.model();
        let permissions = crate::permissions::PermissionManager::default();
        let call_id_owned = call_id.to_string();
        let bg_results = Arc::clone(&self.background_results);

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
                    &client, &sub_agent_id, &prompt, &permissions
                ).await;

                // Delete ephemeral agent
                if ephemeral {
                    let _ = client.delete_agent(&sub_agent_id).await;
                }

                match result {
                    Ok(output) => (output, false),
                    Err(e)     => (format!("Subagent error: {e}"), true),
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
            stdout.flush()?;

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
        stdout.flush()?;
        Ok(())
    }

    fn print_help(&self, stdout: &mut io::Stdout) -> Result<()> {
        let (icon, label, _) = mode_display(self.permissions.mode());
        // Use plain println! to avoid crossterm cursor misalignment from emojis
        // (emojis are double-width in some terminals which confuses crossterm's Print)
        execute!(stdout, ResetColor)?;
        stdout.flush()?;
        println!();
        println!("Commands:");
        println!();
        println!("  Session:");
        println!("    /info           - agent, model, mode, cwd");
        println!("    /agent          - show current agent ID");
        println!("    /agents         - list all agents + switch");
        println!("    /new            - create a fresh agent (hot-swap)");
        println!("    /pin            - pin current agent for quick access");
        println!("    /clear          - clear screen + context window");
        println!();
        println!("  Hooks:");
        println!("    /hooks                    - show active hook configuration");
        println!("    (configure in ~/.cade/settings.json or .cade/settings.json)");
        println!("    events: PreToolUse PostToolUse Stop UserPromptSubmit SessionStart …");
        println!();
        println!("  Permissions:");
        println!("    /permissions              - show mode + active allow/deny rules");
        println!("    /approve-always <pattern> - always allow a tool pattern");
        println!("    /deny-always <pattern>    - always deny a tool pattern");
        println!("    pattern syntax: Bash(cargo test)  Read(src/**)  Bash(rm -rf:*)");
        println!();
        println!("  Providers:");
        println!("    /providers           - list configured providers");
        println!("    /connect             - interactive provider setup");
        println!("    /connect <name>      - connect: anthropic, openai, gemini, openrouter, groq…");
        println!("    /disconnect <name>   - remove a provider (persisted + live)");
        println!();
        println!("  Subagents:");
        println!("    /subagents      - list available subagents (built-in + custom)");
        println!("    ask agent to    - run_subagent(type, task) — spawns subagent");
        println!("    custom def      - .cade/agents/<name>.md in project or ~/.cade/agents/");
        println!();
        println!("  Skills:");
        println!("    /skills                - list loaded skills");
        println!("    /skills create <name>  - scaffold a new SKILL.MD file");
        println!("    /skills show <id>      - show full skill content");
        println!("    /skills reload         - re-discover skills + update agent memory");
        println!();
        println!("  Memory:");
        println!("    /memory                    - list memory blocks (label + description + preview)");
        println!("    /memory view <label>       - show full block content");
        println!("    /memory set <label> <val>  - set a block directly");
        println!("    /memory delete <label>     - delete a block");
        println!("    /memory edit <label>       - multi-line inline editor");
        println!("    /remember [text]           - ask agent to store something in memory");
        println!("    /init                      - analyse project + init memory");
        println!("    /search <q>                - search past messages");
        println!();
        println!("  Model:");
        println!("    /model <m>      - switch model  (e.g. /model gemini/gemini-2.5-pro)");
        println!();
        println!("  Permission modes  (currently: {icon} {label}):");
        println!("    /default        - ask before each tool  [Shift+Tab to cycle]");
        println!("    /plan           - read-only tools; write ops blocked");
        println!("    /yolo           - auto-approve all tools");
        println!("    /mode [name]    - show / set: default | plan | yolo | acceptEdits");
        println!();
        println!("  Direct bash (bypasses agent):");
        println!("    ! <cmd>         - e.g.  ! git status");
        println!();
        println!("    /feedback       - report issues");
        println!("    /help  /?       - this message");
        println!("    exit  /exit     - quit CADE");
        println!();
        stdout.flush()?;
        Ok(())
    }

    // ── Input reading (raw mode readline) ─────────────────────────────────────

    fn read_line(
        &self,
        history: &mut Vec<String>,
        hist_idx: &mut Option<usize>,
    ) -> Result<Option<String>> {
        let mut buf = String::new();
        let mut cursor_pos = 0usize;
        let mut stdout = io::stdout();

        terminal::enable_raw_mode()?;
        let result: Result<Option<String>> = (|| {
            loop {
                if !event::poll(std::time::Duration::from_millis(50))? {
                    continue;
                }
                match event::read()? {
                    Event::Key(KeyEvent { code, modifiers, .. }) => {
                        match (code, modifiers) {
                            (KeyCode::BackTab, _) => {
                                // Shift+Tab: cycle Default → AcceptEdits → Plan → BypassPermissions → Default
                                let next = match self.permissions.mode() {
                                    PermissionMode::Default           => PermissionMode::AcceptEdits,
                                    PermissionMode::AcceptEdits       => PermissionMode::Plan,
                                    PermissionMode::Plan              => PermissionMode::BypassPermissions,
                                    PermissionMode::BypassPermissions => PermissionMode::Default,
                                };
                                self.permissions.set_mode(next);
                                let (icon, label, _) = mode_display(next);
                                // Briefly show mode without breaking the input line
                                execute!(stdout,
                                    Print("\r"),
                                    terminal::Clear(ClearType::CurrentLine),
                                    SetForegroundColor(Color::DarkGrey),
                                    Print(format!("  {icon} {label}  (Shift+Tab to cycle)\r\n")),
                                    ResetColor,
                                    SetForegroundColor(Color::Green),
                                    Print(format!("cade{}> ", mode_prompt_tag(next))),
                                    ResetColor,
                                    Print(&buf),
                                )?;
                                stdout.flush()?;
                                continue;
                            }
                            (KeyCode::Enter, _) => return Ok(Some(buf.clone())),
                            (KeyCode::Char('d'), KeyModifiers::CONTROL) if buf.is_empty() => {
                                return Ok(None);
                            }
                            (KeyCode::Char('c'), KeyModifiers::CONTROL) => {
                                execute!(stdout, Print("^C\r\n"))?;
                                buf.clear();
                                cursor_pos = 0;
                                return Ok(Some(String::new()));
                            }
                            (KeyCode::Char('u'), KeyModifiers::CONTROL) => {
                                // Kill line
                                if cursor_pos > 0 {
                                    execute!(stdout, cursor::MoveLeft(cursor_pos as u16))?;
                                }
                                let clear = " ".repeat(buf.len());
                                execute!(stdout, Print(&clear))?;
                                if !clear.is_empty() {
                                    execute!(stdout, cursor::MoveLeft(clear.len() as u16))?;
                                }
                                buf.clear();
                                cursor_pos = 0;
                            }
                            (KeyCode::Backspace, _) if cursor_pos > 0 => {
                                cursor_pos -= 1;
                                buf.remove(cursor_pos);
                                execute!(stdout, cursor::MoveLeft(1), Print(" "), cursor::MoveLeft(1))?;
                                let rest = buf[cursor_pos..].to_string();
                                execute!(stdout, Print(&rest), Print(" "))?;
                                let back = rest.len() as u16 + 1;
                                execute!(stdout, cursor::MoveLeft(back))?;
                            }
                            (KeyCode::Left, _) if cursor_pos > 0 => {
                                cursor_pos -= 1;
                                execute!(stdout, cursor::MoveLeft(1))?;
                            }
                            (KeyCode::Right, _) if cursor_pos < buf.len() => {
                                cursor_pos += 1;
                                execute!(stdout, cursor::MoveRight(1))?;
                            }
                            (KeyCode::Home, _) | (KeyCode::Char('a'), KeyModifiers::CONTROL) => {
                                if cursor_pos > 0 {
                                    execute!(stdout, cursor::MoveLeft(cursor_pos as u16))?;
                                    cursor_pos = 0;
                                }
                            }
                            (KeyCode::End, _) | (KeyCode::Char('e'), KeyModifiers::CONTROL) => {
                                let dist = buf.len() - cursor_pos;
                                if dist > 0 {
                                    execute!(stdout, cursor::MoveRight(dist as u16))?;
                                    cursor_pos = buf.len();
                                }
                            }
                            (KeyCode::Up, _) if !history.is_empty() => {
                                let new_idx = match *hist_idx {
                                    None => history.len() - 1,
                                    Some(i) if i > 0 => i - 1,
                                    Some(i) => i,
                                };
                                *hist_idx = Some(new_idx);
                                let entry = history[new_idx].clone();
                                self.replace_line_buf(&mut stdout, &buf, &entry, &mut cursor_pos)?;
                                buf = entry;
                            }
                            (KeyCode::Down, _) => {
                                if let Some(i) = *hist_idx {
                                    if i + 1 < history.len() {
                                        *hist_idx = Some(i + 1);
                                        let entry = history[i + 1].clone();
                                        self.replace_line_buf(&mut stdout, &buf, &entry, &mut cursor_pos)?;
                                        buf = entry;
                                    } else {
                                        *hist_idx = None;
                                        self.replace_line_buf(&mut stdout, &buf, "", &mut cursor_pos)?;
                                        buf.clear();
                                    }
                                }
                            }
                            (KeyCode::Char(c), mods)
                                if mods == KeyModifiers::NONE || mods == KeyModifiers::SHIFT =>
                            {
                                buf.insert(cursor_pos, c);
                                cursor_pos += 1;
                                execute!(stdout, Print(c))?;
                                if cursor_pos < buf.len() {
                                    let rest = buf[cursor_pos..].to_string();
                                    execute!(stdout, Print(&rest))?;
                                    execute!(stdout, cursor::MoveLeft(rest.len() as u16))?;
                                }
                            }
                            _ => {}
                        }
                    }
                    _ => {}
                }
                stdout.flush()?;
            }
        })();

        terminal::disable_raw_mode()?;
        result
    }

    fn replace_line_buf(
        &self,
        stdout: &mut io::Stdout,
        old: &str,
        new: &str,
        cursor_pos: &mut usize,
    ) -> Result<()> {
        if *cursor_pos > 0 {
            execute!(stdout, cursor::MoveLeft(*cursor_pos as u16))?;
        }
        let width = old.len().max(new.len()) + 1;
        execute!(stdout, Print(" ".repeat(width)))?;
        execute!(stdout, cursor::MoveLeft(width as u16))?;
        execute!(stdout, Print(new))?;
        *cursor_pos = new.len();
        stdout.flush()?;
        Ok(())
    }
}

fn truncate(s: &str, max: usize) -> String {
    super::truncate(s, max)
}

/// Read a line from stdin with no echo (for API key input).
/// Falls back to normal readline if raw mode can't be set.
/// Read a password from stdin with no echo. Caller must ensure raw mode is OFF before
/// calling; this function enables and then disables raw mode internally.
fn rpassword_read() -> anyhow::Result<String> {
    use crossterm::event::{self, Event, KeyCode};
    let mut buf = String::new();
    terminal::enable_raw_mode()?;
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
    terminal::disable_raw_mode()?; // always leave raw mode OFF on exit
    Ok(buf)
}

fn mode_prompt_tag(mode: PermissionMode) -> &'static str {
    match mode {
        PermissionMode::Plan               => " \x1b[36m[plan]\x1b[0m",
        PermissionMode::BypassPermissions  => " \x1b[33m[yolo]\x1b[0m",
        PermissionMode::AcceptEdits        => " \x1b[35m[edits]\x1b[0m",
        PermissionMode::Default            => "",
    }
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
