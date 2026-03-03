/// TuiApp — single-terminal, pure ratatui fullscreen rendering for CADE.
///
/// Replaces the old hybrid (OutputRenderer DECSTBM + InputWidget Inline viewport +
/// ThinkingBar raw crossterm).  A single `Terminal<CrosstermBackend<Stdout>>`
/// (alternate screen, raw mode) is owned here.  Every piece of output — agent
/// streaming, tool results, slash-command text, errors — is represented as a
/// `RenderLine` pushed into `lines`.  `draw()` redraws the whole screen on every
/// state change, eliminating all the CPR / DECSTBM / blank-row-tracking hacks.
///
/// Layout (each frame):
/// ```
/// ┌─────────────────────────────────────────┐
/// │       Content area  (scrollable)        │  term_h - (4 + input_rows)
/// ├─────────────────────────────────────────┤
/// │  ⠋ assessing…  OR  ✻ Considered for…   │  1  (status row)
/// ├─────────────────────────────────────────┤
/// │  ──────────────────────────── (sep)     │  1
/// │  > user input                           │  1..MAX_INPUT_ROWS
/// │  ──────────────────────────── (sep)     │  1
/// │  mode ✦          AgentName [model]      │  1  (footer)
/// └─────────────────────────────────────────┘
/// ```

use std::sync::{Arc, Mutex};
use std::time::Instant;

use anyhow::Result;
use crossterm::event::{self, Event, KeyCode, KeyEvent, KeyModifiers};
use ratatui::{
    DefaultTerminal, Frame,
    layout::{Constraint, Layout},
    style::{Color as RC, Modifier, Style},
    text::{Line, Span},
    widgets::{Paragraph, Widget, Wrap},
};
use unicode_width::UnicodeWidthStr;

use crate::permissions::PermissionMode;

// ── Constants ─────────────────────────────────────────────────────────────────

/// Fixed non-input rows at the bottom: status + top_sep + bot_sep + footer.
const FIXED_ROWS: u16 = 4;
/// Maximum rows the input area may grow to.
const MAX_INPUT_ROWS: u16 = 6;
/// Braille spinner frames for thinking animation.
const BRAILLE: &[&str] = &["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏"];
/// Result prefix: "  ⎿  "
const RESULT_PREFIX: &str = "  ⎿  ";
/// Left-margin indent for markdown paragraphs (matches Letta Code style).
const INDENT: &str = "";

// ── RenderLine ────────────────────────────────────────────────────────────────

/// One logical unit of committed content in the conversation view.
#[derive(Clone, Debug)]
pub enum RenderLine {
    /// Full-width dim separator (between user turns).
    Separator,
    /// User message with `> ` prefix and preceding separator.
    UserMessage(String),
    /// Complete (committed) assistant response block.
    AssistantText(String),
    /// Tool call header: `● Name(args…)`.
    ToolCall { name: String, preview: String },
    /// Tool result: `  ⎿  summary`.
    ToolResult { is_error: bool, content: String },
    /// Collapsed reasoning block: `💭 Reasoning (N words)`.
    ReasoningHeader(usize),
    /// System / info message (dim gray).
    SystemMsg(String),
    /// Success message (green, ✓ prefix).
    SuccessMsg(String),
    /// Section header (cyan bold — e.g. "  MCP Servers").
    InfoHeader(String),
    /// Dim hint / secondary text (dark gray italic).
    DimMsg(String),
    /// Key-value pair aligned with padding between them.
    Pair { label: String, value: String },
    /// Error message (red).
    ErrorMsg(String),
    /// Blank spacer line.
    Blank,
}

// ── ThinkingState ─────────────────────────────────────────────────────────────

/// Active thinking animation state.
pub struct ThinkingState {
    /// Shared text updated by the assessing timer and on_event.
    pub text:    Arc<Mutex<String>>,
    /// When this turn started (for elapsed time display).
    pub started: Instant,
}

// ── TuiApp ────────────────────────────────────────────────────────────────────

pub struct TuiApp {
    /// The single ratatui terminal (alternate screen, raw mode).
    pub terminal: DefaultTerminal,

    // ── Content state ──────────────────────────────────────────────────────
    pub lines:    Vec<RenderLine>,
    /// Lines scrolled up from the bottom.  0 = show latest content.
    pub scroll:   usize,

    // ── Streaming state ────────────────────────────────────────────────────
    streaming_text:   String,
    streaming_active: bool,
    reasoning_text:   String,
    reasoning_active: bool,

    // ── Input state ────────────────────────────────────────────────────────
    pub input:      String,
    pub cursor_pos: usize,

    // ── Status / thinking ──────────────────────────────────────────────────
    pub thinking:    Option<ThinkingState>,
    pub last_status: Option<String>,

