/// Streaming and bounded output renderer for the CADE REPL.
///
/// All output is rendered via ratatui `insert_before` (committed to scrollback).
/// The ThinkingBar (one row at terminal bottom) is the sole live element —
/// it shows streaming progress (tool name, word count) while the agent works.
/// Completed content (assistant text, tool calls, results) is committed in full
/// with Letta Code-style formatting: `●` prefix, dim separators, colored ⎿.

use std::io::{self, Write};
use unicode_width::UnicodeWidthStr;

use anyhow::Result;
use crossterm::{
    cursor,
    execute,
    style::{Color, Print, ResetColor, SetForegroundColor},
    terminal,
};
use ratatui::{
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

    /// Number of rows reserved at the terminal bottom by an active ThinkingBar.
    /// When non-zero, `with_insert_before` anchors to `term_h - 1 - status_bar_height`
    /// so content scrolls above the status bar rather than through it.
    status_bar_height: u16,

    /// When set, streaming progress updates (reasoning/assistant chunks) are
    /// written to this shared string rather than via raw stdout writes.
    /// The ThinkingBar task reads this to display animated status.
    bar_text: Option<std::sync::Arc<std::sync::Mutex<String>>>,

    /// ThinkingBar pause flag — set while a modal owns the terminal so the bar
    /// does not write to the alternate screen.
    bar_pause: Option<std::sync::Arc<std::sync::atomic::AtomicBool>>,

    /// Number of blank rows sitting at the bottom of the content area left over
    /// from the most recent InputWidget viewport clear.  `with_insert_before`
    /// reuses these rows instead of scrolling new blank ones, then compacts any
    /// remaining gap with ANSI Delete-Line so content stays adjacent to the banner.
    blank_rows_at_bottom: u16,
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
            status_bar_height: 0,
            bar_text: None,
            bar_pause: None,
            blank_rows_at_bottom: 0,
        }
    }

    /// Attach the ThinkingBar's shared text and pause flag — streaming updates
    /// route to bar_text; pause_bar() uses bar_pause to suppress rendering during
    /// modal dialogs that use the alternate screen.
    pub fn attach_bar(
        &mut self,
        text:  std::sync::Arc<std::sync::Mutex<String>>,
        pause: std::sync::Arc<std::sync::atomic::AtomicBool>,
    ) {
        self.bar_text  = Some(text);
        self.bar_pause = Some(pause);
    }

    /// Pause or resume ThinkingBar rendering (use while a modal owns the terminal).
    pub fn pause_bar(&self, paused: bool) {
        if let Some(ref f) = self.bar_pause {
            f.store(paused, std::sync::atomic::Ordering::SeqCst);
        }
    }

    /// Detach the ThinkingBar (call just before stopping the bar task).
    pub fn detach_bar(&mut self) {
        self.bar_text  = None;
        self.bar_pause = None;
    }

    /// Update the bar text (helper used by streaming methods).
    fn update_bar(&self, msg: impl Into<String>) {
        if let Some(ref bar) = self.bar_text {
            *bar.lock().unwrap() = msg.into();
        }
    }

    /// Activate (height=1) or deactivate (height=0) the ThinkingBar reservation.
    /// Must be called before `ThinkingBar::start()` and after it stops.
    pub fn set_status_bar(&mut self, active: bool) {
        self.status_bar_height = if active { 1 } else { 0 };
        if active {
            // ThinkingBar claims the bottom content row (old anchor). Decrement
            // rather than reset so the remaining blank rows stay tracked.
            self.blank_rows_at_bottom = self.blank_rows_at_bottom.saturating_sub(1);
        } else {
            // Former ThinkingBar row is now part of the content area and will be
            // overwritten by the next output call. Treat it as one extra blank row
            // so the compaction formula (blank_row_start = write_start - remaining)
            // stays correct after the anchor increases by 1.
            self.blank_rows_at_bottom = self.blank_rows_at_bottom.saturating_add(1);
        }
    }

    /// Record N blank rows that the InputWidget left at the bottom of the content
    /// area after clearing its viewport.  The next `with_insert_before` call will
    /// reuse these rows and compact any remaining gap.
    ///
    /// `status_bar_height` is subtracted so that, when the ThinkingBar is active,
    /// the row it occupies is not counted as a blank content row — preventing the
    /// compaction formula from pointing at a non-blank row.
    pub fn note_blank_rows(&mut self, n: u16) {
        self.blank_rows_at_bottom = n.saturating_sub(self.status_bar_height);
    }

    /// Insert a blank spacer line after an agent turn so consecutive response
    /// blocks are visually separated.
    pub fn turn_end(&mut self) -> Result<()> {
        self.close_streaming()?;
        self.with_insert_before(1, |_buf| {})
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

    /// Start of a reasoning block — init state and set ThinkingBar text.
    /// No raw stdout writes; the ThinkingBar shows the animated status.
    pub fn reasoning_header(&mut self) -> io::Result<()> {
        self.in_reasoning = true;
        self.stream_col = 0;
        self.update_bar("💭 thinking…");
        Ok(())
    }

    /// Write one chunk of reasoning text — buffer it, update ThinkingBar text.
    pub fn reasoning_chunk(&mut self, text: &str) -> io::Result<()> {
        self.reason_buf.push_str(text);
        let words = self.reason_buf.split_whitespace().count();
        self.update_bar(format!("💭 thinking… ({words} words)"));
        Ok(())
    }

    /// Close reasoning block — commit a collapsed header (Letta Code style), reset bar.
    /// Only the summary line is shown; the full text is not rendered (keeps screen clean).
    pub fn reasoning_done(&mut self) -> Result<()> {
        if !self.in_reasoning {
            return Ok(());
        }
        self.in_reasoning = false;
        self.stream_col = 0;
        self.update_bar("CADE thinking…");
        let buf = std::mem::take(&mut self.reason_buf);
        self.update_width();

        if !buf.trim().is_empty() {
            let words = buf.split_whitespace().count();
            // Collapsed: just show "💭 Reasoning (N words)" — matches Letta Code default
            let header = Line::from(Span::styled(
                format!("💭 Reasoning ({words} words)"),
                Style::default().fg(RC::DarkGray).add_modifier(Modifier::ITALIC),
            ));
            self.with_insert_before(1, move |buf_ref| {
                Paragraph::new(header).render(*buf_ref.area(), buf_ref);
            })?;
        }
        Ok(())
    }

    /// Write one chunk of assistant text — buffer it, update ThinkingBar text.
    /// Content is committed as a styled block when the stream finishes.
    pub fn assistant_chunk(&mut self, text: &str) -> io::Result<()> {
        if !self.in_assistant {
            self.in_assistant = true;
            self.stream_col = 0;
        }
        self.response_buf.push_str(text);
        let words = self.response_buf.split_whitespace().count();
        self.update_bar(format!("● generating… ({words} words)"));
        Ok(())
    }

    /// Close assistant block — commit text with Letta Code-style purple ● prefix.
    pub fn assistant_done(&mut self) -> Result<()> {
        if !self.in_assistant {
            return Ok(());
        }
        self.in_assistant = false;
        self.stream_col = 0;
        self.update_bar("CADE thinking…");
        let buf = std::mem::take(&mut self.response_buf);
        self.update_width();

        if !buf.trim().is_empty() {
            let width = self.wrap_width();
            // Letta Code style: first line gets purple ●, continuations are indented
            let lines = build_assistant_lines(&buf);
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

    /// Echo user message — Letta Code's UserMessage style.
    /// Renders via `with_insert_before` so it stays in line with all other output.
    /// Prepends a turn-separator (───) above the user text.
    pub fn user_message(&mut self, text: &str) -> Result<()> {
        self.close_streaming()?;
        self.update_width();
        let trimmed = text.trim();
        if trimmed.is_empty() {
            return Ok(());
        }
        let wrap_w = self.wrap_width();
        let sep = "─".repeat(wrap_w as usize);
        let mut lines: Vec<Line<'static>> = vec![
            Line::from(Span::styled(sep, Style::default().fg(RC::DarkGray))),
        ];
        for (i, line) in trimmed.lines().enumerate() {
            let prefix = if i == 0 { "> " } else { "  " };
            lines.push(Line::from(Span::styled(
                format!("{prefix}{line}"),
                Style::default().fg(RC::White),
            )));
        }
        let height = estimate_height(&lines, wrap_w as usize);
        self.with_insert_before(height, move |buf| {
            Paragraph::new(lines)
                .wrap(Wrap { trim: false })
                .render(*buf.area(), buf);
        })
    }

    /// Tool call — Letta Code-style: `● Name(args…)` on a single line.
    /// ● is green, Name is bold-white (or purple for memory tools), (args) is plain.
    /// Preceded by a 1-row blank spacer for visual breathing room.
    pub fn tool_call(&mut self, name: &str, preview: &str) -> Result<()> {
        self.close_streaming()?;
        self.update_width();
        // Blank spacer before each tool group (Letta Code block spacing)
        self.with_insert_before(1, |_buf| {})?;
        let display = display_tool_name(name);
        // Budget: full width minus "● " (2) minus display name minus parens
        let args_budget = self.term_width.saturating_sub(2 + display.len() as u16 + 2) as usize;

        // Memory tools get Letta Code's colors.tool.memoryName = #8C8CF9 (purple)
        let is_memory = name.to_ascii_lowercase().contains("memory");
        let name_style = if is_memory {
            Style::default().add_modifier(Modifier::BOLD).fg(RC::Rgb(140, 140, 249))
        } else {
            Style::default().add_modifier(Modifier::BOLD)
        };

        // Letta Code: tool dot is GRAY while running (#A5A8AB = streaming/running phase)
        // It turns green only after the result arrives — we can't retroactively update the
        // committed dot, so the result ⎿ line carries the green/red success indicator.
        let dot_color = RC::Rgb(165, 168, 171); // #A5A8AB
        let mut spans = vec![
            Span::styled("● ", Style::default().fg(dot_color).add_modifier(Modifier::BOLD)),
            Span::styled(display.clone(), name_style),
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
        // Letta Code palette: completed=green #64CF64, error=pink #F1689F
        let color = if is_error {
            RC::Rgb(241, 104, 159) // #F1689F
        } else {
            RC::Rgb(100, 207, 100) // #64CF64
        };
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

    /// Print the CADE banner via `with_insert_before` so it anchors to the
    /// terminal bottom — eliminating the blank-space gap before the input widget.
    pub fn banner(
        &mut self,
        banner: &str,
        agent_name: &str,
        agent_id: &str,
        model: &str,
        mode: &str,
    ) -> io::Result<()> {
        // Build ratatui Lines: ASCII art in cyan, info lines in dark-gray.
        let info = format!(
            " Agent : {} ({})\n Model : {}\n Mode  : {}\n",
            agent_name, agent_id, model, mode
        );
        let full = format!("{}{}", banner, info);
        let lines: Vec<Line<'static>> = full
            .lines()
            .map(|l| {
                let owned = l.to_string();
                // Heuristic: info lines start with " Agent", " Model", " Mode",
                // or "Type /help".  Everything else is the ASCII art → cyan.
                let is_info = owned.trim_start().starts_with("Agent")
                    || owned.trim_start().starts_with("Model")
                    || owned.trim_start().starts_with("Mode")
                    || owned.trim_start().starts_with("Type /");
                if is_info {
                    Line::from(Span::styled(owned, Style::default().fg(RC::DarkGray)))
                } else if owned.trim().is_empty() {
                    Line::from(owned)
                } else {
                    Line::from(Span::styled(owned, Style::default().fg(RC::Cyan)))
                }
            })
            .collect();
        let height = lines.len() as u16;
        self.with_insert_before(height, move |buf| {
            Paragraph::new(lines).render(*buf.area(), buf);
        })
        .map_err(|e| io::Error::new(io::ErrorKind::Other, e))
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
        // Blank spacer before each tool group (Letta Code block spacing)
        self.with_insert_before(1, |_buf| {})?;

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

    /// Scroll `height` rows of space into the content area and render `f` there.
    ///
    /// Uses DECSTBM (scroll-region escape) to scroll only the content rows,
    /// keeping the ThinkingBar row untouched.  Renders the ratatui `Buffer`
    /// directly with raw ANSI codes — **no** `Terminal::with_options` and **no**
    /// cursor-position query (`\033[6n`).
    ///
    /// Eliminating cursor-position queries removes the race with the REPL's
    /// `crossterm::event::read()` that caused CPR bytes (`\033[row;colR`) to
    /// appear as visible text in the terminal output.
    fn with_insert_before<F>(&mut self, height: u16, f: F) -> Result<()>
    where
        F: FnOnce(&mut Buffer),
    {
        let height = height.max(1);
        let Ok((term_w, term_h)) = terminal::size() else { return Ok(()); };

        // Build ratatui buffer in memory (widget API unchanged for callers).
        let area = Rect::new(0, 0, term_w, height);
        let mut buf = Buffer::empty(area);
        f(&mut buf);

        // anchor: last content row (0-indexed), just above the status bar.
        let anchor = term_h.saturating_sub(1).saturating_sub(self.status_bar_height);
        // Clamp height to the available content rows to avoid writing past anchor.
        let height = height.min(anchor + 1);

        // How many pre-existing blank rows (left by InputWidget) can we reuse?
        // Clamped to [0, anchor+1] to stay in-bounds.
        let blank = self.blank_rows_at_bottom.min(anchor + 1);
        // Rows we must scroll in vs rows we can reuse from the blank region.
        let used       = blank.min(height);
        let new_rows   = height - used;           // newlines to emit
        let remaining  = blank.saturating_sub(height); // blank rows left after write

        let mut out = io::stdout();

        // 1. Restrict the scroll region to rows [0, anchor] (DECSTBM, 1-indexed).
        //    Newlines at `anchor` now scroll only the content area; the ThinkingBar
        //    at term_h-1 is outside the region and is completely unaffected.
        write!(out, "\x1b[1;{}r", anchor + 1)?;

        // 2. Move to the bottom of the scroll region, emit `new_rows` newlines.
        //    Each newline at the bottom of the scroll region scrolls everything
        //    within the region up by 1 and keeps the cursor at `anchor`.
        //    When `new_rows < height`, we reuse pre-existing blank rows instead of
        //    scrolling new ones, avoiding spurious gaps in the output.
        execute!(out, cursor::MoveToRow(anchor), cursor::MoveToColumn(0))?;
        for _ in 0..new_rows {
            write!(out, "\n")?;
        }

        // 3. Reset scroll region to the full terminal (rows 1..term_h).
        write!(out, "\x1b[r")?;

        // 4. Write buffer rows into the space at [anchor-height+1 .. anchor].
        //    After `new_rows` newlines the blank region (pre-existing + scrolled-in)
        //    covers exactly `height` rows ending at `anchor`.
        let write_start = anchor.saturating_sub(height.saturating_sub(1));
        for y in 0..height {
            execute!(out, cursor::MoveTo(0, write_start + y))?;
            render_buf_row(&mut out, &buf, y)?;
        }

        // 5. Compact any remaining blank rows that sit between the previous content
        //    and the newly written rows.  ANSI Delete Line (\x1b[nM]) at the first
        //    blank row deletes those rows and shifts the written rows UP, closing
        //    the gap without leaving visible whitespace.
        if remaining > 0 {
            let blank_row_start = write_start.saturating_sub(remaining);
            write!(out, "\x1b[1;{}r", anchor + 1)?;   // DECSTBM [0, anchor]
            execute!(out, cursor::MoveTo(0, blank_row_start))?;
            write!(out, "\x1b[{}M", remaining)?;       // delete `remaining` lines
            write!(out, "\x1b[r")?;                    // reset DECSTBM
        }

        // Keep tracking: blank rows left at bottom of content area after compaction.
        self.blank_rows_at_bottom = remaining;

        out.flush()?;
        Ok(())
    }

}

impl Default for OutputRenderer {
    fn default() -> Self {
        Self::new()
    }
}

use crate::ui::markdown::parse_markdown_lines;

// ── Direct terminal rendering helpers ─────────────────────────────────────────
//
// Used by `with_insert_before` to write a ratatui Buffer to the terminal
// WITHOUT creating a `Terminal::with_options(Viewport::Inline)`.  That
// constructor queries cursor position (`\033[6n` → stdin), which races with
// `crossterm::event::read()` on the REPL's event loop and causes CPR bytes
// (`\033[row;colR`) to leak into the terminal output as visible text.

/// Convert ratatui Color to crossterm Color for raw ANSI output.
fn rc_to_ct(c: RC) -> Color {
    match c {
        RC::Black         => Color::Black,
        RC::Red           => Color::DarkRed,
        RC::Green         => Color::DarkGreen,
        RC::Yellow        => Color::DarkYellow,
        RC::Blue          => Color::DarkBlue,
        RC::Magenta       => Color::DarkMagenta,
        RC::Cyan          => Color::DarkCyan,
        RC::Gray          => Color::Grey,
        RC::DarkGray      => Color::DarkGrey,
        RC::LightRed      => Color::Red,
        RC::LightGreen    => Color::Green,
        RC::LightYellow   => Color::Yellow,
        RC::LightBlue     => Color::Blue,
        RC::LightMagenta  => Color::Magenta,
        RC::LightCyan     => Color::Cyan,
        RC::White         => Color::White,
        RC::Rgb(r, g, b)  => Color::Rgb { r, g, b },
        RC::Indexed(i)    => Color::AnsiValue(i),
        RC::Reset         => Color::Reset,
    }
}

/// Write one row of a ratatui `Buffer` to `out` using raw ANSI codes.
///
/// Emits SGR (Select Graphic Rendition) codes only when the style changes,
/// then writes `cell.symbol()` for each cell.  Ends with a full attribute
/// reset (`\x1b[0m`) and an "erase to end of line" (`\x1b[K`) so no stale
/// content from a previous render bleeds through.
fn render_buf_row(out: &mut impl io::Write, buf: &Buffer, y: u16) -> io::Result<()> {
    let width = buf.area().width;
    let mut prev_fg = RC::Reset;
    let mut prev_modifier = Modifier::empty();

    for x in 0..width {
        let Some(cell) = buf.cell((x, y)) else { continue; };

        if cell.fg != prev_fg || cell.modifier != prev_modifier {
            // Full reset then reapply — simplest way to handle modifier removal.
            write!(out, "\x1b[0m")?;
            prev_fg = RC::Reset;
            prev_modifier = Modifier::empty();

            if cell.modifier.contains(Modifier::BOLD)   { write!(out, "\x1b[1m")?; }
            if cell.modifier.contains(Modifier::DIM)    { write!(out, "\x1b[2m")?; }
            if cell.modifier.contains(Modifier::ITALIC) { write!(out, "\x1b[3m")?; }
            match cell.fg {
                RC::Reset => { /* already reset above */ }
                c => execute!(out, SetForegroundColor(rc_to_ct(c)))?,
            }
            prev_fg = cell.fg;
            prev_modifier = cell.modifier;
        }

        write!(out, "{}", cell.symbol())?;
    }
    // Reset all attributes and erase any stale content to the right.
    write!(out, "\x1b[0m\x1b[K")?;
    Ok(())
}

// ── Assistant line builder ─────────────────────────────────────────────────────

/// Build ratatui lines for a committed (or live-streaming) assistant message.
///
/// Format (Letta Code style):
///   `●  first markdown line`   ← purple ● (processing color #8C8CF9)
///   `   continuation…`         ← 3-space indent for subsequent lines
///
/// The first parsed text line gets the `●` prefix; subsequent lines get
/// a 3-space indent so they align under the text (not the dot).
fn build_assistant_lines(text: &str) -> Vec<Line<'static>> {
    // Purple processing color: Letta Code colors.status.processing = #8C8CF9
    const PURPLE: RC = RC::Rgb(140, 140, 249);
    const CONT_INDENT: &str = "   "; // 3 spaces align with text after "●  "

    let mut markdown_lines = parse_markdown_lines(text);
    if markdown_lines.is_empty() {
        return vec![];
    }

    // Prepend the ● to the very first line
    let first = markdown_lines.remove(0);
    let mut first_spans: Vec<Span<'static>> = vec![
        Span::styled("● ", Style::default().fg(PURPLE).add_modifier(Modifier::BOLD)),
    ];
    first_spans.extend(first.spans);
    markdown_lines.insert(0, Line::from(first_spans));

    // For each continuation line, prepend 3-space indent
    for line in markdown_lines.iter_mut().skip(1) {
        let mut spans: Vec<Span<'static>> = vec![Span::raw(CONT_INDENT)];
        spans.extend(std::mem::take(&mut line.spans));
        line.spans = spans;
    }

    markdown_lines
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
        "install_skill"                                       => "Install Skill",
        "run_skill_script"                                    => "Run Script",
        "load_skill_ref"                                      => "Skill Ref",
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
