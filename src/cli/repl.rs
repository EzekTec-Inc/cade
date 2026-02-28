use anyhow::Result;
use crossterm::{
    cursor,
    event::{self, Event, KeyCode, KeyEvent, KeyModifiers},
    execute,
    style::{Attribute, Color, Print, ResetColor, SetAttribute, SetForegroundColor},
    terminal::{self, ClearType},
};
use std::io::{self, Write};

use crate::agent::{CadeClient, client::CadeMessage};
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
    client: CadeClient,
    agent_id: String,
    agent_name: String,
    permissions: PermissionManager,
}

impl Repl {
    pub fn new(
        client: CadeClient,
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

    /// Send a user message and drive the tool-call loop with live SSE streaming.
    async fn agent_turn(&self, stdout: &mut io::Stdout, input: &str) -> Result<()> {
        // Live-stream the response, rendering each event as it arrives
        let messages = self.stream_turn(stdout, input, false, "", "").await?;
        // After the stream ends, execute any tool calls that were collected
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

        let messages = if is_tool_return {
            match self
                .client
                .stream_tool_return(
                    &self.agent_id,
                    tool_call_id,
                    tool_output,
                    false,
                    on_event,
                )
                .await
            {
                Ok(m) => m,
                Err(e) => {
                    self.print_error(stdout, &e.to_string())?;
                    return Ok(vec![]);
                }
            }
        } else {
            match self
                .client
                .stream_message(&self.agent_id, input, on_event)
                .await
            {
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