    // ── Footer info ────────────────────────────────────────────────────────
    pub mode:       PermissionMode,
    pub agent_name: String,
    pub model:      String,
}

impl TuiApp {
    /// Create the TuiApp and initialise the ratatui terminal
    /// (enters alternate screen + enables raw mode).
    pub fn new(mode: PermissionMode, agent_name: String, model: String) -> Self {
        let terminal = ratatui::init();
        Self {
            terminal,
            lines: Vec::new(),
            scroll: 0,
            streaming_text: String::new(),
            streaming_active: false,
            reasoning_text: String::new(),
            reasoning_active: false,
            input: String::new(),
            cursor_pos: 0,
            thinking: None,
            last_status: None,
            mode,
            agent_name,
            model,
        }
    }

    // ── Content mutation ──────────────────────────────────────────────────

    /// Commit any in-progress streaming, push a line, and redraw.
    pub fn push(&mut self, line: RenderLine) -> Result<()> {
        self.commit_streaming_inner();
        self.commit_reasoning_inner();
        self.lines.push(line);
        self.draw()
    }

    /// Push without redrawing (for bulk initialisation / banner).
    pub fn push_silent(&mut self, line: RenderLine) {
        self.commit_streaming_inner();
        self.commit_reasoning_inner();
        self.lines.push(line);
    }

    /// Append a streaming chunk and redraw.
    pub fn push_streaming_chunk(&mut self, text: &str) -> Result<()> {
        self.commit_reasoning_inner();
        self.streaming_active = true;
        self.streaming_text.push_str(text);
        self.draw()
    }

    /// Append a reasoning chunk (accumulated; committed as header on done).
    pub fn push_reasoning_chunk(&mut self, text: &str) {
        self.reasoning_active = true;
        self.reasoning_text.push_str(text);
    }

    /// Commit any in-progress assistant streaming to `lines`.
    pub fn commit_streaming(&mut self) -> Result<()> {
        self.commit_streaming_inner();
        self.draw()
    }

    /// Commit reasoning block as a collapsed header.
    pub fn commit_reasoning(&mut self) -> Result<()> {
        self.commit_reasoning_inner();
        self.draw()
    }

    /// Discard streaming state without committing (on cancel / error).
    pub fn discard_streaming(&mut self) {
        self.streaming_text.clear();
        self.streaming_active = false;
        self.reasoning_text.clear();
        self.reasoning_active = false;
    }

    pub fn has_streaming(&self) -> bool { self.streaming_active }

    /// Clear all content (e.g. /clear).
    pub fn clear_content(&mut self) -> Result<()> {
        self.lines.clear();
        self.discard_streaming();
        self.scroll = 0;
        self.draw()
    }

    fn commit_streaming_inner(&mut self) {
        if self.streaming_active {
            let text = std::mem::take(&mut self.streaming_text);
            if !text.trim().is_empty() {
                self.lines.push(RenderLine::AssistantText(text));
            }
            self.streaming_active = false;
        }
    }

    fn commit_reasoning_inner(&mut self) {
        if self.reasoning_active {
            let text = std::mem::take(&mut self.reasoning_text);
            let words = text.split_whitespace().count();
            if words > 0 {
                self.lines.push(RenderLine::ReasoningHeader(words));
            }
            self.reasoning_active = false;
        }
    }

    // ── Config updates ────────────────────────────────────────────────────

    pub fn update_model(&mut self, model: String)           { self.model = model; }
    pub fn update_mode(&mut self, mode: PermissionMode)     { self.mode  = mode; }
    pub fn update_agent_name(&mut self, name: String)       { self.agent_name = name; }
    pub fn set_last_status(&mut self, s: Option<String>)    { self.last_status = s; }

    // ── Thinking animation ────────────────────────────────────────────────

    /// Start the thinking animation.  Returns the shared text Arc so callers
    /// can update the status text (e.g. assessing timer, tool name updates).
    pub fn start_thinking(&mut self, text: impl Into<String>) -> Arc<Mutex<String>> {
        let arc = Arc::new(Mutex::new(text.into()));
        self.thinking = Some(ThinkingState { text: arc.clone(), started: Instant::now() });
        arc
    }

    /// Update the thinking text from the animation/assessing timer.
    pub fn update_thinking_text(&mut self, text: String) {
        if let Some(ref ts) = self.thinking {
            *ts.text.lock().unwrap() = text;
        }
    }

    /// Stop the thinking animation.  Returns elapsed seconds (for summary line).
    pub fn stop_thinking(&mut self) -> u64 {
        let secs = self.thinking.as_ref()
            .map(|ts| ts.started.elapsed().as_secs())
            .unwrap_or(0);
        self.thinking = None;
        secs
    }

    // ── Rendering ─────────────────────────────────────────────────────────

