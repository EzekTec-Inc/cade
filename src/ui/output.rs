/// Streaming and bounded output renderer for the CADE REPL.
///
/// Two rendering paths:
/// 1. **Streaming** (reasoning/assistant chunks): raw text via direct stdout
///    writes while the LLM is generating. On completion, raw lines are erased
///    and re-rendered via ratatui `insert_before` with proper markdown styling.
/// 2. **Bounded** (tool calls, system msgs, headers): ratatui `insert_before`
///    with styled `Paragraph`/`Line`/`Span` widgets.

use std::io::{self, Write};
use unicode_width::UnicodeWidthStr;

use anyhow::Result;
use crossterm::{
    cursor,
    execute,
    style::{Attribute, Color, Print, ResetColor, SetAttribute, SetForegroundColor},
    terminal::{self, ClearType},
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
            // Use display width (2 for emoji/CJK) not char count, so insert_before
            // reserves the correct number of rows and doesn't overwrite content.
            let display_width: usize = l.spans.iter().map(|s| s.content.as_ref().width()).sum();
            ((display_width.max(1) - 1) / width.max(1) + 1) as u16
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

    // ── Streaming buffers (text accumulated for re-render at completion) ──
    /// Full assistant response text (accumulated while streaming).
    response_buf: String,
    /// Full reasoning text (accumulated while streaming).
    reason_buf: String,
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
            response_buf: String::new(),
            reason_buf: String::new(),
        }
    }

    /// Refresh terminal width (call on resize events).
    pub fn update_width(&mut self) {
        if let Ok((w, _)) = crossterm::terminal::size() {
            self.term_width = w;
        }
    }

    // ── Streaming paths ───────────────────────────────────────────────────────
    //
    // A single-line spinner is shown in-place during streaming (overwritten with
    // \r on each token). On completion, only that ONE line is erased via
    // Clear(CurrentLine), then insert_before renders the fully formatted content.
    //
    // This avoids cursor::MoveUp(N) + Clear(FromCursorDown) which previously:
    //   1) had an off-by-one (moved up to the row BEFORE streaming, clearing
    //      content from the previous output section)
    //   2) cleared the entire visible terminal for long responses (> term height)
    //   3) raced with println!() calls on the main thread (print_help, etc.)

    /// Start of a reasoning block — print spinner, init state.
    pub fn reasoning_header(&mut self) -> io::Result<()> {
        self.in_reasoning = true;
        self.stream_col = CONTENT_PAD;
        let mut out = io::stdout();
        execute!(
            out,
            Print("\n"),
            SetForegroundColor(Color::DarkGrey),
            SetAttribute(Attribute::Italic),
            Print(format!("{INDENT}💭 thinking…")),
            cursor::MoveToColumn(0),
        )?;
        out.flush()
    }

    /// Write one chunk of reasoning text — buffer it, update spinner.
    pub fn reasoning_chunk(&mut self, text: &str) -> io::Result<()> {
        self.reason_buf.push_str(text);
        // Overwrite spinner in-place with word count update
        let words = self.reason_buf.split_whitespace().count();
        let mut out = io::stdout();
        execute!(
            out,
            cursor::MoveToColumn(0),
            terminal::Clear(ClearType::CurrentLine),
            SetForegroundColor(Color::DarkGrey),
            SetAttribute(Attribute::Italic),
            Print(format!("{INDENT}💭 thinking… ({words} words)")),
            cursor::MoveToColumn(0),
        )?;
        out.flush()
    }

    /// Close reasoning block — erase spinner, re-render via ratatui.
    pub fn reasoning_done(&mut self) -> Result<()> {
        if !self.in_reasoning {
            return Ok(());
        }
        self.in_reasoning = false;
        self.stream_col = 0;
        let buf = std::mem::take(&mut self.reason_buf);
        self.update_width();

        let mut out = io::stdout();
        // Erase ONLY the spinner line (cursor already at col 0 from last chunk update)
        execute!(
            out,
            cursor::MoveToColumn(0),
            terminal::Clear(ClearType::CurrentLine),
            SetAttribute(Attribute::Reset),
            ResetColor,
        )?;
        out.flush()?;

        // Re-render with ratatui: italic gray header + body
        if !buf.trim().is_empty() {
            let width = self.wrap_width();
            let mut lines: Vec<Line> = vec![Line::from(Span::styled(
                format!("{INDENT}💭 thinking…"),
                Style::default().fg(RC::DarkGray).add_modifier(Modifier::ITALIC),
            ))];
            for text_line in buf.lines() {
                lines.push(Line::from(Span::styled(
                    format!("{INDENT}{text_line}"),
                    Style::default().fg(RC::DarkGray).add_modifier(Modifier::ITALIC),
                )));
            }
            let height = estimate_height(&lines, width);
            self.with_insert_before(height, move |buf_ref| {
                let area = padded_rect(*buf_ref.area(), CONTENT_PAD);
                Paragraph::new(lines).wrap(Wrap { trim: false }).render(area, buf_ref);
            })?;
        }
        Ok(())
    }

    /// Write one chunk of assistant text — buffer it, update spinner.
    pub fn assistant_chunk(&mut self, text: &str) -> io::Result<()> {
        if !self.in_assistant {
            self.in_assistant = true;
            self.stream_col = CONTENT_PAD;
            let mut out = io::stdout();
            execute!(
                out,
                SetAttribute(Attribute::Reset),
                ResetColor,
                SetForegroundColor(Color::DarkGrey),
                Print(format!("\n{INDENT}● generating…")),
                cursor::MoveToColumn(0),
            )?;
            out.flush()?;
        }
        self.response_buf.push_str(text);
        // Overwrite spinner in-place with token count
        let words = self.response_buf.split_whitespace().count();
        let mut out = io::stdout();
        execute!(
            out,
            cursor::MoveToColumn(0),
            terminal::Clear(ClearType::CurrentLine),
            SetForegroundColor(Color::DarkGrey),
            Print(format!("{INDENT}● generating… ({words} words)")),
            cursor::MoveToColumn(0),
        )?;
        out.flush()
    }

    /// Close assistant block — erase spinner, re-render via ratatui with markdown.
    pub fn assistant_done(&mut self) -> Result<()> {
        if !self.in_assistant {
            return Ok(());
        }
        self.in_assistant = false;
        self.stream_col = 0;
        let buf = std::mem::take(&mut self.response_buf);
        self.update_width();

        let mut out = io::stdout();
        // Erase ONLY the spinner line (cursor at col 0 from last chunk update)
        execute!(
            out,
            cursor::MoveToColumn(0),
            terminal::Clear(ClearType::CurrentLine),
            SetAttribute(Attribute::Reset),
            ResetColor,
        )?;
        out.flush()?;

        // Re-render with ratatui markdown formatting
        if !buf.trim().is_empty() {
            let width = self.wrap_width();
            let lines = parse_markdown_lines(&buf);
            let height = estimate_height(&lines, width);
            self.with_insert_before(height, move |buf_ref| {
                let area = padded_rect(*buf_ref.area(), CONTENT_PAD);
                Paragraph::new(lines).wrap(Wrap { trim: false }).render(area, buf_ref);
            })?;
        }
        Ok(())
    }

    /// Print a plain text block via ratatui insert_before.
    /// Routes output through OutputRenderer (safe for concurrent use) and
    /// calls close_streaming() first to avoid racing with any active stream.
    pub fn print_block(&mut self, text: &str) -> Result<()> {
        self.close_streaming()?;
        self.update_width();
        let wrap_w = self.wrap_width();
        let lines: Vec<Line> = text
            .lines()
            .map(|l| Line::from(Span::raw(l.to_string())))
            .collect();
        if lines.is_empty() {
            return Ok(());
        }
        let height = estimate_height(&lines, wrap_w);
        self.with_insert_before(height, move |buf| {
            let area = padded_rect(*buf.area(), CONTENT_PAD);
            Paragraph::new(lines).wrap(Wrap { trim: false }).render(area, buf);
        })
    }

    /// Close any open streaming block (call before bounded content).
    pub fn close_streaming(&mut self) -> Result<()> {
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

// ── Markdown parser ───────────────────────────────────────────────────────────

/// Convert a complete markdown text string into a `Vec<Line>` for ratatui rendering.
/// Handles: headings, bullets, numbered lists, code fences, horizontal rules, inline bold/code.
fn parse_markdown_lines(text: &str) -> Vec<Line<'static>> {
    let mut lines: Vec<Line<'static>> = Vec::new();
    let mut in_fence = false;

    for raw_line in text.lines() {
        let trimmed = raw_line.trim_start();

        // ── Code fence toggle ────────────────────────────────────────────
        if trimmed.starts_with("```") {
            in_fence = !in_fence;
            if in_fence {
                let lang = trimmed.trim_start_matches('`').trim();
                if !lang.is_empty() {
                    lines.push(Line::from(Span::styled(
                        format!("{INDENT}  {lang}"),
                        Style::default().fg(RC::DarkGray).add_modifier(Modifier::DIM),
                    )));
                }
            }
            // Don't emit the ``` line itself
            continue;
        }

        if in_fence {
            lines.push(Line::from(Span::styled(
                format!("{INDENT}  {raw_line}"),
                Style::default().fg(RC::Yellow),
            )));
            continue;
        }

        // ── Empty line ────────────────────────────────────────────────────
        if raw_line.trim().is_empty() {
            lines.push(Line::from(""));
            continue;
        }

        // ── Headings ──────────────────────────────────────────────────────
        if let Some(rest) = trimmed.strip_prefix("### ") {
            lines.push(Line::from(Span::styled(
                format!("{INDENT}{rest}"),
                Style::default().fg(RC::Cyan),
            )));
            continue;
        }
        if let Some(rest) = trimmed.strip_prefix("## ") {
            lines.push(Line::from(Span::styled(
                format!("{INDENT}{rest}"),
                Style::default().fg(RC::Cyan).add_modifier(Modifier::BOLD),
            )));
            continue;
        }
        if let Some(rest) = trimmed.strip_prefix("# ") {
            lines.push(Line::from(Span::styled(
                format!("{INDENT}{rest}"),
                Style::default().fg(RC::Cyan).add_modifier(Modifier::BOLD | Modifier::UNDERLINED),
            )));
            continue;
        }

        // ── Horizontal rule ────────────────────────────────────────────────
        if trimmed == "---" || trimmed == "***" || trimmed == "===" {
            lines.push(Line::from(Span::styled(
                format!("{INDENT}{}", "─".repeat(40)),
                Style::default().fg(RC::DarkGray),
            )));
            continue;
        }

        // ── Bullet list ────────────────────────────────────────────────────
        let bullet_rest = trimmed
            .strip_prefix("- ")
            .or_else(|| trimmed.strip_prefix("* "))
            .or_else(|| trimmed.strip_prefix("• "));
        if let Some(rest) = bullet_rest {
            let mut spans: Vec<Span<'static>> = vec![
                Span::raw(format!("{INDENT}  ")),
                Span::styled("• ", Style::default().fg(RC::Green)),
            ];
            spans.extend(parse_inline(rest));
            lines.push(Line::from(spans));
            continue;
        }

        // ── Numbered list ─────────────────────────────────────────────────
        if let Some((num, rest)) = parse_list_prefix(trimmed) {
            let mut spans: Vec<Span<'static>> = vec![
                Span::raw(format!("{INDENT}  ")),
                Span::styled(format!("{num}. "), Style::default().add_modifier(Modifier::BOLD)),
            ];
            spans.extend(parse_inline(rest));
            lines.push(Line::from(spans));
            continue;
        }

        // ── Normal paragraph line with inline spans ───────────────────────
        let mut spans: Vec<Span<'static>> = vec![Span::raw(INDENT)];
        spans.extend(parse_inline(trimmed));
        lines.push(Line::from(spans));
    }

    lines
}

