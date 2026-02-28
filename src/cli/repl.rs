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

use crate::agent::{CadeClient, client::{CadeMessage, MemoryBlock}};
use crate::agent::session::SessionStore;
use crate::permissions::{PermissionManager, PermissionMode};
use crate::settings::SettingsManager;
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
        "yolo"                   => Some(SlashCmd::Yolo),
        "plan"                   => Some(SlashCmd::Plan),
        "default" | "normal" | "resume" => Some(SlashCmd::Default),
        "mode"                   => Some(SlashCmd::Mode(arg)),
        "model" if arg.is_some() => Some(SlashCmd::Model(arg.unwrap())),
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
    settings: Arc<Mutex<SettingsManager>>,
    session:  Arc<Mutex<SessionStore>>,
    /// Working directory (for /init context)
    cwd: std::path::PathBuf,
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

        let mut history: Vec<String> = Vec::new();
        let mut hist_idx: Option<usize> = None;

        loop {
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
                        execute!(stdout, SetForegroundColor(Color::DarkGrey),
                            Print(format!("\n  Switching model → {m}…\n")), ResetColor)?;
                        stdout.flush()?;
                        match self.client.patch_agent_model(&self.agent_id(), &m).await {
                            Ok(new_model) => {
                                *self.current_model.lock().unwrap() = new_model.clone();
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
                        let init_prompt = format!(
                            "Please analyse this project directory ('{}') and initialise your memory. \
                             Read relevant files (README.md, Cargo.toml, package.json, pyproject.toml, etc.), \
                             then store key information in memory blocks: \
                             (1) project purpose, (2) tech stack, (3) key conventions, (4) important paths. \
                             Be concise.",
                            self.cwd.display()
                        );
                        self.agent_turn(&mut stdout, &init_prompt).await?;
                    }

                    SlashCmd::Remember(text) => {
                        let id = self.agent_id();
                        // Append to existing 'preferences' block (or create it)
                        let existing = self.client.get_memory(&id).await
                            .unwrap_or_default()
                            .into_iter()
                            .find(|b| b.label == "preferences")
                            .map(|b| b.value)
                            .unwrap_or_default();
                        let ts = chrono::Local::now().format("%Y-%m-%d");
                        let updated = if existing.is_empty() {
                            format!("[{ts}] {text}")
                        } else {
                            format!("{existing}\n[{ts}] {text}")
                        };
                        match self.client.upsert_memory(&id, "preferences", &updated).await {
                            Ok(_) => {
                                execute!(stdout, SetForegroundColor(Color::Green),
                                    Print(format!("\n  ✓ Remembered: {text}\n")), ResetColor)?;
                            }
                            Err(e) => {
                                execute!(stdout, SetForegroundColor(Color::Red),
                                    Print(format!("\n  ✗ {e}\n")), ResetColor)?;
                            }
                        }
                        stdout.flush()?;
                    }

                    SlashCmd::Memory => {
                        match self.client.get_memory(&self.agent_id()).await {
                            Ok(blocks) if blocks.is_empty() => {
                                execute!(stdout, SetForegroundColor(Color::DarkGrey),
                                    Print("\n  (no memory blocks)\n"), ResetColor)?;
                            }
                            Ok(blocks) => {
                                execute!(stdout, Print("\n"))?;
                                for b in &blocks {
                                    let preview: String = b.value.chars().take(300).collect();
                                    let ellipsis = if b.value.len() > 300 { "…" } else { "" };
                                    execute!(stdout,
                                        SetForegroundColor(Color::Cyan),
                                        Print(format!("  [{}]\n", b.label)),
                                        SetForegroundColor(Color::White),
                                        Print(format!("  {}{}\n\n", preview, ellipsis)),
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

            // Send to agent and handle tool loop
            self.agent_turn(&mut stdout, &input).await?;
        }

        Ok(())
    }

    /// Send a user message and drive the tool-call loop with live SSE streaming.
    async fn agent_turn(&self, stdout: &mut io::Stdout, input: &str) -> Result<()> {
        let messages = self.stream_turn(stdout, input, false, "", "").await?;
        self.dispatch_tool_calls(stdout, messages).await
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
    ) -> Result<()> {
        let tool_calls: Vec<(String, String, serde_json::Value)> = messages
            .iter()
            .filter_map(|m| m.as_tool_call())
            .collect();

        for (call_id, tool_name, args) in tool_calls {
            let result = self.execute_tool(stdout, &call_id, &tool_name, &args).await?;

            // Stream the tool return and process any chained tool calls
            let follow = self
                .stream_turn(stdout, "", true, &call_id, &result.output)
                .await?;

            Box::pin(self.dispatch_tool_calls(stdout, follow)).await?;
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

        // Permission check — plan mode allows read operations, blocks write ones
        if self.permissions.is_blocked(tool_name, args) {
            let msg = self.permissions.block_reason(tool_name, args);
            execute!(
                stdout,
                SetForegroundColor(Color::Red),
                Print(format!("  ✗ {msg}\n")),
                ResetColor
            )?;
            return Ok(crate::tools::ToolResult {
                tool_call_id: call_id.to_string(),
                tool_name: tool_name.to_string(),
                output: msg,
                is_error: true,
            });
        }

        if !self.permissions.auto_approve(tool_name) {
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

        // Execute
        execute!(
            stdout,
            SetForegroundColor(Color::DarkGrey),
            Print("  ▶ running…\n"),
            ResetColor
        )?;
        stdout.flush()?;

        let result = dispatch(call_id.to_string(), tool_name, args).await;

        // Show result summary
        if result.is_error {
            execute!(
                stdout,
                SetForegroundColor(Color::Red),
                Print(format!("  ✗ {}\n", truncate(&result.output, 120))),
                ResetColor
            )?;
        } else {
            let lines = result.output.lines().count();
            execute!(
                stdout,
                SetForegroundColor(Color::Green),
                Print(format!("  ✓ {} lines\n", lines)),
                ResetColor
            )?;
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
        stdout.flush()?;

        Ok(approved)
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
        println!("  Memory:");
        println!("    /init           - analyse project + initialise memory");
        println!("    /remember <t>   - store a fact in agent memory");
        println!("    /memory         - view all memory blocks");
        println!("    /search <q>     - search past messages");
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
                                    Print(format!("  {icon} {label}  (Shift+Tab to cycle)\n")),
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
                                execute!(stdout, Print("^C\n"))?;
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