    /// Redraw the full screen.
    pub fn draw(&mut self) -> Result<()> {
        // Snapshot all rendering data (avoids borrow conflicts).
        let lines           = self.lines.clone();
        let streaming       = if self.streaming_active { Some(self.streaming_text.clone()) } else { None };
        let scroll          = self.scroll;
        let input           = self.input.clone();
        let cursor_pos      = self.cursor_pos;
        let mode            = self.mode;
        let agent_name      = self.agent_name.clone();
        let model           = self.model.clone();
        let last_status     = self.last_status.clone();
        let thinking_text   = self.thinking.as_ref().map(|ts| ts.text.lock().unwrap().clone());
        let thinking_elapsed = self.thinking.as_ref().map(|ts| ts.started.elapsed());

        self.terminal.draw(move |frame| {
            render_frame(
                frame,
                &lines,
                streaming.as_deref(),
                scroll,
                &input,
                cursor_pos,
                mode,
                &agent_name,
                &model,
                &last_status,
                thinking_text.as_deref(),
                thinking_elapsed,
            );
        })?;
        Ok(())
    }

    // ── Input loop ────────────────────────────────────────────────────────

    /// Block until the user submits input or presses Ctrl+D.
    /// Returns `None` on Ctrl+D (exit signal).
    pub fn read_input(
        &mut self,
        history:  &mut Vec<String>,
        hist_idx: &mut Option<usize>,
    ) -> Result<Option<String>> {
        self.input.clear();
        self.cursor_pos = 0;
        *hist_idx = None;

        loop {
            self.draw()?;
            // 50 ms poll: allows animation ticks without burning CPU.
            if !event::poll(std::time::Duration::from_millis(50))? {
                continue;
            }
            match event::read()? {
                Event::Key(k) => {
                    if let Some(result) = self.handle_key_input(k, history, hist_idx)? {
                        return Ok(result);
                    }
                }
                Event::Resize(_, _) => { /* ratatui picks up resize on next draw */ }
                _ => {}
            }
        }
    }

    fn handle_key_input(
        &mut self,
        k:        KeyEvent,
        history:  &mut Vec<String>,
        hist_idx: &mut Option<usize>,
    ) -> Result<Option<Option<String>>> {
        // Some(None)        = Ctrl+D (exit)
        // Some(Some(s))     = line submitted
        // None              = continue reading
        match (k.code, k.modifiers) {
            // ── Submit ────────────────────────────────────────────────────
            (KeyCode::Enter, m) if m == KeyModifiers::SHIFT => {
                self.input.insert(self.cursor_pos, '\n');
                self.cursor_pos += 1;
            }
            (KeyCode::Enter, _) => {
                let line = self.input.clone();
                self.scroll = 0; // snap to bottom on submit
                return Ok(Some(Some(line)));
            }

            // ── Exit ──────────────────────────────────────────────────────
            (KeyCode::Char('d'), KeyModifiers::CONTROL) if self.input.is_empty() => {
                return Ok(Some(None));
            }

            // ── Cancel / clear ────────────────────────────────────────────
            (KeyCode::Char('c'), KeyModifiers::CONTROL) => {
                self.input.clear();
                self.cursor_pos = 0;
                return Ok(Some(Some(String::new())));
            }
            (KeyCode::Esc, _) => {
                self.input.clear();
                self.cursor_pos = 0;
            }

            // ── Edit shortcuts ────────────────────────────────────────────
            (KeyCode::Char('u'), KeyModifiers::CONTROL) => {
                self.input.drain(..self.cursor_pos);
                self.cursor_pos = 0;
            }
            (KeyCode::Char('w'), KeyModifiers::CONTROL) => {
                let end   = self.cursor_pos;
                let start = self.input[..end]
                    .rfind(|c: char| !c.is_whitespace())
                    .and_then(|p| self.input[..p].rfind(char::is_whitespace).map(|q| q + 1))
                    .unwrap_or(0);
                self.input.drain(start..end);
                self.cursor_pos = start;
            }
            (KeyCode::Home, _) | (KeyCode::Char('a'), KeyModifiers::CONTROL) => {
                self.cursor_pos = 0;
            }
            (KeyCode::End, _) | (KeyCode::Char('e'), KeyModifiers::CONTROL) => {
                self.cursor_pos = self.input.len();
            }

            // ── Cursor movement ───────────────────────────────────────────
            (KeyCode::Left, _) if self.cursor_pos > 0 => {
                self.cursor_pos -= self.input[..self.cursor_pos]
                    .chars().last().map(|c| c.len_utf8()).unwrap_or(1);
            }
            (KeyCode::Right, _) if self.cursor_pos < self.input.len() => {
                self.cursor_pos += self.input[self.cursor_pos..]
                    .chars().next().map(|c| c.len_utf8()).unwrap_or(1);
            }

            // ── History ───────────────────────────────────────────────────
            (KeyCode::Up, _) if !history.is_empty() => {
                let new_idx = match *hist_idx {
                    None        => history.len() - 1,
                    Some(i) if i > 0 => i - 1,
                    Some(i)     => i,
                };
                *hist_idx    = Some(new_idx);
                self.input   = history[new_idx].clone();
                self.cursor_pos = self.input.len();
            }
            (KeyCode::Down, _) => {
                if let Some(i) = *hist_idx {
                    if i + 1 < history.len() {
                        *hist_idx = Some(i + 1);
                        self.input = history[i + 1].clone();
                        self.cursor_pos = self.input.len();
                    } else {
                        *hist_idx = None;
                        self.input.clear();
                        self.cursor_pos = 0;
                    }
                }
            }

            // ── Content scroll ────────────────────────────────────────────
            (KeyCode::PageUp, _) | (KeyCode::Up, KeyModifiers::ALT) => {
                self.scroll = self.scroll.saturating_add(10);
            }
            (KeyCode::PageDown, _) | (KeyCode::Down, KeyModifiers::ALT) => {
                self.scroll = self.scroll.saturating_sub(10);
            }

            // ── Mode cycle ────────────────────────────────────────────────
            (KeyCode::Tab, _) => {
                // Return a sentinel; repl.rs handles the actual mode change.
                self.scroll = 0;
                return Ok(Some(Some("__TAB__".to_string())));
            }
            (KeyCode::BackTab, _) => {
                self.scroll = 0;
                return Ok(Some(Some("__BACKTAB__".to_string())));
            }

            // ── Editing ───────────────────────────────────────────────────
            (KeyCode::Backspace, _) if self.cursor_pos > 0 => {
                let char_len = self.input[..self.cursor_pos]
                    .chars().last().map(|c| c.len_utf8()).unwrap_or(1);
                self.cursor_pos -= char_len;
                self.input.remove(self.cursor_pos);
            }
            (KeyCode::Delete, _) if self.cursor_pos < self.input.len() => {
                self.input.remove(self.cursor_pos);
            }
            (KeyCode::Char(c), m)
                if m == KeyModifiers::NONE || m == KeyModifiers::SHIFT =>
            {
                self.input.insert(self.cursor_pos, c);
                self.cursor_pos += c.len_utf8();
            }
            _ => {}
        }
        Ok(None)
    }
}

