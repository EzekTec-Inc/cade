use anyhow::Result;
use crossterm::{
    cursor,
    event::{self, Event, KeyCode, KeyEvent, KeyModifiers},
    execute,
    style::{Color, Print, ResetColor, SetForegroundColor},
    terminal::{self, ClearType},
};
use std::io::{self, Write};

use crate::agent::LettaClient;
use crate::permissions::PermissionManager;

const BANNER: &str = r#"
   ___    _    ____  _____
  / __|  / \  |  _ \| ____|
 | |    / _ \ | | | |  _|
 | |_  / ___ \| |_| | |___
  \__|/_/   \_|____/|_____|

 Coding AI assistant with Desktop Extensions
 Type /help for commands, /exit to quit
"#;

/// Slash commands available in the REPL
#[derive(Debug)]
enum SlashCmd {
    Help,
    Exit,
    Clear,
    Agent,
    Model(String),
    New,
    Info,
}

fn parse_slash(input: &str) -> Option<SlashCmd> {
    let input = input.trim();
    if !input.starts_with('/') {
        return None;
    }
    let parts: Vec<&str> = input[1..].splitn(2, ' ').collect();
    match parts[0] {
        "help" | "?" => Some(SlashCmd::Help),
        "exit" | "quit" | "q" => Some(SlashCmd::Exit),
        "clear" => Some(SlashCmd::Clear),
        "agent" => Some(SlashCmd::Agent),
        "new" => Some(SlashCmd::New),
        "info" => Some(SlashCmd::Info),
        "model" if parts.len() > 1 => Some(SlashCmd::Model(parts[1].to_string())),
        _ => None,
    }
}

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

        // Print banner
        execute!(stdout, SetForegroundColor(Color::Cyan), Print(BANNER), ResetColor)?;
        execute!(
            stdout,
            SetForegroundColor(Color::DarkGrey),
            Print(format!(" Agent: {} ({})\n Permission: {}\n\n",
                self.agent_name, self.agent_id, self.permissions.mode())),
            ResetColor
        )?;

        let mut history: Vec<String> = Vec::new();
        let mut hist_idx: Option<usize> = None;

        loop {
            // Print prompt
            execute!(
                stdout,
                SetForegroundColor(Color::Green),
                Print("\ncade> "),
                ResetColor,
            )?;
            stdout.flush()?;

            // Read input line
            let input = match self.read_line(&mut history, &mut hist_idx)? {
                Some(s) => s,
                None => break, // Ctrl-D / EOF
            };
            let input = input.trim().to_string();
            if input.is_empty() {
                continue;
            }
            history.push(input.clone());
            hist_idx = None;

            // Handle slash commands
            if let Some(cmd) = parse_slash(&input) {
                match cmd {
                    SlashCmd::Exit => {
                        execute!(stdout, Print("\nBye!\n"))?;
                        break;
                    }
                    SlashCmd::Clear => {
                        execute!(stdout, terminal::Clear(ClearType::All), cursor::MoveTo(0, 0))?;
                    }
                    SlashCmd::Help => self.print_help(&mut stdout)?,
                    SlashCmd::Agent => {
                        execute!(stdout, Print(format!("\nAgent: {} ({})\n", self.agent_name, self.agent_id)))?;
                    }
                    SlashCmd::Info => {
                        execute!(stdout, Print(format!(
                            "\nAgent: {} ({})\nPermission mode: {}\n",
                            self.agent_name, self.agent_id, self.permissions.mode()
                        )))?;
                    }
                    SlashCmd::Model(m) => {
                        execute!(stdout, Print(format!("\nModel switching not yet implemented (requested: {m})\n")))?;
                    }
                    SlashCmd::New => {
                        execute!(stdout, Print("\nUse 'cade --new' to create a new agent.\n"))?;
                    }
                }
                continue;
            }

            // Send message to agent
            execute!(stdout, SetForegroundColor(Color::DarkGrey), Print("\n⟳ thinking...\n"), ResetColor)?;
            stdout.flush()?;

            match self.client.send_message(&self.agent_id, &input).await {
                Ok(messages) => {
                    for msg in &messages {
                        self.render_message(&mut stdout, msg)?;
                    }
                }
                Err(e) => {
                    execute!(
                        stdout,
                        SetForegroundColor(Color::Red),
                        Print(format!("\nError: {e}\n")),
                        ResetColor,
                    )?;
                }
            }
        }

        Ok(())
    }

    fn render_message(
        &self,
        stdout: &mut io::Stdout,
        msg: &crate::agent::client::LettaMessage,
    ) -> Result<()> {
        let msg_type = msg.data.get("message_type")
            .and_then(|v| v.as_str())
            .unwrap_or("");

        match msg_type {
            "assistant_message" => {
                if let Some(content) = msg.data.get("content").and_then(|v| v.as_str()) {
                    execute!(
                        stdout,
                        SetForegroundColor(Color::White),
                        Print(format!("\n{content}\n")),
                        ResetColor,
                    )?;
                }
            }
            "reasoning_message" => {
                if let Some(reasoning) = msg.data.get("reasoning").and_then(|v| v.as_str()) {
                    execute!(
                        stdout,
                        SetForegroundColor(Color::DarkGrey),
                        Print(format!("  💭 {reasoning}\n")),
                        ResetColor,
                    )?;
                }
            }
            "tool_call_message" => {
                if let Some(tool_name) = msg.data.get("tool_call")
                    .and_then(|v| v.get("name"))
                    .and_then(|v| v.as_str())
                {
                    execute!(
                        stdout,
                        SetForegroundColor(Color::Yellow),
                        Print(format!("  🔧 {tool_name}(...)\n")),
                        ResetColor,
                    )?;
                }
            }
            _ => {}
        }

        stdout.flush()?;
        Ok(())
    }

    fn print_help(&self, stdout: &mut io::Stdout) -> Result<()> {
        execute!(stdout, SetForegroundColor(Color::Cyan), Print(
            "\nSlash commands:\n\
             /help      — show this help\n\
             /clear     — clear the screen\n\
             /agent     — show current agent ID\n\
             /info      — show session info\n\
             /model <m> — switch model (upcoming)\n\
             /new       — create new agent\n\
             /exit      — exit CADE\n"
        ), ResetColor)?;
        stdout.flush()?;
        Ok(())
    }

    fn read_line(
        &self,
        history: &mut Vec<String>,
        hist_idx: &mut Option<usize>,
    ) -> Result<Option<String>> {
        let mut buf = String::new();
        let mut cursor_pos = 0usize;
        let mut stdout = io::stdout();

        terminal::enable_raw_mode()?;
        let result = (|| -> Result<Option<String>> {
            loop {
                if !event::poll(std::time::Duration::from_millis(100))? {
                    continue;
                }
                match event::read()? {
                    Event::Key(KeyEvent { code, modifiers, .. }) => {
                        match (code, modifiers) {
                            // Submit
                            (KeyCode::Enter, _) => {
                                break Ok(Some(buf.clone()));
                            }
                            // EOF / Ctrl-D
                            (KeyCode::Char('d'), KeyModifiers::CONTROL) if buf.is_empty() => {
                                break Ok(None);
                            }
                            // Ctrl-C
                            (KeyCode::Char('c'), KeyModifiers::CONTROL) => {
                                execute!(stdout, Print("^C\n"))?;
                                buf.clear();
                                cursor_pos = 0;
                                break Ok(Some(String::new()));
                            }
                            // Backspace
                            (KeyCode::Backspace, _) if cursor_pos > 0 => {
                                cursor_pos -= 1;
                                buf.remove(cursor_pos);
                                execute!(
                                    stdout,
                                    cursor::MoveLeft(1),
                                    Print(" "),
                                    cursor::MoveLeft(1)
                                )?;
                                // Reprint rest of line
                                let rest = &buf[cursor_pos..];
                                execute!(stdout, Print(rest), Print(" "))?;
                                let back = rest.len() as u16 + 1;
                                if back > 0 {
                                    execute!(stdout, cursor::MoveLeft(back))?;
                                }
                            }
                            // Up arrow — history
                            (KeyCode::Up, _) => {
                                if history.is_empty() {
                                    continue;
                                }
                                let new_idx = match *hist_idx {
                                    None => history.len() - 1,
                                    Some(i) if i > 0 => i - 1,
                                    Some(i) => i,
                                };
                                *hist_idx = Some(new_idx);
                                let entry = history[new_idx].clone();
                                self.replace_line(&mut stdout, &buf, &entry, &mut cursor_pos)?;
                                buf = entry;
                            }
                            // Down arrow — history
                            (KeyCode::Down, _) => {
                                if let Some(i) = *hist_idx {
                                    if i + 1 < history.len() {
                                        *hist_idx = Some(i + 1);
                                        let entry = history[i + 1].clone();
                                        self.replace_line(&mut stdout, &buf, &entry, &mut cursor_pos)?;
                                        buf = entry;
                                    } else {
                                        *hist_idx = None;
                                        self.replace_line(&mut stdout, &buf, "", &mut cursor_pos)?;
                                        buf.clear();
                                    }
                                }
                            }
                            // Left arrow
                            (KeyCode::Left, _) if cursor_pos > 0 => {
                                cursor_pos -= 1;
                                execute!(stdout, cursor::MoveLeft(1))?;
                            }
                            // Right arrow
                            (KeyCode::Right, _) if cursor_pos < buf.len() => {
                                cursor_pos += 1;
                                execute!(stdout, cursor::MoveRight(1))?;
                            }
                            // Normal character
                            (KeyCode::Char(c), mods)
                                if mods == KeyModifiers::NONE || mods == KeyModifiers::SHIFT =>
                            {
                                buf.insert(cursor_pos, c);
                                cursor_pos += 1;
                                execute!(stdout, Print(c))?;
                                // Reprint rest if inserting mid-line
                                if cursor_pos < buf.len() {
                                    let rest = &buf[cursor_pos..];
                                    execute!(stdout, Print(rest))?;
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

    fn replace_line(
        &self,
        stdout: &mut io::Stdout,
        old: &str,
        new: &str,
        cursor_pos: &mut usize,
    ) -> Result<()> {
        // Move to start of current input
        if *cursor_pos > 0 {
            execute!(stdout, cursor::MoveLeft(*cursor_pos as u16))?;
        }
        // Clear old text
        let clear = " ".repeat(old.len().max(new.len()) + 2);
        execute!(stdout, Print(&clear))?;
        if !clear.is_empty() {
            execute!(stdout, cursor::MoveLeft(clear.len() as u16))?;
        }
        // Print new
        execute!(stdout, Print(new))?;
        *cursor_pos = new.len();
        stdout.flush()?;
        Ok(())
    }
}
