/// Streaming and bounded output renderer for the CADE REPL.
///
/// Two rendering paths:
/// 1. **Streaming** (reasoning/assistant chunks): direct stdout writes with
///    terminal-width-aware word wrapping. Fast, smooth, no flicker.
/// 2. **Bounded** (tool calls, system msgs, headers): ratatui `insert_before`
///    with styled boxes. The closure receives `&mut Buffer`; widgets are
///    rendered via `Widget::render(area, buf)`.

use std::io::{self, Write};

use anyhow::Result;
use crossterm::{
    execute,
    style::{Attribute, Color, Print, ResetColor, SetAttribute, SetForegroundColor},
};
use ratatui::{
    Terminal, TerminalOptions, Viewport,
    backend::CrosstermBackend,
    buffer::Buffer,
    style::{Color as RC, Modifier, Style},
    text::Span,
    widgets::{Block, Borders, Paragraph, Widget, Wrap},
};

// ── OutputRenderer ─────────────────────────────────────────────────────────────

/// Renders all CADE output: streaming text, tool boxes, system messages, etc.
pub struct OutputRenderer {
    /// Terminal column width (refreshed on resize).
    pub term_width: u16,
    /// Are we currently mid-stream in a reasoning block?
    pub in_reasoning: bool,
    /// Are we currently mid-stream in an assistant block?
    pub in_assistant: bool,
    /// Current column position while streaming (for word-wrap tracking).
    pub stream_col: u16,
}

impl OutputRenderer {
    pub fn new() -> Self {
        let term_width = crossterm::terminal::size()
            .map(|(w, _)| w)
            .unwrap_or(80);
        Self {
            term_width,
            in_reasoning: false,
            in_assistant: false,
            stream_col: 0,
        }
    }

    /// Refresh terminal width (call on resize events).
    pub fn update_width(&mut self) {
        if let Ok((w, _)) = crossterm::terminal::size() {
            self.term_width = w;
        }
    }

    // ── Streaming paths (direct stdout, no ratatui overhead) ─────────────────

    /// Start of a reasoning block — print styled header line.
    pub fn reasoning_header(&mut self) -> io::Result<()> {
        self.in_reasoning = true;
        self.stream_col = 2;
        let mut out = io::stdout();
        execute!(
            out,
            Print("\n"),
            SetForegroundColor(Color::DarkGrey),
            SetAttribute(Attribute::Italic),
            Print("  💭 thinking…\n  "),
        )?;
        out.flush()
    }

    /// Write one chunk of reasoning text, with soft word-wrap.
    pub fn reasoning_chunk(&mut self, text: &str) -> io::Result<()> {
        let wrap_at = self.wrap_width();
        let mut out = io::stdout();
        self.write_wrapped(&mut out, text, wrap_at)?;
        out.flush()
    }

    /// Close the reasoning block — reset style.
    pub fn reasoning_done(&mut self) -> io::Result<()> {
        if !self.in_reasoning {
            return Ok(());
        }
        self.in_reasoning = false;
        self.stream_col = 0;
        let mut out = io::stdout();
        execute!(out, Print("\n"), SetAttribute(Attribute::Reset), ResetColor)?;
        out.flush()
    }

    /// Write one chunk of assistant text, with soft word-wrap.
    pub fn assistant_chunk(&mut self, text: &str) -> io::Result<()> {
        if !self.in_assistant {
            self.in_assistant = true;
            self.stream_col = 2;
            let mut out = io::stdout();
            execute!(
                out,
                SetAttribute(Attribute::Reset),
                ResetColor,
                SetForegroundColor(Color::White),
                Print("\n  "),
            )?;
        }
        let wrap_at = self.wrap_width();
        let mut out = io::stdout();
        self.write_wrapped(&mut out, text, wrap_at)?;
        out.flush()
    }

    /// Close the assistant block — reset and trailing newline.
    pub fn assistant_done(&mut self) -> io::Result<()> {
        if !self.in_assistant {
            return Ok(());
        }
        self.in_assistant = false;
        self.stream_col = 0;
        let mut out = io::stdout();
        execute!(out, Print("\n"), ResetColor)?;
        out.flush()
    }

    /// Close any open streaming block (call before bounded content).
    pub fn close_streaming(&mut self) -> io::Result<()> {
        if self.in_reasoning {
            self.reasoning_done()?;
        }
        if self.in_assistant {
            self.assistant_done()?;
        }
        Ok(())
    }

    // ── Bounded paths (ratatui insert_before) ─────────────────────────────────

    /// Tool call box — yellow bordered, shows tool name + preview line.
    pub fn tool_call(&mut self, name: &str, preview: &str) -> Result<()> {
        self.close_streaming()?;
        self.update_width();
        // Truncate preview to fit inside the box (width - 4 for borders + padding)
        let inner_w = self.term_width.saturating_sub(4) as usize;
        let preview_trunc = truncate_str(preview, inner_w);

        // Content height: 1 line for the preview inside the box
        let box_height: u16 = 3; // top-border + content + bottom-border
        let title = format!(" 🔧 {name} ");
        let title2 = title.clone();
        let preview2 = preview_trunc.clone();

        self.with_insert_before(box_height, move |buf| {
            let area = *buf.area();
            let block = Block::default()
                .borders(Borders::ALL)
                .border_style(Style::default().fg(RC::Yellow))
                .title(Span::styled(
                    title2,
                    Style::default().fg(RC::Yellow).add_modifier(Modifier::BOLD),
                ));
            let inner = block.inner(area);
            block.render(area, buf);
            Paragraph::new(preview2.as_str())
                .style(Style::default().fg(RC::DarkGray))
                .render(inner, buf);
        })
    }