impl Drop for TuiApp {
    fn drop(&mut self) {
        ratatui::restore();
    }
}

// ── Frame renderer ────────────────────────────────────────────────────────────

#[allow(clippy::too_many_arguments)]
fn render_frame(
    frame:            &mut Frame,
    lines:            &[RenderLine],
    streaming:        Option<&str>,
    scroll:           usize,
    input:            &str,
    cursor_pos:       usize,
    mode:             PermissionMode,
    agent_name:       &str,
    model:            &str,
    last_status:      &Option<String>,
    thinking_text:    Option<&str>,
    thinking_elapsed: Option<std::time::Duration>,
) {
    let area = frame.area();
    let w    = area.width as usize;

    let available_w  = area.width.saturating_sub(2).max(1);
    let input_rows   = calc_input_rows(input, available_w).clamp(1, MAX_INPUT_ROWS);
    let bottom_rows  = FIXED_ROWS + input_rows;

    if area.height <= bottom_rows + 1 {
        frame.render_widget(Paragraph::new("Terminal too small"), area);
        return;
    }

    let content_height = area.height - bottom_rows;

    let chunks = Layout::vertical([
        Constraint::Length(content_height),  // [0] content
        Constraint::Length(1),               // [1] status
        Constraint::Length(1),               // [2] top separator
        Constraint::Length(input_rows),      // [3] input
        Constraint::Length(1),               // [4] bottom separator
        Constraint::Length(1),               // [5] footer
    ])
    .split(area);

    // ── Content area ─────────────────────────────────────────────────────────
    let mut text_lines: Vec<Line<'static>> = Vec::new();
    for rl in lines {
        render_line_to_text(rl, w, &mut text_lines);
    }
    if let Some(s) = streaming {
        render_assistant_lines(s, w, &mut text_lines);
    }

    // Count visual rows (accounting for line-wrap at content width).
    let content_w = area.width.saturating_sub(0).max(1);
    let total_visual: u16 = text_lines.iter().map(|l| {
        let char_count: usize = l.spans.iter().map(|s| s.content.chars().count()).sum();
        if char_count == 0 { 1 } else { ((char_count as u16).saturating_sub(1) / content_w) + 1 }
    }).sum();

    let visible = content_height;
    let para_scroll = if total_visual > visible {
        let max_skip     = total_visual - visible;
        let effective_up = (scroll as u16).min(max_skip);
        max_skip - effective_up
    } else {
        0
    };

    frame.render_widget(
        Paragraph::new(text_lines)
            .wrap(ratatui::widgets::Wrap { trim: false })
            .scroll((para_scroll, 0)),
        chunks[0],
    );

    // ── Status row ────────────────────────────────────────────────────────────
    let (status_text, status_style) = if let Some(elapsed) = thinking_elapsed {
        let text = thinking_text.unwrap_or("thinking…");
        let frame_idx = (elapsed.as_millis() / 120) as usize;
        let spinner   = BRAILLE[frame_idx % BRAILLE.len()];
        (
            format!("{spinner} {text}"),
            Style::default().fg(RC::DarkGray),
        )
    } else if let Some(s) = last_status {
        (
            s.clone(),
            Style::default().fg(RC::Rgb(100, 170, 120)).add_modifier(Modifier::DIM),
        )
    } else {
        (String::new(), Style::default())
    };
    frame.render_widget(
        Paragraph::new(Span::styled(status_text, status_style)),
        chunks[1],
    );

    // ── Separators ────────────────────────────────────────────────────────────
    let sep_color = mode_sep_color(mode);
    let sep       = "─".repeat(area.width as usize);
    frame.render_widget(
        Paragraph::new(Span::styled(sep.clone(), Style::default().fg(sep_color))),
        chunks[2],
    );
    frame.render_widget(
        Paragraph::new(Span::styled(sep, Style::default().fg(sep_color))),
        chunks[4],
    );

    // ── Input area ────────────────────────────────────────────────────────────
    let input_display = if input.is_empty() {
        Line::from(vec![
            Span::styled("> ", Style::default().fg(RC::White)),
            Span::styled("Type a message…", Style::default().fg(RC::DarkGray)),
        ])
    } else {
        Line::from(vec![
            Span::styled("> ", Style::default().fg(RC::White)),
            Span::styled(input.replace('\n', "↵ "), Style::default().fg(RC::White)),
        ])
    };
    frame.render_widget(
        Paragraph::new(input_display).wrap(Wrap { trim: false }),
        chunks[3],
    );

    // Cursor position
    let before = &input[..cursor_pos.min(input.len())];
    let (vis_row, vis_col) = calc_visual_cursor(before, available_w);
    let cx = (chunks[3].x + vis_col).min(chunks[3].x + chunks[3].width.saturating_sub(1));
    let cy = (chunks[3].y + vis_row).min(chunks[3].y + chunks[3].height.saturating_sub(1));
    frame.set_cursor_position((cx, cy));

    // ── Footer ────────────────────────────────────────────────────────────────
    let (left_label, left_glyph, left_color) = mode_footer_left(mode);
    let right_agent = agent_name.to_string();
    let right_model = format!(" [{}]", truncate_str(model, 30));

    let right_len = (right_agent.chars().count() + right_model.chars().count()) as u16;
    let left_base_len: u16 = left_label.chars().count() as u16
        + if left_glyph.is_empty() { 0 } else { 1 + left_glyph.chars().count() as u16 };
    let pad = chunks[5].width.saturating_sub(left_base_len + right_len) as usize;

    let mut footer: Vec<Span<'static>> = vec![
        Span::styled(left_label, Style::default().fg(left_color).add_modifier(Modifier::BOLD)),
    ];
    if !left_glyph.is_empty() {
        footer.push(Span::styled(
            format!(" {left_glyph}"),
            Style::default().fg(left_color),
        ));
    }
    footer.push(Span::raw(" ".repeat(pad)));
    footer.push(Span::styled(right_agent, Style::default().fg(RC::Rgb(140, 140, 249))));
    footer.push(Span::styled(right_model, Style::default().fg(RC::DarkGray)));

    frame.render_widget(Paragraph::new(Line::from(footer)), chunks[5]);
}

