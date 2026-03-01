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
    layout::Rect,
    style::{Color as RC, Modifier, Style},
    text::{Line, Span},
    widgets::{Paragraph, Widget, Wrap},
};

/// Horizontal padding (columns) applied to both sides of all rendered content.
pub const CONTENT_PAD: u16 = 4;
/// Left-margin string for direct stdout writes (equals CONTENT_PAD spaces).
const INDENT: &str = "    ";

/// Shrink a buffer area by `pad` columns on each side for consistent margins.
fn padded_rect(area: Rect, pad: u16) -> Rect {
    Rect {
        x:      area.x + pad,
        y:      area.y,
        width:  area.width.saturating_sub(pad * 2),
        height: area.height,
    }
}

/// Estimate the number of terminal rows a set of Lines will occupy when rendered
/// at `width` columns (for `insert_before` height calculation).
fn estimate_height(lines: &[Line], width: usize) -> u16 {
    lines
        .iter()
        .map(|l| {
            let char_len: usize = l.spans.iter().map(|s| s.content.chars().count()).sum();
            ((char_len.max(1) - 1) / width.max(1) + 1) as u16
        })
        .sum::<u16>()
        .max(1)
}

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

    // ── Inline markdown state (assistant streaming only) ──────────────────
    /// Currently inside `**...**` bold span.
    in_bold: bool,
    /// Currently inside `` `...` `` code span.
    in_code: bool,
    /// Pending `*` waiting to see if a second `*` follows (for `**` detection).
    md_pending: Option<char>,
    /// True immediately after a newline — enables bullet / heading detection.
    at_line_start: bool,
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
            in_bold: false,
            in_code: false,
            md_pending: None,
            at_line_start: false,
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
        self.stream_col = CONTENT_PAD;
        let mut out = io::stdout();
        execute!(
            out,
            Print("\n"),
            SetForegroundColor(Color::DarkGrey),
            SetAttribute(Attribute::Italic),
            Print(format!("{INDENT}💭 thinking…\n{INDENT}")),
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
        self.in_bold = false;
        self.in_code = false;
        self.md_pending = None;
        self.at_line_start = false;
        let mut out = io::stdout();
        execute!(out, Print("\n"), SetAttribute(Attribute::Reset), ResetColor)?;
        out.flush()
    }

    /// Write one chunk of assistant text, with soft word-wrap.
    pub fn assistant_chunk(&mut self, text: &str) -> io::Result<()> {
        if !self.in_assistant {
            self.in_assistant = true;
            self.stream_col = CONTENT_PAD;
            self.at_line_start = true;
            let mut out = io::stdout();
            execute!(
                out,
                SetAttribute(Attribute::Reset),
                ResetColor,
                SetForegroundColor(Color::White),
                Print(format!("\n{INDENT}")),
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
        self.in_bold = false;
        self.in_code = false;
        self.md_pending = None;
        self.at_line_start = false;
        let mut out = io::stdout();
        execute!(out, Print("\n"), SetAttribute(Attribute::Reset), ResetColor)?;
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

    /// Tool call — Letta Code-style bullet header + indented arg preview.
    pub fn tool_call(&mut self, name: &str, preview: &str) -> Result<()> {
        self.close_streaming()?;
        self.update_width();
        let inner_w = self.term_width.saturating_sub(CONTENT_PAD * 2 + 2) as usize;
        let preview_trunc = truncate_str(preview, inner_w);

        let lines: Vec<Line> = vec![
            Line::from(vec![
                Span::styled("● ", Style::default().fg(RC::Green).add_modifier(Modifier::BOLD)),
                Span::styled(name.to_string(), Style::default().fg(RC::Green).add_modifier(Modifier::BOLD)),
            ]),
            Line::from(vec![
                Span::raw("  "),
                Span::styled(preview_trunc, Style::default().fg(RC::DarkGray)),
            ]),
        ];
        let height = lines.len() as u16;
        self.with_insert_before(height, move |buf| {
            let area = padded_rect(*buf.area(), CONTENT_PAD);
            Paragraph::new(lines).render(area, buf);
        })
    }

    /// Tool result — `⎿ summary` line in green (success) or red (error).
    pub fn tool_result(&mut self, is_error: bool, summary: &str) -> Result<()> {
        let color = if is_error { RC::Red } else { RC::Green };
        let max = self.term_width.saturating_sub(CONTENT_PAD * 2 + 6) as usize;
        let line = Line::from(vec![
            Span::styled("  ⎿ ", Style::default().fg(RC::DarkGray)),
            Span::styled(truncate_str(summary, max), Style::default().fg(color).add_modifier(Modifier::BOLD)),
        ]);
        self.with_insert_before(1, move |buf| {
            let area = padded_rect(*buf.area(), CONTENT_PAD);
            Paragraph::new(line).render(area, buf);
        })
    }

    /// System / info message — dim gray, word-wrapped.
    pub fn system(&mut self, msg: &str) -> Result<()> {
        self.update_width();
        let wrap_w = self.term_width.saturating_sub(CONTENT_PAD * 2) as usize;
        let lines: Vec<Line> = msg
            .lines()
            .map(|l| Line::from(Span::styled(
                format!("  ℹ {l}"),
                Style::default().fg(RC::DarkGray),
            )))
            .collect();
        let height = estimate_height(&lines, wrap_w);
        self.with_insert_before(height, move |buf| {
            let area = padded_rect(*buf.area(), CONTENT_PAD);
            Paragraph::new(lines)
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
            Print(format!("{INDENT}✗ {msg}\n")),
            ResetColor,
        )?;
        out.flush()?;
        Ok(())
    }

    /// Hook continuation notice.
    pub fn hook_continuation(&mut self, reason: &str) -> Result<()> {
        let line = Line::from(vec![
            Span::styled("  ⎿ ", Style::default().fg(RC::DarkGray)),
            Span::styled(format!("Hook continuing: {reason}"), Style::default().fg(RC::DarkGray)),
        ]);
        self.with_insert_before(1, move |buf| {
            let area = padded_rect(*buf.area(), CONTENT_PAD);
            Paragraph::new(line).render(area, buf);
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
        let wrap_w = self.term_width.saturating_sub(CONTENT_PAD * 2) as usize;
        let mut lines: Vec<Line> = vec![
            Line::from(vec![
                Span::styled("● ", Style::default().fg(RC::Cyan).add_modifier(Modifier::BOLD)),
                Span::styled(format!("[background: {subagent}]"), Style::default().fg(RC::Cyan).add_modifier(Modifier::BOLD)),
            ]),
            Line::from(vec![
                Span::raw("  "),
                Span::styled(format!("Task: {task}"), Style::default().fg(RC::DarkGray)),
            ]),
        ];
        for result_line in result.lines() {
            lines.push(Line::from(vec![
                Span::raw("  "),
                Span::styled(result_line.to_string(), Style::default().fg(RC::White)),
            ]));
        }
        let height = estimate_height(&lines, wrap_w);
        self.with_insert_before(height, move |buf| {
            let area = padded_rect(*buf.area(), CONTENT_PAD);
            Paragraph::new(lines)
                .wrap(Wrap { trim: false })
                .render(area, buf);
        })
    }

    // ── Internals ─────────────────────────────────────────────────────────────

    fn wrap_width(&self) -> usize {
        self.term_width.saturating_sub(CONTENT_PAD * 2) as usize
    }

    /// Write `text` to `out` with soft word-wrapping at `wrap_at` columns,
    /// tracking `self.stream_col`. Applies inline markdown detection when
    /// `self.in_assistant` is true: **bold**, `code`, `- `/`* ` bullets, `# ` headings.
    fn write_wrapped(
        &mut self,
        out: &mut impl Write,
        text: &str,
        wrap_at: usize,
    ) -> io::Result<()> {
        let do_md = self.in_assistant;

        for ch in text.chars() {
            if ch == '\n' {
                // Flush any pending `*` before newline
                if let Some(p) = self.md_pending.take() {
                    execute!(out, Print(p))?;
                    self.stream_col = self.stream_col.saturating_add(1);
                }
                execute!(out, Print(format!("\r\n{INDENT}")))?;
                self.stream_col = CONTENT_PAD;
                self.at_line_start = true;
            } else if ch == '\r' {
                // skip bare CR
            } else {
                // ── Soft-wrap at word boundary ─────────────────────────────
                if self.stream_col >= wrap_at as u16 && ch == ' ' {
                    if let Some(p) = self.md_pending.take() {
                        execute!(out, Print(p))?;
                    }
                    execute!(out, Print(format!("\r\n{INDENT}")))?;
                    self.stream_col = CONTENT_PAD;
                    self.at_line_start = false;
                    continue; // skip the space that triggered the wrap
                }

                if do_md {
                    // ── Heading: `# ` at line start ───────────────────────
                    if self.at_line_start && ch == '#' {
                        self.at_line_start = false;
                        // consume leading hashes + space by switching to heading style
                        execute!(
                            out,
                            SetForegroundColor(Color::Cyan),
                            SetAttribute(Attribute::Bold),
                        )?;
                        continue; // don't print the `#`
                    }
                    // ── Bullet: `- ` or `* ` at line start ───────────────
                    if self.at_line_start && (ch == '-' || ch == '*') {
                        self.md_pending = Some('•');
                        self.at_line_start = false;
                        continue;
                    }
                    if let Some('•') = self.md_pending {
                        if ch == ' ' {
                            // confirmed bullet — emit `• ` in current style
                            self.md_pending = None;
                            execute!(out, Print("• "))?;
                            self.stream_col = self.stream_col.saturating_add(2);
                            continue;
                        } else {
                            // not a bullet — emit the held char literally
                            let held = self.md_pending.take().unwrap();
                            execute!(out, Print(held))?;
                            self.stream_col = self.stream_col.saturating_add(1);
                            // fall through to process `ch` normally
                        }
                    }
                    // ── Backtick code span ─────────────────────────────────
                    if ch == '`' {
                        self.in_code = !self.in_code;
                        if self.in_code {
                            execute!(out, SetForegroundColor(Color::Yellow))?;
                        } else {
                            execute!(out, SetForegroundColor(Color::White))?;
                        }
                        continue; // don't print the backtick
                    }
                    // ── Bold `**...**` ─────────────────────────────────────
                    if ch == '*' {
                        if self.md_pending == Some('*') {
                            // second `*` → toggle bold
                            self.md_pending = None;
                            self.in_bold = !self.in_bold;
                            if self.in_bold {
                                execute!(out, SetAttribute(Attribute::Bold))?;
                            } else {
                                execute!(out, SetAttribute(Attribute::NormalIntensity))?;
                            }
                        } else {
                            self.md_pending = Some('*');
                        }
                        continue;
                    }
                    // If we have a pending `*` and hit a non-`*` char, emit it
                    if let Some('*') = self.md_pending {
                        execute!(out, Print('*'))?;
                        self.stream_col = self.stream_col.saturating_add(1);
                        self.md_pending = None;
                    }
                    // After first non-whitespace on a line, clear line-start flag
                    if self.at_line_start && !ch.is_whitespace() {
                        self.at_line_start = false;
                    }
                }

                execute!(out, Print(ch))?;
                self.stream_col = self.stream_col.saturating_add(1);
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