    /// Tool result — green ✓ or red ✗ line pushed above.
    pub fn tool_result(&mut self, is_error: bool, summary: &str) -> Result<()> {
        let (icon, color) = if is_error { ("✗", RC::Red) } else { ("✓", RC::Green) };
        let max = self.term_width.saturating_sub(8) as usize;
        let text = format!("  {icon} {}", truncate_str(summary, max));

        self.with_insert_before(1, move |buf| {
            let area = *buf.area();
            Paragraph::new(Span::styled(text, Style::default().fg(color).add_modifier(Modifier::BOLD)))
                .render(area, buf);
        })
    }

    /// System / info message — dim gray, word-wrapped.
    pub fn system(&mut self, msg: &str) -> Result<()> {
        self.update_width();
        let tw = self.term_width;
        let wrap_w = tw.saturating_sub(4) as usize;
        // Estimate height needed
        let estimated_height = (msg.len() / wrap_w.max(1) + 1) as u16;
        let text = format!("  ℹ {msg}");

        self.with_insert_before(estimated_height.max(1), move |buf| {
            let area = *buf.area();
            Paragraph::new(text.as_str())
                .style(Style::default().fg(RC::DarkGray))
                .wrap(Wrap { trim: false })
                .render(area, buf);
        })
    }

    /// Error message — red, direct stdout write.
    pub fn error(&mut self, msg: &str) -> Result<()> {
        self.close_streaming()?;
        let mut out = io::stdout();
        execute!(
            out,
            Print("\n"),
            SetForegroundColor(Color::Red),
            Print(format!("  ✗ {msg}\n")),
            ResetColor,
        )?;
        out.flush()?;
        Ok(())
    }

    /// Hook continuation notice.
    pub fn hook_continuation(&mut self, reason: &str) -> Result<()> {
        let text = format!("  ↩ Hook continuing turn: {reason}");
        self.with_insert_before(1, move |buf| {
            let area = *buf.area();
            Paragraph::new(Span::styled(text, Style::default().fg(RC::DarkGray)))
                .render(area, buf);
        })
    }

    /// Print the CADE banner (crossterm, one-shot).
    pub fn banner(
        &mut self,
        banner: &str,
        agent_name: &str,
        agent_id: &str,
        model: &str,
        mode: &str,
    ) -> io::Result<()> {
        let mut out = io::stdout();
        execute!(
            out,
            SetForegroundColor(Color::Cyan),
            Print(banner),
            ResetColor,
            SetForegroundColor(Color::DarkGrey),
            Print(format!(
                " Agent : {} ({})\n Model : {}\n Mode  : {}\n\n",
                agent_name, agent_id, model, mode
            )),
            ResetColor,
        )?;
        out.flush()
    }

    /// Display a completed background subagent result.
    pub fn background_result(
        &mut self,
        subagent: &str,
        task: &str,
        result: &str,
    ) -> Result<()> {
        self.update_width();
        let title = format!(" [background: {subagent}] ");
        let body = format!("Task: {task}\nResult: {result}");
        let lines = (body.lines().count() as u16 + 2).max(3);

        self.with_insert_before(lines, move |buf| {
            let area = *buf.area();
            let block = Block::default()
                .borders(Borders::ALL)
                .border_style(Style::default().fg(RC::Cyan))
                .title(Span::styled(title, Style::default().fg(RC::Cyan)));
            let inner = block.inner(area);
            block.render(area, buf);
            Paragraph::new(body.as_str())
                .style(Style::default().fg(RC::White))
                .wrap(Wrap { trim: false })
                .render(inner, buf);
        })
    }

    // ── Internals ─────────────────────────────────────────────────────────────

    fn wrap_width(&self) -> usize {
        self.term_width.saturating_sub(4) as usize
    }

    /// Write `text` to `out` with soft word-wrapping at `wrap_at` columns,
    /// tracking `self.stream_col`.
    fn write_wrapped(
        &mut self,
        out: &mut impl Write,
        text: &str,
        wrap_at: usize,
    ) -> io::Result<()> {
        for ch in text.chars() {
            if ch == '\n' {
                execute!(out, Print("\r\n  "))?;
                self.stream_col = 2;
            } else if ch == '\r' {
                // skip bare CR
            } else {
                // Soft-wrap at a word boundary (space) when approaching the edge
                if self.stream_col >= wrap_at as u16 && ch == ' ' {
                    execute!(out, Print("\r\n  "))?;
                    self.stream_col = 2;
                    // skip the space that caused the wrap
                } else {
                    execute!(out, Print(ch))?;
                    self.stream_col = self.stream_col.saturating_add(1);
                }
            }
        }
        Ok(())
    }

    /// Create a minimal Viewport::Inline(0) terminal, call `insert_before(height, f)`,
    /// then immediately drop the terminal.
    fn with_insert_before<F>(&self, height: u16, f: F) -> Result<()>
    where
        F: FnOnce(&mut Buffer),
    {
        let backend = CrosstermBackend::new(io::stdout());
        let mut term = Terminal::with_options(
            backend,
            TerminalOptions {
                viewport: Viewport::Inline(0),
            },
        )?;
        term.insert_before(height, f)?;
        Ok(())
    }
}

impl Default for OutputRenderer {
    fn default() -> Self {
        Self::new()
    }
}

// ── Helpers ───────────────────────────────────────────────────────────────────

fn truncate_str(s: &str, max_chars: usize) -> String {
    let s = s.trim();
    if s.chars().count() <= max_chars {
        s.to_string()
    } else {
        format!("{}…", s.chars().take(max_chars.saturating_sub(1)).collect::<String>())
    }
}