// ── Line renderers ────────────────────────────────────────────────────────────

fn render_line_to_text(rl: &RenderLine, width: usize, out: &mut Vec<Line<'static>>) {
    match rl {
        RenderLine::Separator => {
            out.push(Line::from(Span::styled(
                "─".repeat(width),
                Style::default().fg(RC::DarkGray),
            )));
        }
        RenderLine::Blank => {
            out.push(Line::from(""));
        }
        RenderLine::UserMessage(text) => {
            let sep = "─".repeat(width);
            out.push(Line::from(Span::styled(sep, Style::default().fg(RC::DarkGray))));
            for (i, ln) in text.trim().lines().enumerate() {
                let prefix = if i == 0 { "> " } else { "  " };
                out.push(Line::from(Span::styled(
                    format!("{prefix}{ln}"),
                    Style::default().fg(RC::White),
                )));
            }
        }
        RenderLine::AssistantText(text) => {
            render_assistant_lines(text, width, out);
        }
        RenderLine::ToolCall { name, preview } => {
            // Blank spacer before each tool group (Letta Code spacing).
            out.push(Line::from(""));
            let display = display_tool_name(name);
            let is_memory = name.to_ascii_lowercase().contains("memory");
            let name_style = if is_memory {
                Style::default().add_modifier(Modifier::BOLD).fg(RC::Rgb(140, 140, 249))
            } else {
                Style::default().add_modifier(Modifier::BOLD)
            };
            let budget = width.saturating_sub(display.len() + 4);
            let args   = truncate_str(preview, budget);
            let dot_color = RC::Rgb(165, 168, 171);
            let mut spans: Vec<Span<'static>> = vec![
                Span::styled("● ", Style::default().fg(dot_color).add_modifier(Modifier::BOLD)),
                Span::styled(display, name_style),
            ];
            if !preview.is_empty() {
                spans.push(Span::raw(format!("({args})")));
            }
            out.push(Line::from(spans));
        }
        RenderLine::ToolResult { is_error, content } => {
            let color = if *is_error {
                RC::Rgb(241, 104, 159)   // Letta Code error pink
            } else {
                RC::Rgb(100, 207, 100)   // Letta Code success green
            };
            let inner_w  = width.saturating_sub(5);
            let lns: Vec<&str> = content.lines().collect();
            if lns.is_empty() {
                out.push(Line::from(vec![
                    Span::styled(RESULT_PREFIX, Style::default().fg(RC::DarkGray)),
                    Span::styled("(no output)", Style::default().fg(RC::DarkGray)),
                ]));
            } else {
                out.push(Line::from(vec![
                    Span::styled(RESULT_PREFIX, Style::default().fg(RC::DarkGray)),
                    Span::styled(
                        truncate_str(lns[0], inner_w),
                        Style::default().fg(color).add_modifier(Modifier::BOLD),
                    ),
                ]));
                let show = lns.len().min(5);
                for ln in &lns[1..show] {
                    out.push(Line::from(vec![
                        Span::raw("     "),
                        Span::styled(truncate_str(ln, inner_w), Style::default().fg(color)),
                    ]));
                }
                if lns.len() > 5 {
                    out.push(Line::from(Span::styled(
                        format!("     … ({} more lines)", lns.len() - 5),
                        Style::default().fg(RC::DarkGray),
                    )));
                }
            }
        }
        RenderLine::ReasoningHeader(words) => {
            out.push(Line::from(Span::styled(
                format!("💭 Reasoning ({words} words)"),
                Style::default().fg(RC::DarkGray).add_modifier(Modifier::ITALIC),
            )));
        }
        RenderLine::SystemMsg(text) => {
            for ln in text.lines() {
                out.push(Line::from(Span::styled(
                    ln.to_string(),
                    Style::default().fg(RC::Gray),
                )));
            }
        }
        RenderLine::SuccessMsg(text) => {
            for ln in text.lines() {
                out.push(Line::from(Span::styled(
                    ln.to_string(),
                    Style::default().fg(RC::Green),
                )));
            }
        }
        RenderLine::InfoHeader(text) => {
            for ln in text.lines() {
                out.push(Line::from(Span::styled(
                    ln.to_string(),
                    Style::default().fg(RC::Cyan).add_modifier(Modifier::BOLD),
                )));
            }
        }
        RenderLine::DimMsg(text) => {
            for ln in text.lines() {
                out.push(Line::from(Span::styled(
                    ln.to_string(),
                    Style::default().fg(RC::DarkGray).add_modifier(Modifier::DIM),
                )));
            }
        }
        RenderLine::Pair { label, value } => {
            out.push(Line::from(vec![
                Span::styled(format!("  {label:<20}"), Style::default().fg(RC::DarkGray)),
                Span::styled(value.clone(), Style::default().fg(RC::White)),
            ]));
        }
        RenderLine::ErrorMsg(text) => {
            for ln in text.lines() {
                out.push(Line::from(Span::styled(
                    format!("  ✗ {ln}"),
                    Style::default().fg(RC::Red),
                )));
            }
        }
    }
}

