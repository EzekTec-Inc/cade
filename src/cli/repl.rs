use anyhow::Result;
use crossterm::{
    cursor,
    event::{self, Event, KeyCode, KeyEvent, KeyModifiers},
    execute,
    style::{Attribute, Color, Print, ResetColor, SetAttribute, SetForegroundColor},
    terminal::{self, ClearType},
};
use std::io::{self, Write};

use crate::agent::{LettaClient, client::LettaMessage};
use crate::permissions::{PermissionManager, PermissionMode};
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
    Yolo,
    Plan,
}

fn parse_slash(input: &str) -> Option<SlashCmd> {
    let trimmed = input.trim();
    if !trimmed.starts_with('/') {
        return None;
    }
    let parts: Vec<&str> = trimmed[1..].splitn(2, ' ').collect();
    match parts[0] {
        "help" | "?" => Some(SlashCmd::Help),
        "exit" | "quit" | "q" => Some(SlashCmd::Exit),
        "clear" => Some(SlashCmd::Clear),
        "agent" => Some(SlashCmd::Agent),
        "info" => Some(SlashCmd::Info),
        "new" => Some(SlashCmd::New),
        "yolo" => Some(SlashCmd::Yolo),
        "plan" => Some(SlashCmd::Plan),
        "model" if parts.len() > 1 => Some(SlashCmd::Model(parts[1].to_string())),
        _ => None,
    }
}

// ── Repl ──────────────────────────────────────────────────────────────────────

pub struct Repl {
    client: LettaClient,
    agent_id: String,
    agent_name: String,
    permissions: PermissionManager,
}

impl Repl {
    pub fn new(
        client: LettaClient,
        agent_id: String,
        agent_name: String,
        permissions: PermissionManager,
    ) -> Self {
        Self { client, agent_id, agent_name, permissions }
    }

    pub async fn run(self) -> Result<()> {
        let mut stdout = io::stdout();

        execute!(stdout, SetForegroundColor(Color::Cyan), Print(BANNER), ResetColor)?;
        execute!(
            stdout,
            SetForegroundColor(Color::DarkGrey),
            Print(format!(
                " Agent : {} ({})\n Mode  : {}\n\n",
                self.agent_name, self.agent_id, self.permissions.mode()
            )),
            ResetColor
        )?;

        let mut history: Vec<String> = Vec::new();
        let mut hist_idx: Option<usize> = None;

        loop {
            // Prompt
            execute!(
                stdout,
                SetForegroundColor(Color::Green),
                Print("\ncade> "),
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

            // Slash commands
            if let Some(cmd) = parse_slash(&input) {
                match cmd {
                    SlashCmd::Exit => {
                        execute!(stdout, Print("\nBye!\n"))?;
                        break;
                    }
                    SlashCmd::Clear => {
                        execute!(
                            stdout,
                            terminal::Clear(ClearType::All),
                            cursor::MoveTo(0, 0)
                        )?;
                    }
                    SlashCmd::Help => self.print_help(&mut stdout)?,
                    SlashCmd::Agent => {
                        println!("\nAgent: {} ({})", self.agent_name, self.agent_id);
                    }
                    SlashCmd::Info => {
                        println!(
                            "\nAgent : {} ({})\nMode  : {}\nVersion: {}",
                            self.agent_name,
                            self.agent_id,
                            self.permissions.mode(),
                            env!("CARGO_PKG_VERSION")
                        );
                    }
                    SlashCmd::Yolo => {
                        self.permissions.set_mode(PermissionMode::BypassPermissions);
                        println!("\n⚡ Permission mode: bypassPermissions (--yolo)");
                    }
                    SlashCmd::Plan => {
                        self.permissions.set_mode(PermissionMode::Plan);
                        println!("\n📖 Permission mode: plan (read-only)");
                    }
                    SlashCmd::New => {
                        println!("\nUse 'cade --new' to start a fresh agent session.");
                    }
                    SlashCmd::Model(m) => {
                        println!("\n/model switching not yet implemented (requested: {m})");
                    }
                }
                continue;
            }

            // Send to agent and handle tool loop
            self.agent_turn(&mut stdout, &input).await?;
        }

        Ok(())
    }

    /// Send a user message and drive the tool-call loop until the agent sends
    /// a final assistant_message (or stop_reason).
    async fn agent_turn(&self, stdout: &mut io::Stdout, input: &str) -> Result<()> {
        execute!(
            stdout,
            SetForegroundColor(Color::DarkGrey),
            Print("\n⟳ thinking…\n"),
            ResetColor
        )?;
        stdout.flush()?;

        // Initial send
        let messages = match self.client.send_message(&self.agent_id, input).await {
            Ok(m) => m,
            Err(e) => {
                self.print_error(stdout, &e.to_string())?;
                return Ok(());
            }
        };

        self.handle_messages(stdout, messages).await
    }

    /// Recursively process messages, executing tool calls and sending results back.
    async fn handle_messages(
        &self,
        stdout: &mut io::Stdout,
        messages: Vec<LettaMessage>,
    ) -> Result<()> {
        let mut pending_tool_calls: Vec<(String, String, serde_json::Value)> = Vec::new();

        for msg in &messages {
            match msg.msg_type() {
                "assistant_message" => {
                    if let Some(text) = msg.assistant_text() {
                        if !text.is_empty() {
                            execute!(
                                stdout,
                                SetForegroundColor(Color::White),
                                Print(format!("\n{text}\n")),
                                ResetColor
                            )?;
                        }
                    }
                }
                "reasoning_message" => {
                    if let Some(r) = msg.reasoning_text() {
                        execute!(
                            stdout,
                            SetForegroundColor(Color::DarkGrey),
                            Print(format!("  💭 {r}\n")),
                            ResetColor
                        )?;
                    }
                }
                "tool_call_message" => {
                    if let Some(tc) = msg.as_tool_call() {
                        pending_tool_calls.push(tc);
                    }
                }
                _ => {}
            }
        }

        stdout.flush()?;

        // Execute all pending tool calls
        for (call_id, tool_name, args) in pending_tool_calls {
            let result = self
                .execute_tool(stdout, &call_id, &tool_name, &args)
                .await?;

            // Send result back to agent
            let follow_up = match self
                .client
                .send_tool_return(
                    &self.agent_id,
                    &call_id,
                    &result.output,
                    result.is_error,
                )
                .await
            {
                Ok(m) => m,
                Err(e) => {
                    self.print_error(stdout, &format!("tool return failed: {e}"))?;
                    return Ok(());
                }
            };

            // Recursively handle follow-up messages (agent may chain more tools)
            Box::pin(self.handle_messages(stdout, follow_up)).await?;
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

        // Permission check
        if self.permissions.is_blocked(tool_name) {
            let msg = format!("Tool '{tool_name}' blocked (plan mode)");
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
        execute!(
            stdout,
            SetForegroundColor(Color::Cyan),
            Print(concat!(
                "\nSlash commands:\n",
                "  /help        — this message\n",
                "  /agent       — show current agent ID\n",
                "  /info        — show session info\n",
                "  /clear       — clear the screen\n",
                "  /yolo        — disable all permission prompts\n",
                "  /plan        — read-only mode (block write/exec tools)\n",
                "  /model <m>   — switch model (upcoming)\n",
                "  /new         — create new agent session\n",
                "  /exit        — quit CADE\n"
            )),
            ResetColor
        )?;
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
