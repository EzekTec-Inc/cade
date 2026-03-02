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

/// No global content padding — content starts at column 0 (Letta Code style).
pub const CONTENT_PAD: u16 = 0;
/// Left-margin for direct stdout writes — empty, content at column 0.
const INDENT: &str = "";
/// Result prefix: "  ⎿  " (5 chars — 2 spaces + ⎿ + 2 spaces).
const RESULT_PREFIX: &str = "  ⎿  ";
/// Result indent for continuation lines (5 spaces, aligns under RESULT_PREFIX).
const RESULT_INDENT: &str = "     ";
/// Kept for call-site compatibility; with CONTENT_PAD=0 this is a no-op.
fn padded_rect(area: Rect, _pad: u16) -> Rect { area }

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
        self.stream_col = 0;
        let mut out = io::stdout();
        execute!(
            out,
            Print("\n"),
            SetForegroundColor(Color::DarkGrey),
            SetAttribute(Attribute::Italic),
            Print("💭 thinking…"),
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
            Print(format!("💭 thinking… ({words} words)")),
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
                "💭 thinking…",
                Style::default().fg(RC::DarkGray).add_modifier(Modifier::ITALIC),
            ))];
            for text_line in buf.lines() {
                lines.push(Line::from(Span::styled(
                    text_line.to_string(),
                    Style::default().fg(RC::DarkGray).add_modifier(Modifier::ITALIC),
                )));
            }
            let height = estimate_height(&lines, width);
            self.with_insert_before(height, move |buf_ref| {
                Paragraph::new(lines).wrap(Wrap { trim: false }).render(*buf_ref.area(), buf_ref);
            })?;
        }
        Ok(())
    }

    /// Write one chunk of assistant text — buffer it, update spinner.
    pub fn assistant_chunk(&mut self, text: &str) -> io::Result<()> {
        if !self.in_assistant {
            self.in_assistant = true;
            self.stream_col = 0;
            let mut out = io::stdout();
            execute!(
                out,
                SetAttribute(Attribute::Reset),
                ResetColor,
                SetForegroundColor(Color::DarkGrey),
                Print("\n● generating…"),
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
            Print(format!("● generating… ({words} words)")),
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
                Paragraph::new(lines).wrap(Wrap { trim: false }).render(*buf_ref.area(), buf_ref);
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
            Paragraph::new(lines).wrap(Wrap { trim: false }).render(*buf.area(), buf);
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

    /// Tool call — Letta Code-style: `● Name(args…)` on a single line.
    /// ● is green, Name is bold-white, (args) is plain — matching Letta Code exactly.
    pub fn tool_call(&mut self, name: &str, preview: &str) -> Result<()> {
        self.close_streaming()?;
        self.update_width();
        let display = display_tool_name(name);
        // Budget: full width minus "● " (2) minus display name minus parens
        let args_budget = self.term_width.saturating_sub(2 + display.len() as u16 + 2) as usize;

        let mut spans = vec![
            Span::styled("● ", Style::default().fg(RC::Green).add_modifier(Modifier::BOLD)),
            Span::styled(display.clone(), Style::default().add_modifier(Modifier::BOLD)),
        ];
        if !preview.is_empty() {
            let truncated = truncate_str(preview, args_budget);
            spans.push(Span::raw(format!("({truncated})")));
        }
        let line = Line::from(spans);
        self.with_insert_before(1, move |buf| {
            Paragraph::new(line).render(*buf.area(), buf);
        })
    }

    /// Tool result — `  ⎿  summary` line in green (success) or red (error).
    pub fn tool_result(&mut self, is_error: bool, summary: &str) -> Result<()> {
        let color = if is_error { RC::Red } else { RC::Green };
        // 5 chars for "  ⎿  " prefix
        let max = self.term_width.saturating_sub(5) as usize;
        let line = Line::from(vec![
            Span::styled(RESULT_PREFIX, Style::default().fg(RC::DarkGray)),
            Span::styled(truncate_str(summary, max), Style::default().fg(color).add_modifier(Modifier::BOLD)),
        ]);
        self.with_insert_before(1, move |buf| {
            Paragraph::new(line).render(*buf.area(), buf);
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
            Paragraph::new(lines)
                .wrap(Wrap { trim: false })
                .render(*buf.area(), buf);
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
            Span::styled(RESULT_PREFIX, Style::default().fg(RC::DarkGray)),
            Span::styled(format!("Hook continuing: {reason}"), Style::default().fg(RC::DarkGray)),
        ]);
        self.with_insert_before(1, move |buf| {
            Paragraph::new(line).render(*buf.area(), buf);
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
            Paragraph::new(lines)
                .wrap(Wrap { trim: false })
                .render(*buf.area(), buf);
        })
    }

    /// Edit tool call — Letta Code style:
    ///   `● Edit(./relative_path)`         ← ● green, name bold-white, path plain
    ///   `  ⎿  Updated ./relative/path`    ← RESULT_PREFIX + header
    ///   `     Showing ~1 context line`    ← RESULT_INDENT + dim
    ///   `     113 -  old_line`            ← RESULT_INDENT + lineNo + " -  " + content (red)
    ///   `     113 +  new_line`            ← RESULT_INDENT + lineNo + " +  " + content (green)
    ///   `     114    context`             ← RESULT_INDENT + lineNo + "    " + content (dim)
    ///
    /// The `⎿ Updated` result is embedded here so `repl.rs` must NOT call
    /// `tool_result` separately for edit_file / apply_patch.
    pub fn tool_edit_call(
        &mut self,
        tool_name: &str,
        file_path: &str,
        old_str: &str,
        new_str: &str,
    ) -> Result<()> {
        self.close_streaming()?;
        self.update_width();

        // Relative path for display
        let rel_path = make_relative_path(file_path);

        // Read the file to extract context lines and the exact start line
        let file_content = std::fs::read_to_string(file_path).unwrap_or_default();
        let file_lines: Vec<&str> = file_content.lines().collect();

        let start_line: usize = file_content
            .find(old_str)
            .map(|byte_off| file_content[..byte_off].lines().count() + 1)
            .unwrap_or(1);

        let old_lines: Vec<&str> = old_str.lines().collect();
        let new_lines: Vec<&str> = new_str.lines().collect();
        const MAX_DIFF_LINES: usize = 6;
        const CONTEXT_LINES: usize = 1; // Letta Code uses 1 context line
        let show_old = old_lines.len().min(MAX_DIFF_LINES);
        let show_new = new_lines.len().min(MAX_DIFF_LINES);

        // Compute gutter width from max line number shown
        let max_ln = start_line + show_old.max(show_new) + CONTEXT_LINES;
        let gutter_w = max_ln.to_string().len();

        // Content budget: width - RESULT_INDENT(5) - gutter - " -  "(4)
        let inner_w = self.term_width.saturating_sub(5 + gutter_w as u16 + 4) as usize;

        // ── Collect context lines ─────────────────────────────────────────────
        let diff_start_0 = start_line.saturating_sub(1);
        let ctx_before_start = diff_start_0.saturating_sub(CONTEXT_LINES);
        let ctx_before: Vec<(usize, &str)> = (ctx_before_start..diff_start_0)
            .filter_map(|i| file_lines.get(i).map(|l| (i + 1, *l)))
            .collect();
        let diff_end_0 = diff_start_0 + old_lines.len();
        let ctx_after: Vec<(usize, &str)> = (diff_end_0..diff_end_0 + CONTEXT_LINES)
            .filter_map(|i| file_lines.get(i).map(|l| (i + 1, *l)))
            .collect();

        // ── Build line list ───────────────────────────────────────────────────
        let display = display_tool_name(tool_name);
        let args_budget = self.term_width.saturating_sub(2 + display.len() as u16 + 2) as usize;
        let path_arg = truncate_str(&rel_path, args_budget);

        let mut lines: Vec<Line> = vec![
            // ● Edit(./path)
            Line::from(vec![
                Span::styled("● ", Style::default().fg(RC::Green).add_modifier(Modifier::BOLD)),
                Span::styled(display, Style::default().add_modifier(Modifier::BOLD)),
                Span::raw(format!("({path_arg})")),
            ]),
            // ⎿  Updated ./rel/path
            Line::from(vec![
                Span::styled(RESULT_PREFIX, Style::default().fg(RC::DarkGray)),
                Span::styled(
                    format!("Updated {rel_path}"),
                    Style::default().fg(RC::Green).add_modifier(Modifier::BOLD),
                ),
            ]),
            // Showing ~1 context line
            Line::from(Span::styled(
                format!("{RESULT_INDENT}Showing ~{CONTEXT_LINES} context line"),
                Style::default().fg(RC::DarkGray),
            )),
        ];

        // Context before (dim)
        for (ln, ctx_l) in &ctx_before {
            lines.push(Line::from(vec![
                Span::styled(
                    format!("{RESULT_INDENT}{ln:>gutter_w$}    "),
                    Style::default().fg(RC::DarkGray),
                ),
                Span::styled(
                    truncate_str(ctx_l, inner_w),
                    Style::default().fg(RC::DarkGray),
                ),
            ]));
        }

        // Old lines (red -)
        for (i, old_l) in old_lines[..show_old].iter().enumerate() {
            let ln = start_line + i;
            lines.push(Line::from(vec![
                Span::styled(
                    format!("{RESULT_INDENT}{ln:>gutter_w$} -  "),
                    Style::default().fg(RC::DarkGray),
                ),
                Span::styled(truncate_str(old_l, inner_w), Style::default().fg(RC::Red)),
            ]));
        }
        if old_lines.len() > MAX_DIFF_LINES {
            lines.push(Line::from(Span::styled(
                format!("{RESULT_INDENT}… ({} more old lines)", old_lines.len() - MAX_DIFF_LINES),
                Style::default().fg(RC::DarkGray),
            )));
        }

        // New lines (green +)
        for (i, new_l) in new_lines[..show_new].iter().enumerate() {
            let ln = start_line + i;
            lines.push(Line::from(vec![
                Span::styled(
                    format!("{RESULT_INDENT}{ln:>gutter_w$} +  "),
                    Style::default().fg(RC::DarkGray),
                ),
                Span::styled(truncate_str(new_l, inner_w), Style::default().fg(RC::Green)),
            ]));
        }
        if new_lines.len() > MAX_DIFF_LINES {
            lines.push(Line::from(Span::styled(
                format!("{RESULT_INDENT}… ({} more new lines)", new_lines.len() - MAX_DIFF_LINES),
                Style::default().fg(RC::DarkGray),
            )));
        }

        // Context after (dim)
        for (ln, ctx_l) in &ctx_after {
            lines.push(Line::from(vec![
                Span::styled(
                    format!("{RESULT_INDENT}{ln:>gutter_w$}    "),
                    Style::default().fg(RC::DarkGray),
                ),
                Span::styled(
                    truncate_str(ctx_l, inner_w),
                    Style::default().fg(RC::DarkGray),
                ),
            ]));
        }

        let height = lines.len() as u16;
        self.with_insert_before(height, move |buf| {
            Paragraph::new(lines).render(*buf.area(), buf);
        })
    }

    /// Bash result — Letta Code CollapsedOutputDisplay style:
    /// first line on `  ⎿  `, continuation on `     `, capped at 5 lines.
    pub fn tool_bash_result(&mut self, output: &str) -> Result<()> {
        self.update_width();
        // 5 chars for RESULT_PREFIX
        let inner_w = self.term_width.saturating_sub(5) as usize;
        let all_lines: Vec<&str> = output.lines().collect();
        let count = all_lines.len();
        const PREVIEW: usize = 5;
        let show_n = count.min(PREVIEW);

        let mut lines: Vec<Line> = Vec::new();

        if count == 0 {
            lines.push(Line::from(vec![
                Span::styled(RESULT_PREFIX, Style::default().fg(RC::DarkGray)),
                Span::styled("(no output)", Style::default().fg(RC::DarkGray)),
            ]));
        } else {
            // First line on the ⎿ line
            lines.push(Line::from(vec![
                Span::styled(RESULT_PREFIX, Style::default().fg(RC::DarkGray)),
                Span::styled(truncate_str(all_lines[0], inner_w), Style::default().fg(RC::DarkGray)),
            ]));
            // Continuation lines: 5-space indent (RESULT_INDENT)
            for line_str in &all_lines[1..show_n] {
                lines.push(Line::from(vec![
                    Span::raw(RESULT_INDENT),
                    Span::styled(truncate_str(line_str, inner_w), Style::default().fg(RC::DarkGray)),
                ]));
            }
            if count > PREVIEW {
                lines.push(Line::from(Span::styled(
                    format!("{RESULT_INDENT}… ({} more lines)", count - PREVIEW),
                    Style::default().fg(RC::DarkGray),
                )));
            }
        }

        let height = lines.len() as u16;
        self.with_insert_before(height, move |buf| {
            Paragraph::new(lines).render(*buf.area(), buf);
        })
    }

    // ── Internals ─────────────────────────────────────────────────────────────

    fn wrap_width(&self) -> usize {
        self.term_width.saturating_sub(CONTENT_PAD * 2) as usize
    }

    /// Create a minimal Viewport::Inline(0) terminal, call `insert_before(height, f)`,
    /// then immediately drop the terminal.
    ///
    /// IMPORTANT: `insert_before` only works correctly when the viewport is at the
    /// terminal bottom. It issues a terminal ScrollUp(N) which renders content at
    /// the last N rows. If the cursor is mid-screen, the rendered content appears
    /// at the terminal bottom but the viewport is left mid-screen — subsequent
    /// calls then render at inconsistent positions.
    ///
    /// Fix: anchor cursor at the terminal bottom row before creating the viewport.
    /// `cursor::MoveToRow(N)` repositions without printing/scrolling, so no extra
    /// blank lines appear. After each call, the cursor returns to term_h - 1.
    fn with_insert_before<F>(&self, height: u16, f: F) -> Result<()>
    where
        F: FnOnce(&mut Buffer),
    {
        // Anchor to terminal bottom so insert_before works correctly
        if let Ok((_, term_h)) = terminal::size() {
            let _ = execute!(io::stdout(), cursor::MoveToRow(term_h.saturating_sub(1)));
        }
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

/// Map internal tool names to display names (capitalized, user-facing).
fn display_tool_name(name: &str) -> String {
    match name {
        "bash" | "run_command" | "execute_command" | "shell" => "Bash",
        "read_file" | "read"                                  => "Read",
        "write_file" | "create_file" | "write"               => "Write",
        "edit_file" | "edit"                                  => "Edit",
        "apply_patch" | "patch"                               => "Patch",
        "glob" | "find_files" | "list_files"                  => "Glob",
        "grep" | "search" | "search_files" | "Search"         => "Search",
        "list_directory" | "ls" | "list"                      => "List",
        "delete_file" | "remove_file"                         => "Delete",
        "move_file" | "rename_file"                           => "Move",
        "copy_file"                                           => "Copy",
        "update_memory" | "memory"                            => "Memory",
        "load_skill"                                          => "Skill",
        "run_subagent"                                        => "Agent",
        other => {
            // Capitalize first letter, leave rest unchanged
            let mut c = other.chars();
            return match c.next() {
                None    => String::new(),
                Some(f) => f.to_uppercase().collect::<String>() + c.as_str(),
            };
        }
    }.to_string()
}

/// Convert an absolute file path to a compact relative path for display.
/// Returns `../N/path` style if the path is under the current working directory,
/// otherwise returns the full absolute path.
pub fn make_relative_path(path: &str) -> String {
    let cwd = std::env::current_dir().unwrap_or_default();
    let p = std::path::Path::new(path);
    if let Ok(rel) = p.strip_prefix(&cwd) {
        format!("./{}", rel.display())
    } else {
        // Try a parent-relative path: find common prefix depth
        let mut cwd_parts: Vec<_> = cwd.components().collect();
        let mut path_parts: Vec<_> = p.components().collect();
        let common = cwd_parts.iter().zip(path_parts.iter())
            .take_while(|(a, b)| a == b)
            .count();
        cwd_parts.drain(..common);
        path_parts.drain(..common);
        let ups = cwd_parts.len();
        let rest: std::path::PathBuf = path_parts.iter().collect();
        if ups == 0 {
            format!("./{}", rest.display())
        } else {
            let prefix: std::path::PathBuf = std::iter::repeat("..").take(ups).collect();
            format!("{}", prefix.join(rest).display())
        }
    }
}

fn truncate_str(s: &str, max_chars: usize) -> String {
    let s = s.trim();
    if s.chars().count() <= max_chars {
        s.to_string()
    } else {
        format!("{}…", s.chars().take(max_chars.saturating_sub(1)).collect::<String>())
    }
}