fn render_assistant_lines(text: &str, _width: usize, out: &mut Vec<Line<'static>>) {
    let md_lines = parse_markdown_lines(text);
    if md_lines.is_empty() {
        out.push(Line::from(Span::styled(
            "● ",
            Style::default().fg(RC::Rgb(140, 100, 200)),
        )));
        return;
    }
    for (i, ml) in md_lines.into_iter().enumerate() {
        if i == 0 {
            // Prepend "● " (purple dot) to the first line of the response.
            let mut spans = vec![Span::styled(
                "● ",
                Style::default().fg(RC::Rgb(140, 100, 200)),
            )];
            spans.extend(ml.spans.into_iter());
            out.push(Line::from(spans));
        } else {
            out.push(ml);
        }
    }
}

// ── Markdown parser (ported from output.rs) ───────────────────────────────────

fn parse_markdown_lines(text: &str) -> Vec<Line<'static>> {
    let mut lines: Vec<Line<'static>> = Vec::new();
    let mut in_fence = false;

    for raw_line in text.lines() {
        let trimmed = raw_line.trim_start();

        // Code fence toggle
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
            continue;
        }
        if in_fence {
            lines.push(Line::from(Span::styled(
                format!("{INDENT}  {raw_line}"),
                Style::default().fg(RC::Yellow),
            )));
            continue;
        }
        if raw_line.trim().is_empty() {
            lines.push(Line::from(""));
            continue;
        }
        // Headings
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
        // Horizontal rule
        if trimmed == "---" || trimmed == "***" || trimmed == "===" {
            lines.push(Line::from(Span::styled(
                format!("{INDENT}{}", "─".repeat(40)),
                Style::default().fg(RC::DarkGray),
            )));
            continue;
        }
        // Bullets
        let bullet = trimmed.strip_prefix("- ")
            .or_else(|| trimmed.strip_prefix("* "))
            .or_else(|| trimmed.strip_prefix("• "));
        if let Some(rest) = bullet {
            let mut spans: Vec<Span<'static>> = vec![
                Span::raw(format!("{INDENT}  ")),
                Span::styled("• ", Style::default().fg(RC::Green)),
            ];
            spans.extend(parse_inline(rest));
            lines.push(Line::from(spans));
            continue;
        }
        // Numbered list
        if let Some((num, rest)) = parse_list_prefix(trimmed) {
            let mut spans: Vec<Span<'static>> = vec![
                Span::raw(format!("{INDENT}  ")),
                Span::styled(
                    format!("{num}. "),
                    Style::default().add_modifier(Modifier::BOLD),
                ),
            ];
            spans.extend(parse_inline(rest));
            lines.push(Line::from(spans));
            continue;
        }
        // Normal paragraph
        let mut spans: Vec<Span<'static>> = vec![Span::raw(INDENT)];
        spans.extend(parse_inline(trimmed));
        lines.push(Line::from(spans));
    }
    lines
}

