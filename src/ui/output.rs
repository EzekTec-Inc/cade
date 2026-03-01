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
    /// Currently inside single-backtick `` `code` `` span.
    in_code: bool,
    /// Pending `*` or sentinel char for multi-char pattern detection.
    md_pending: Option<char>,
    /// True immediately after a newline — enables bullet / heading / list detection.
    at_line_start: bool,

    // ── Code fence state ──────────────────────────────────────────────────
    /// Count of consecutive `` ` `` chars seen (for triple-backtick detection).
    backtick_run: u8,
    /// Currently inside a ` ``` ` fenced code block.
    in_code_fence: bool,
    /// Accumulates the language label after the opening ` ``` `.
    fence_lang_buf: String,
    /// True once the language line's trailing `\n` has been consumed.
    fence_lang_done: bool,

    // ── Numbered list state ───────────────────────────────────────────────
    /// Accumulates ASCII digit chars at line-start (for `1. ` detection).
    num_buf: String,
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
            backtick_run: 0,
            in_code_fence: false,
            fence_lang_buf: String::new(),
            fence_lang_done: false,
            num_buf: String::new(),
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
        self.backtick_run = 0;
        self.in_code_fence = false;
        self.fence_lang_buf.clear();
        self.fence_lang_done = false;
        self.num_buf.clear();
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
        self.backtick_run = 0;
        self.in_code_fence = false;
        self.fence_lang_buf.clear();
        self.fence_lang_done = false;
        self.num_buf.clear();
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

    /// Edit tool call — shows bullet header + file path + styled diff (old/new lines).
    /// Called instead of `tool_call()` for `edit_file` / `apply_patch`.
    pub fn tool_edit_call(
        &mut self,
        tool_name: &str,
        file_path: &str,
        old_str: &str,
        new_str: &str,
    ) -> Result<()> {
        self.close_streaming()?;
        self.update_width();
        let inner_w = self.term_width.saturating_sub(CONTENT_PAD * 2 + 6) as usize;

        // Find start line number by reading the file
        let start_line: usize = std::fs::read_to_string(file_path)
            .ok()
            .and_then(|content| {
                // Find byte offset of old_str in file, then count preceding newlines
                content.find(old_str).map(|byte_off| {
                    content[..byte_off].lines().count() + 1
                })
            })
            .unwrap_or(1);

        let old_lines: Vec<&str> = old_str.lines().collect();
        let new_lines: Vec<&str> = new_str.lines().collect();
        const MAX_DIFF_LINES: usize = 4;
        let show_old = old_lines.len().min(MAX_DIFF_LINES);
        let show_new = new_lines.len().min(MAX_DIFF_LINES);
        let context_n = show_old.max(show_new);

        let mut lines: Vec<Line> = vec![
            // ● edit_file (header)
            Line::from(vec![
                Span::styled("● ", Style::default().fg(RC::Green).add_modifier(Modifier::BOLD)),
                Span::styled(tool_name.to_string(), Style::default().fg(RC::Green).add_modifier(Modifier::BOLD)),
            ]),
            // file path
            Line::from(vec![
                Span::raw("  "),
                Span::styled(file_path.to_string(), Style::default().fg(RC::DarkGray)),
            ]),
            // Showing ~N context line(s)
            Line::from(Span::styled(
                format!("  Showing ~{context_n} context line(s)"),
                Style::default().fg(RC::DarkGray).add_modifier(Modifier::DIM),
            )),
        ];

        // Old lines (red -)
        for (i, old_l) in old_lines[..show_old].iter().enumerate() {
            let ln = start_line + i;
            lines.push(Line::from(vec![
                Span::styled(
                    format!("  {ln:>3} "),
                    Style::default().fg(RC::DarkGray),
                ),
                Span::styled("- ", Style::default().fg(RC::Red).add_modifier(Modifier::BOLD)),
                Span::styled(truncate_str(old_l, inner_w), Style::default().fg(RC::Red)),
            ]));
        }
        if old_lines.len() > MAX_DIFF_LINES {
            lines.push(Line::from(Span::styled(
                format!("      … ({} more old lines)", old_lines.len() - MAX_DIFF_LINES),
                Style::default().fg(RC::DarkGray),
            )));
        }

        // New lines (green +)
        for (i, new_l) in new_lines[..show_new].iter().enumerate() {
            let ln = start_line + i;
            lines.push(Line::from(vec![
                Span::styled(
                    format!("  {ln:>3} "),
                    Style::default().fg(RC::DarkGray),
                ),
                Span::styled("+ ", Style::default().fg(RC::Green).add_modifier(Modifier::BOLD)),
                Span::styled(truncate_str(new_l, inner_w), Style::default().fg(RC::Green)),
            ]));
        }
        if new_lines.len() > MAX_DIFF_LINES {
            lines.push(Line::from(Span::styled(
                format!("      … ({} more new lines)", new_lines.len() - MAX_DIFF_LINES),
                Style::default().fg(RC::DarkGray),
            )));
        }

        let height = lines.len() as u16;
        self.with_insert_before(height, move |buf| {
            let area = padded_rect(*buf.area(), CONTENT_PAD);
            Paragraph::new(lines).render(area, buf);
        })
    }

    /// Bash result — `⎿ N lines` header + first 5 lines of stdout preview.
    pub fn tool_bash_result(&mut self, output: &str) -> Result<()> {
        self.update_width();
        let inner_w = self.term_width.saturating_sub(CONTENT_PAD * 2 + 4) as usize;
        let all_lines: Vec<&str> = output.lines().collect();
        let count = all_lines.len();
        let preview_n = count.min(5);

        let mut lines: Vec<Line> = vec![
            Line::from(vec![
                Span::styled("  ⎿ ", Style::default().fg(RC::DarkGray)),
                Span::styled(
                    if count == 0 { "(no output)".to_string() } else { format!("{count} lines") },
                    if count == 0 {
                        Style::default().fg(RC::DarkGray)
                    } else {
                        Style::default().fg(RC::Green).add_modifier(Modifier::BOLD)
                    },
                ),
            ]),
        ];

        for line_str in &all_lines[..preview_n] {
            lines.push(Line::from(vec![
                Span::raw("    "),
                Span::styled(truncate_str(line_str, inner_w), Style::default().fg(RC::DarkGray)),
            ]));
        }
        if count > preview_n {
            lines.push(Line::from(Span::styled(
                format!("    … ({} more lines)", count - preview_n),
                Style::default().fg(RC::DarkGray),
            )));
        }

        let height = lines.len() as u16;
        self.with_insert_before(height, move |buf| {
            let area = padded_rect(*buf.area(), CONTENT_PAD);
            Paragraph::new(lines).render(area, buf);
        })
    }

    // ── Internals ─────────────────────────────────────────────────────────────

    fn wrap_width(&self) -> usize {
        self.term_width.saturating_sub(CONTENT_PAD * 2) as usize
    }

    /// Write `text` to `out` with soft word-wrapping at `wrap_at` columns.
    /// When `self.in_assistant`, applies inline markdown:
    ///   ``` fences, `inline code`, **bold**, - bullets, # headings, 1. lists
    fn write_wrapped(
        &mut self,
        out: &mut impl Write,
        text: &str,
        wrap_at: usize,
    ) -> io::Result<()> {
        let do_md = self.in_assistant;

        for ch in text.chars() {
            // ── Newline ────────────────────────────────────────────────────
            if ch == '\n' {
                // ── Inside a code fence: handle lang line or normal newline ─
                if do_md && self.in_code_fence && !self.fence_lang_done {
                    // Print dim lang label then switch to yellow for code
                    if !self.fence_lang_buf.is_empty() {
                        let label = self.fence_lang_buf.clone();
                        execute!(
                            out,
                            SetForegroundColor(Color::DarkGrey),
                            Print(format!(" {label}")),
                            SetAttribute(Attribute::Reset),
                            SetForegroundColor(Color::Yellow),
                        )?;
                    } else {
                        execute!(out, SetAttribute(Attribute::Reset), SetForegroundColor(Color::Yellow))?;
                    }
                    self.fence_lang_done = true;
                    execute!(out, Print(format!("\r\n{INDENT}")))?;
                    self.stream_col = CONTENT_PAD;
                    self.at_line_start = true;
                    continue;
                }
                // Flush any pending `*` before newline
                if let Some(p) = self.md_pending.take() {
                    execute!(out, Print(p))?;
                    self.stream_col = self.stream_col.saturating_add(1);
                }
                // Flush any pending num_buf before newline
                if !self.num_buf.is_empty() {
                    let nb = std::mem::take(&mut self.num_buf);
                    execute!(out, Print(&nb))?;
                    self.stream_col = self.stream_col.saturating_add(nb.chars().count() as u16);
                }
                execute!(out, Print(format!("\r\n{INDENT}")))?;
                self.stream_col = CONTENT_PAD;
                self.at_line_start = true;
                continue;
            }

            if ch == '\r' {
                continue; // skip bare CR
            }

            // ── Inside a code fence ────────────────────────────────────────
            if do_md && self.in_code_fence {
                if !self.fence_lang_done {
                    // Accumulate lang label
                    self.fence_lang_buf.push(ch);
                    continue; // don't print yet — wait for \n
                }
                // ── Inside fence body: handle closing ``` via backtick_run ─
                if ch == '`' {
                    self.backtick_run += 1;
                    if self.backtick_run == 3 {
                        // Closing fence
                        self.in_code_fence = false;
                        self.fence_lang_buf.clear();
                        self.fence_lang_done = false;
                        self.backtick_run = 0;
                        execute!(out, SetAttribute(Attribute::Reset), SetForegroundColor(Color::White))?;
                    }
                    continue;
                } else {
                    // Emit any held backticks inside the fence body (shouldn't normally happen)
                    for _ in 0..self.backtick_run {
                        execute!(out, Print('`'))?;
                        self.stream_col = self.stream_col.saturating_add(1);
                    }
                    self.backtick_run = 0;
                }
                // Normal char inside fence — print, track col
                if self.stream_col >= wrap_at as u16 && ch == ' ' {
                    execute!(out, Print(format!("\r\n{INDENT}")))?;
                    self.stream_col = CONTENT_PAD;
                    continue;
                }
                execute!(out, Print(ch))?;
                self.stream_col = self.stream_col.saturating_add(1);
                continue;
            }

            // ── Soft-wrap at word boundary ─────────────────────────────────
            if self.stream_col >= wrap_at as u16 && ch == ' ' {
                if let Some(p) = self.md_pending.take() {
                    execute!(out, Print(p))?;
                }
                if !self.num_buf.is_empty() {
                    let nb = std::mem::take(&mut self.num_buf);
                    execute!(out, Print(&nb))?;
                }
                execute!(out, Print(format!("\r\n{INDENT}")))?;
                self.stream_col = CONTENT_PAD;
                self.at_line_start = false;
                continue; // skip the space that triggered the wrap
            }

            if do_md {
                // ── Backtick: fence or inline code ─────────────────────────
                if ch == '`' {
                    self.backtick_run += 1;
                    continue; // hold — wait to see if it's 1, 2, or 3
                }
                if self.backtick_run > 0 {
                    let run = self.backtick_run;
                    self.backtick_run = 0;
                    if run == 3 {
                        // Opening ``` fence
                        self.in_code_fence = true;
                        self.fence_lang_done = false;
                        self.fence_lang_buf.clear();
                        // Start lang accumulation with current char (if not space/newline)
                        if ch != ' ' && ch != '\n' {
                            self.fence_lang_buf.push(ch);
                        }
                        continue;
                    } else if run == 1 && !self.in_code_fence {
                        // Single backtick: toggle inline code
                        self.in_code = !self.in_code;
                        execute!(
                            out,
                            if self.in_code { SetForegroundColor(Color::Yellow) }
                            else { SetForegroundColor(Color::White) },
                        )?;
                        // fall through: process `ch` normally
                    } else {
                        // 2 backticks: emit literally
                        for _ in 0..run {
                            execute!(out, Print('`'))?;
                            self.stream_col = self.stream_col.saturating_add(1);
                        }
                        // fall through: process `ch` normally
                    }
                }

                // ── Heading: `# ` at line start ───────────────────────────
                if self.at_line_start && ch == '#' {
                    self.at_line_start = false;
                    execute!(out, SetForegroundColor(Color::Cyan), SetAttribute(Attribute::Bold))?;
                    continue; // don't print the `#`
                }

                // ── Numbered list: digits at line start ───────────────────
                if self.at_line_start && ch.is_ascii_digit() {
                    self.num_buf.push(ch);
                    continue; // accumulate
                }
                if !self.num_buf.is_empty() {
                    if ch == '.' {
                        // confirmed list item prefix — store sentinel
                        self.md_pending = Some('N');
                        continue;
                    } else if self.md_pending == Some('N') && ch == ' ' {
                        // emit "N. " in bold
                        let nb = std::mem::take(&mut self.num_buf);
                        self.md_pending = None;
                        execute!(
                            out,
                            SetAttribute(Attribute::Bold),
                            Print(format!("{nb}. ")),
                            SetAttribute(Attribute::NormalIntensity),
                        )?;
                        self.stream_col = self.stream_col.saturating_add((nb.len() + 2) as u16);
                        self.at_line_start = false;
                        continue;
                    } else {
                        // not a list item — emit buffered digits literally
                        let nb = std::mem::take(&mut self.num_buf);
                        self.md_pending = None;
                        execute!(out, Print(&nb))?;
                        self.stream_col = self.stream_col.saturating_add(nb.chars().count() as u16);
                        // fall through to process `ch` normally
                    }
                }

                // ── Bullet: `- ` or `* ` at line start ───────────────────
                if self.at_line_start && (ch == '-' || ch == '*') {
                    self.md_pending = Some('•');
                    self.at_line_start = false;
                    continue;
                }
                if let Some('•') = self.md_pending {
                    if ch == ' ' {
                        self.md_pending = None;
                        execute!(out, Print("• "))?;
                        self.stream_col = self.stream_col.saturating_add(2);
                        continue;
                    } else {
                        let held = self.md_pending.take().unwrap();
                        execute!(out, Print(held))?;
                        self.stream_col = self.stream_col.saturating_add(1);
                        // fall through
                    }
                }

                // ── Bold `**...**` ─────────────────────────────────────────
                if ch == '*' {
                    if self.md_pending == Some('*') {
                        self.md_pending = None;
                        self.in_bold = !self.in_bold;
                        execute!(
                            out,
                            if self.in_bold { SetAttribute(Attribute::Bold) }
                            else { SetAttribute(Attribute::NormalIntensity) },
                        )?;
                    } else {
                        self.md_pending = Some('*');
                    }
                    continue;
                }
                // Flush pending `*` on non-`*` char
                if let Some('*') = self.md_pending {
                    execute!(out, Print('*'))?;
                    self.stream_col = self.stream_col.saturating_add(1);
                    self.md_pending = None;
                }

                // Clear line-start flag on first non-whitespace
                if self.at_line_start && !ch.is_whitespace() {
                    self.at_line_start = false;
                }
            }

            execute!(out, Print(ch))?;
            self.stream_col = self.stream_col.saturating_add(1);
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