/// Parse inline markdown spans within a single line of text.
/// Handles: `**bold**`, `` `code` ``, `*italic*`.
fn parse_inline(text: &str) -> Vec<Span<'static>> {
    let mut spans = Vec::new();
    let mut rest = text.to_string();

    while !rest.is_empty() {
        // ── Bold: **…** ────────────────────────────────────────────────
        if let Some(pos) = rest.find("**") {
            let before = rest[..pos].to_string();
            if !before.is_empty() {
                spans.push(Span::raw(before));
            }
            let after_open = &rest[pos + 2..];
            if let Some(end) = after_open.find("**") {
                spans.push(Span::styled(
                    after_open[..end].to_string(),
                    Style::default().add_modifier(Modifier::BOLD),
                ));
                rest = after_open[end + 2..].to_string();
                continue;
            } else {
                // Unmatched ** — emit literally
                spans.push(Span::raw(format!("**{after_open}")));
                break;
            }
        }
        // ── Inline code: `…` ──────────────────────────────────────────
        if let Some(pos) = rest.find('`') {
            let before = rest[..pos].to_string();
            if !before.is_empty() {
                spans.push(Span::raw(before));
            }
            let after_open = &rest[pos + 1..];
            if let Some(end) = after_open.find('`') {
                spans.push(Span::styled(
                    after_open[..end].to_string(),
                    Style::default().fg(RC::Yellow),
                ));
                rest = after_open[end + 1..].to_string();
                continue;
            } else {
                spans.push(Span::raw(format!("`{after_open}")));
                break;
            }
        }
        // ── Italic: *…* ───────────────────────────────────────────────
        if let Some(pos) = rest.find('*') {
            let before = rest[..pos].to_string();
            if !before.is_empty() {
                spans.push(Span::raw(before));
            }
            let after_open = &rest[pos + 1..];
            if let Some(end) = after_open.find('*') {
                spans.push(Span::styled(
                    after_open[..end].to_string(),
                    Style::default().add_modifier(Modifier::ITALIC),
                ));
                rest = after_open[end + 1..].to_string();
                continue;
            } else {
                spans.push(Span::raw(format!("*{after_open}")));
                break;
            }
        }
        // ── Plain text (no more delimiters) ──────────────────────────
        spans.push(Span::raw(rest.clone()));
        break;
    }

    if spans.is_empty() {
        spans.push(Span::raw(String::new()));
    }
    spans
}

/// Detect a numbered list prefix like `1. ` at the start of `s`.
/// Returns `(number_str, rest_of_line)` or `None`.
fn parse_list_prefix(s: &str) -> Option<(&str, &str)> {
    let end = s.find(|c: char| !c.is_ascii_digit())?;
    if end == 0 {
        return None;
    }
    let rest = s[end..].strip_prefix(". ")?;
    Some((&s[..end], rest))
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