fn parse_inline(text: &str) -> Vec<Span<'static>> {
    let mut spans = Vec::new();
    let mut rest  = text.to_string();
    while !rest.is_empty() {
        if let Some(pos) = rest.find("**") {
            let before = rest[..pos].to_string();
            if !before.is_empty() { spans.push(Span::raw(before)); }
            let after = &rest[pos + 2..];
            if let Some(end) = after.find("**") {
                spans.push(Span::styled(after[..end].to_string(), Style::default().add_modifier(Modifier::BOLD)));
                rest = after[end + 2..].to_string();
                continue;
            }
            spans.push(Span::raw(format!("**{after}")));
            break;
        }
        if let Some(pos) = rest.find('`') {
            let before = rest[..pos].to_string();
            if !before.is_empty() { spans.push(Span::raw(before)); }
            let after = &rest[pos + 1..];
            if let Some(end) = after.find('`') {
                spans.push(Span::styled(after[..end].to_string(), Style::default().fg(RC::Yellow)));
                rest = after[end + 1..].to_string();
                continue;
            }
            spans.push(Span::raw(format!("`{after}")));
            break;
        }
        if let Some(pos) = rest.find('*') {
            let before = rest[..pos].to_string();
            if !before.is_empty() { spans.push(Span::raw(before)); }
            let after = &rest[pos + 1..];
            if let Some(end) = after.find('*') {
                spans.push(Span::styled(after[..end].to_string(), Style::default().add_modifier(Modifier::ITALIC)));
                rest = after[end + 1..].to_string();
                continue;
            }
            spans.push(Span::raw(format!("*{after}")));
            break;
        }
        spans.push(Span::raw(rest.clone()));
        break;
    }
    if spans.is_empty() { spans.push(Span::raw(String::new())); }
    spans
}

fn parse_list_prefix(s: &str) -> Option<(&str, &str)> {
    let end = s.find(|c: char| !c.is_ascii_digit())?;
    if end == 0 { return None; }
    let rest = s[end..].strip_prefix(". ")?;
    Some((&s[..end], rest))
}

// ── Input helpers (ported from input.rs) ──────────────────────────────────────

fn calc_input_rows(buf: &str, available_width: u16) -> u16 {
    let w = available_width.max(1) as usize;
    if buf.is_empty() { return 1; }
    let mut total: u16 = 0;
    for (i, line) in buf.split('\n').enumerate() {
        let chars = line.chars().count();
        let row_w = if i == 0 { w.saturating_sub(2) } else { w }.max(1);
        let rows  = if chars == 0 { 1 } else { ((chars + row_w - 1) / row_w) as u16 };
        total += rows;
    }
    total.max(1).min(MAX_INPUT_ROWS)
}

fn calc_visual_cursor(before_cursor: &str, available_width: u16) -> (u16, u16) {
    let w            = available_width.max(1) as usize;
    let first_row_w  = w.saturating_sub(2).max(1);
    let mut vis_row: u16 = 0;
    let mut vis_col: u16 = 2;   // starts after "> "
    let mut is_first_visual_row = true;
    let mut chars_on_row: usize = 0;

    for ch in before_cursor.chars() {
        if ch == '\n' {
            vis_row += 1;
            vis_col = 0;
            chars_on_row = 0;
            is_first_visual_row = false;
        } else {
            chars_on_row += 1;
            let cap = if is_first_visual_row { first_row_w } else { w };
            if chars_on_row > cap {
                vis_row += 1;
                chars_on_row = 1;
                is_first_visual_row = false;
                vis_col = 1;
            } else {
                let prefix: u16 = if is_first_visual_row { 2 } else { 0 };
                vis_col = prefix + chars_on_row as u16;
            }
        }
    }
    (vis_row, vis_col)
}

fn mode_sep_color(mode: PermissionMode) -> RC {
    match mode {
        PermissionMode::Default           => RC::Rgb(70, 72, 74),
        PermissionMode::AcceptEdits       => RC::Rgb(140, 140, 249),
        PermissionMode::Plan              => RC::Green,
        PermissionMode::BypassPermissions => RC::Red,
    }
}

fn mode_footer_left(mode: PermissionMode) -> (&'static str, &'static str, RC) {
    match mode {
        PermissionMode::Default           => ("Press / for commands", "",    RC::Rgb(70, 72, 74)),
        PermissionMode::AcceptEdits       => ("accept edits",         "⏵⏵", RC::Rgb(140, 140, 249)),
        PermissionMode::Plan              => ("plan mode",            "⏸",  RC::Green),
        PermissionMode::BypassPermissions => ("bypass (allow all)",   "⚡",  RC::Red),
    }
}

pub fn cycle_mode(mode: PermissionMode) -> PermissionMode {
    match mode {
        PermissionMode::Default           => PermissionMode::AcceptEdits,
        PermissionMode::AcceptEdits       => PermissionMode::Plan,
        PermissionMode::Plan              => PermissionMode::BypassPermissions,
        PermissionMode::BypassPermissions => PermissionMode::Default,
    }
}

pub fn cycle_mode_back(mode: PermissionMode) -> PermissionMode {
    match mode {
        PermissionMode::Default           => PermissionMode::BypassPermissions,
        PermissionMode::AcceptEdits       => PermissionMode::Default,
        PermissionMode::Plan              => PermissionMode::AcceptEdits,
        PermissionMode::BypassPermissions => PermissionMode::Plan,
    }
}

// ── Misc helpers ──────────────────────────────────────────────────────────────

fn display_tool_name(name: &str) -> String {
    // Strip MCP server prefix: "developer__shell" → "shell"
    let stripped = if let Some(pos) = name.rfind("__") {
        &name[pos + 2..]
    } else {
        name
    };
    stripped.to_string()
}

pub fn truncate_str(s: &str, max: usize) -> String {
    let chars: Vec<char> = s.chars().collect();
    if chars.len() <= max {
        s.to_string()
    } else {
        format!("{}…", chars[..max.saturating_sub(1)].iter().collect::<String>())
    }
}

// ── Compatibility shims ───────────────────────────────────────────────────────

/// Dummy RawModeGuard — TuiApp keeps raw mode active throughout the session.
/// This no-op struct exists so call-sites in repl.rs don't need to change.
pub struct RawModeGuard;
impl RawModeGuard {
    pub fn enable() -> anyhow::Result<Self> { Ok(Self) }
}

/// Make a path relative to the current working directory for display.
pub fn make_relative_path(path: &str) -> String {
    let cwd = std::env::current_dir().unwrap_or_default();
    let p = std::path::Path::new(path);
    if let Ok(rel) = p.strip_prefix(&cwd) {
        format!("./{}", rel.display())
    } else {
        let cwd_parts: Vec<_> = cwd.components().collect();
        let path_parts: Vec<_> = p.components().collect();
        let common = cwd_parts.iter().zip(path_parts.iter())
            .take_while(|(a, b)| a == b)
            .count();
        let ups = cwd_parts.len() - common;
        let rest: std::path::PathBuf = path_parts[common..].iter().collect();
        if ups == 0 {
            format!("./{}", rest.display())
        } else {
            let prefix = "../".repeat(ups);
            format!("{prefix}{}", rest.display())
        }
    }
}
