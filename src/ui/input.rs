/// Ratatui-based input widget for the CADE REPL.
///
/// Layout matches Letta Code's InputRich component (Viewport::Inline(N)):
/// ```
///  ─────────────────────────────────────────────── (dim separator, full width)
///  > user types here                               (prompt char + text field)
///  ─────────────────────────────────────────────── (dim separator, full width)
///  plan (read-only) mode ⏸       AgentName [model] (footer: left=mode, right=agent/model)
/// ```
/// The input area grows dynamically (up to MAX_INPUT_ROWS) as the user types or
/// presses Shift+Enter.

use std::io::{self, Write};

use anyhow::Result;
use crossterm::{
    cursor,
    event::{self, Event, KeyCode, KeyEvent, KeyModifiers},
    execute,
    terminal::{self, ClearType},
};
use ratatui::{
    Terminal, TerminalOptions, Viewport,
    backend::CrosstermBackend,
    layout::{Constraint, Direction, Layout},
    style::{Color as RC, Modifier, Style},
    text::{Line, Span},
    widgets::{Paragraph, Wrap},
};

use crate::permissions::PermissionMode;

// ── Constants ─────────────────────────────────────────────────────────────────

/// Fixed rows: status(1) + top_sep(1) + bot_sep(1) + footer(1).
const FIXED_ROWS: u16 = 4;
/// Maximum rows the input area may occupy before capping.
const MAX_INPUT_ROWS: u16 = 6;

// ── RawModeGuard ──────────────────────────────────────────────────────────────

/// RAII guard that enables raw mode on construction and disables it on drop.
pub struct RawModeGuard;

impl RawModeGuard {
    pub fn enable() -> io::Result<Self> {
        terminal::enable_raw_mode()?;
        Ok(Self)
    }
}

impl Drop for RawModeGuard {
    fn drop(&mut self) {
        let _ = terminal::disable_raw_mode();
    }
}

// ── InputWidget ───────────────────────────────────────────────────────────────

/// Persistent state for the input widget across REPL iterations.
pub struct InputWidget {
    buf: String,
    cursor_pos: usize,
    term_width: u16,
    pub last_status: Option<String>,
}

impl InputWidget {
    pub fn new() -> Self {
        let term_width = crossterm::terminal::size()
            .map(|(w, _)| w)
            .unwrap_or(80);
        Self {
            buf: String::new(),
            cursor_pos: 0,
            term_width,
            last_status: None,
        }
    }

    pub fn update_width(&mut self) {
        if let Ok((w, _)) = crossterm::terminal::size() {
            self.term_width = w;
        }
    }

    /// Show the ratatui input box and read one user message.
    /// Returns `None` on Ctrl+D (exit signal).
    pub fn read(
        &mut self,
        history: &mut Vec<String>,
        hist_idx: &mut Option<usize>,
        mode: PermissionMode,
        permissions: &crate::permissions::PermissionManager,
        agent_name: &str,
        model: &str,
        _in_tokens: u64,
        _out_tokens: u64,
    ) -> Result<Option<String>> {
        self.buf.clear();
        self.cursor_pos = 0;

        // Dynamic viewport: starts at FIXED_ROWS + 1; grows up to FIXED_ROWS + MAX_INPUT_ROWS.
        let mut current_input_rows: u16 = 1;
        let mut viewport_height: u16 = FIXED_ROWS + current_input_rows;

        let (_, init_term_h) = terminal::size().unwrap_or((80, 24));
        let _ = execute!(io::stdout(), cursor::MoveToRow(init_term_h.saturating_sub(1)));
        {
            let pre_backend = CrosstermBackend::new(io::stdout());
            let mut pre_term = Terminal::with_options(
                pre_backend,
                TerminalOptions { viewport: Viewport::Inline(0) },
            )?;
            let _ = pre_term.insert_before(viewport_height, |_buf| {});
        }
        let _ = execute!(io::stdout(), cursor::MoveToRow(init_term_h.saturating_sub(1)));
        let backend = CrosstermBackend::new(io::stdout());
        let mut term = Terminal::with_options(
            backend,
            TerminalOptions { viewport: Viewport::Inline(viewport_height) },
        )?;
        let mut viewport_start_row = init_term_h.saturating_sub(viewport_height);

        let _raw = RawModeGuard::enable()?;

        let result: Result<Option<String>> = (|| {
            loop {
                // ── Snapshots ─────────────────────────────────────────────
                let buf_snapshot = self.buf.clone();
                let cursor_pos   = self.cursor_pos;
                let agent_name   = agent_name.to_string();
                let model        = model.to_string();
                let last_status_snapshot = self.last_status.clone();

                // ── Dynamic resize ────────────────────────────────────────
                // available_w: width available for text after "> " prefix.
                let available_w = self.term_width.saturating_sub(2).max(1);
                let needed_rows = calc_input_rows(&buf_snapshot, available_w)
                    .min(MAX_INPUT_ROWS);
                if needed_rows != current_input_rows {
                    let new_vh = FIXED_ROWS + needed_rows;
                    let (_, th) = terminal::size().unwrap_or((80, 24));
                    // Clear old viewport rows.
                    let old_start = th.saturating_sub(viewport_height);
                    let mut out = io::stdout();
                    for row in old_start..old_start.saturating_add(viewport_height) {
                        let _ = execute!(
                            out,
                            cursor::MoveTo(0, row),
                            terminal::Clear(ClearType::CurrentLine)
                        );
                    }
                    let _ = out.flush();
                    // Re-anchor, pre-scroll, create new terminal.
                    let _ = execute!(io::stdout(), cursor::MoveToRow(th.saturating_sub(1)));
                    {
                        let pre = CrosstermBackend::new(io::stdout());
                        let mut pre_t = Terminal::with_options(
                            pre,
                            TerminalOptions { viewport: Viewport::Inline(0) },
                        )?;
                        let _ = pre_t.insert_before(new_vh, |_| {});
                    }
                    let _ = execute!(io::stdout(), cursor::MoveToRow(th.saturating_sub(1)));
                    let new_backend = CrosstermBackend::new(io::stdout());
                    let new_term = Terminal::with_options(
                        new_backend,
                        TerminalOptions { viewport: Viewport::Inline(new_vh) },
                    )?;
                    // Replacing drops the old terminal.
                    term = new_term;
                    viewport_start_row = th.saturating_sub(new_vh);
                    current_input_rows = needed_rows;
                    viewport_height    = new_vh;
                }

                // ── Draw ──────────────────────────────────────────────────
                let cur_input_rows = current_input_rows;
                term.draw(|frame| {
                    let area = frame.area();

                    let chunks = Layout::default()
                        .direction(Direction::Vertical)
                        .constraints([
                            Constraint::Length(1),              // status
                            Constraint::Length(1),              // top separator
                            Constraint::Length(cur_input_rows), // input (dynamic)
                            Constraint::Length(1),              // bottom separator
                            Constraint::Length(1),              // footer
                        ])
                        .split(area);

                    // ── Status row ────────────────────────────────────────
                    if let Some(ref status) = last_status_snapshot {
                        frame.render_widget(
                            Paragraph::new(Span::styled(
                                status.clone(),
                                Style::default()
                                    .fg(RC::Rgb(100, 170, 120))
                                    .add_modifier(Modifier::DIM),
                            )),
                            chunks[0],
                        );
                    }

                    // ── Separators ────────────────────────────────────────
                    let sep_color = mode_sep_color(mode);
                    let sep = "─".repeat(chunks[1].width as usize);
                    frame.render_widget(
                        Paragraph::new(Span::styled(sep.clone(), Style::default().fg(sep_color))),
                        chunks[1],
                    );
                    frame.render_widget(
                        Paragraph::new(Span::styled(sep, Style::default().fg(sep_color))),
                        chunks[3],
                    );

                    // ── Input area ────────────────────────────────────────
                    let display = if buf_snapshot.is_empty() {
                        Line::from(vec![
                            Span::styled("> ", Style::default().fg(RC::White)),
                            Span::styled(
                                "Type a message…",
                                Style::default().fg(RC::DarkGray),
                            ),
                        ])
                    } else {
                        Line::from(vec![
                            Span::styled("> ", Style::default().fg(RC::White)),
                            Span::styled(
                                buf_snapshot.replace('\n', "↵ "),
                                Style::default().fg(RC::White),
                            ),
                        ])
                    };
                    frame.render_widget(
                        Paragraph::new(display).wrap(Wrap { trim: false }),
                        chunks[2],
                    );

                    // ── Cursor position ───────────────────────────────────
                    let before_cursor = &buf_snapshot[..cursor_pos.min(buf_snapshot.len())];
                    let (vis_row, vis_col) = calc_visual_cursor(before_cursor, available_w);
                    let cursor_col = (chunks[2].x + vis_col)
                        .min(chunks[2].x + chunks[2].width.saturating_sub(1));
                    let cursor_row = (chunks[2].y + vis_row)
                        .min(chunks[2].y + chunks[2].height.saturating_sub(1));
                    frame.set_cursor_position((cursor_col, cursor_row));

                    // ── Footer ────────────────────────────────────────────
                    let (left_label, left_glyph, left_color) = mode_footer_left(mode);
                    let mut footer_spans: Vec<Span> = vec![
                        Span::styled(
                            left_label,
                            Style::default().fg(left_color).add_modifier(Modifier::BOLD),
                        ),
                    ];
                    if !left_glyph.is_empty() {
                        footer_spans.push(Span::styled(
                            format!(" {left_glyph}"),
                            Style::default().fg(left_color),
                        ));
                    }
                    let right_agent = format!("{agent_name}");
                    let right_model = format!(" [{}]", truncate_str(&model, 30));
                    let right_len = (right_agent.chars().count() + right_model.chars().count()) as u16;
                    let left_len: u16 = footer_spans.iter()
                        .map(|s| s.content.chars().count() as u16)
                        .sum();
                    let pad = chunks[4].width.saturating_sub(left_len + right_len) as usize;
                    footer_spans.push(Span::styled(" ".repeat(pad), Style::default()));
                    footer_spans.push(Span::styled(
                        right_agent,
                        Style::default().fg(RC::Rgb(140, 140, 249)),
                    ));
                    footer_spans.push(Span::styled(
                        right_model,
                        Style::default().fg(RC::DarkGray),
                    ));
                    frame.render_widget(
                        Paragraph::new(Line::from(footer_spans)),
                        chunks[4],
                    );
                })?;

                // ── Event ─────────────────────────────────────────────────
                if !event::poll(std::time::Duration::from_millis(50))? {
                    continue;
                }

                match event::read()? {
                    Event::Key(KeyEvent { code, modifiers, .. }) => {
                        match (code, modifiers) {
                            // ── Mode cycling ──────────────────────────────
                            (KeyCode::Tab, _) => {
                                let next = cycle_mode(mode);
                                permissions.set_mode(next);
                                return Ok(Some(String::new()));
                            }
                            (KeyCode::BackTab, _) => {
                                let prev = cycle_mode_back(mode);
                                permissions.set_mode(prev);
                                return Ok(Some(String::new()));
                            }

                            // ── Shift+Enter: insert newline (expands input) ─
                            (KeyCode::Enter, m) if m == KeyModifiers::SHIFT => {
                                self.buf.insert(self.cursor_pos, '\n');
                                self.cursor_pos += 1;
                            }
                            // ── Enter: submit ─────────────────────────────
                            (KeyCode::Enter, _) => {
                                let line = self.buf.clone();
                                return Ok(Some(line));
                            }

                            // ── Exit (Ctrl+D on empty) ────────────────────
                            (KeyCode::Char('d'), KeyModifiers::CONTROL)
                                if self.buf.is_empty() =>
                            {
                                return Ok(None);
                            }

                            // ── Clear (Ctrl+C) ────────────────────────────
                            (KeyCode::Char('c'), KeyModifiers::CONTROL) => {
                                self.buf.clear();
                                self.cursor_pos = 0;
                                return Ok(Some(String::new()));
                            }

                            // ── Esc: clear buffer ─────────────────────────
                            (KeyCode::Esc, _) => {
                                self.buf.clear();
                                self.cursor_pos = 0;
                            }

                            // ── Kill line (Ctrl+U) ────────────────────────
                            (KeyCode::Char('u'), KeyModifiers::CONTROL) => {
                                self.buf.drain(..self.cursor_pos);
                                self.cursor_pos = 0;
                            }

                            // ── Delete word (Ctrl+W) ──────────────────────
                            (KeyCode::Char('w'), KeyModifiers::CONTROL) => {
                                let end = self.cursor_pos;
                                let start = self.buf[..end]
                                    .rfind(|c: char| !c.is_whitespace())
                                    .and_then(|p| {
                                        self.buf[..p].rfind(char::is_whitespace).map(|q| q + 1)
                                    })
                                    .unwrap_or(0);
                                self.buf.drain(start..end);
                                self.cursor_pos = start;
                            }

                            // ── Home / Ctrl+A ─────────────────────────────
                            (KeyCode::Home, _)
                            | (KeyCode::Char('a'), KeyModifiers::CONTROL) => {
                                self.cursor_pos = 0;
                            }

                            // ── End / Ctrl+E ──────────────────────────────
                            (KeyCode::End, _)
                            | (KeyCode::Char('e'), KeyModifiers::CONTROL) => {
                                self.cursor_pos = self.buf.len();
                            }

                            // ── Cursor left ───────────────────────────────
                            (KeyCode::Left, _) if self.cursor_pos > 0 => {
                                self.cursor_pos -= self.buf[..self.cursor_pos]
                                    .chars()
                                    .last()
                                    .map(|c| c.len_utf8())
                                    .unwrap_or(1);
                            }

                            // ── Cursor right ──────────────────────────────
                            (KeyCode::Right, _) if self.cursor_pos < self.buf.len() => {
                                self.cursor_pos += self.buf[self.cursor_pos..]
                                    .chars()
                                    .next()
                                    .map(|c| c.len_utf8())
                                    .unwrap_or(1);
                            }

                            // ── History up ────────────────────────────────
                            (KeyCode::Up, _) if !history.is_empty() => {
                                let new_idx = match *hist_idx {
                                    None => history.len() - 1,
                                    Some(i) if i > 0 => i - 1,
                                    Some(i) => i,
                                };
                                *hist_idx = Some(new_idx);
                                self.buf = history[new_idx].clone();
                                self.cursor_pos = self.buf.len();
                            }

                            // ── History down ──────────────────────────────
                            (KeyCode::Down, _) => {
                                if let Some(i) = *hist_idx {
                                    if i + 1 < history.len() {
                                        *hist_idx = Some(i + 1);
                                        self.buf = history[i + 1].clone();
                                        self.cursor_pos = self.buf.len();
                                    } else {
                                        *hist_idx = None;
                                        self.buf.clear();
                                        self.cursor_pos = 0;
                                    }
                                }
                            }

                            // ── Backspace ─────────────────────────────────
                            (KeyCode::Backspace, _) if self.cursor_pos > 0 => {
                                let char_len = self.buf[..self.cursor_pos]
                                    .chars()
                                    .last()
                                    .map(|c| c.len_utf8())
                                    .unwrap_or(1);
                                self.cursor_pos -= char_len;
                                self.buf.remove(self.cursor_pos);
                            }

                            // ── Delete ────────────────────────────────────
                            (KeyCode::Delete, _)
                                if self.cursor_pos < self.buf.len() =>
                            {
                                self.buf.remove(self.cursor_pos);
                            }

                            // ── Regular character ─────────────────────────
                            (KeyCode::Char(c), mods)
                                if mods == KeyModifiers::NONE
                                    || mods == KeyModifiers::SHIFT =>
                            {
                                self.buf.insert(self.cursor_pos, c);
                                self.cursor_pos += c.len_utf8();
                            }

                            _ => {}
                        }
                    }

                    Event::Resize(w, _) => {
                        self.term_width = w;
                    }

                    _ => {}
                }
            }
        })();

        drop(_raw);
        drop(term);
        // Clear ALL viewport rows so no stale prompt text lingers.
        let mut out = io::stdout();
        for row in viewport_start_row..viewport_start_row.saturating_add(viewport_height) {
            let _ = execute!(out, cursor::MoveTo(0, row), terminal::Clear(ClearType::CurrentLine));
        }
        let (_, final_term_h) = terminal::size().unwrap_or((80, 24));
        let _ = execute!(out, cursor::MoveToRow(final_term_h.saturating_sub(1)));
        let _ = out.flush();
        result
    }
}

impl Default for InputWidget {
    fn default() -> Self {
        Self::new()
    }
}

// ── Helpers ───────────────────────────────────────────────────────────────────

/// Calculate how many visual rows `buf` needs given `available_width` columns
/// for text (after the "> " prefix on the first logical line).
///
/// Capped at [`MAX_INPUT_ROWS`].
fn calc_input_rows(buf: &str, available_width: u16) -> u16 {
    let w = available_width.max(1) as usize;
    if buf.is_empty() {
        return 1;
    }
    let mut total: u16 = 0;
    for (i, line) in buf.split('\n').enumerate() {
        let chars = line.chars().count();
        // First logical line: 2 cols taken by "> " prefix.
        let row_w = if i == 0 { w.saturating_sub(2) } else { w }.max(1);
        let rows = if chars == 0 { 1 } else { ((chars + row_w - 1) / row_w) as u16 };
        total += rows;
    }
    total.max(1).min(MAX_INPUT_ROWS)
}

/// Return `(visual_row, visual_col)` of the cursor within the input area.
///
/// Correctly handles:
/// - The `"> "` prefix (2 columns) on the very first visual row only.
/// - `\n` characters (Shift+Enter) as explicit row breaks.
/// - Visual wrapping when a line's length exceeds the available width.
fn calc_visual_cursor(before_cursor: &str, available_width: u16) -> (u16, u16) {
    let w = available_width.max(1) as usize;
    let first_row_w = w.saturating_sub(2).max(1);

    let mut vis_row: u16 = 0;
    let mut vis_col: u16 = 2; // starts right after "> "
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
                // Visual wrap.
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
        PermissionMode::Default           => ("Press / for commands", "", RC::Rgb(70, 72, 74)),
        PermissionMode::AcceptEdits       => ("accept edits", "⏵⏵", RC::Rgb(140, 140, 249)),
        PermissionMode::Plan              => ("plan mode", "⏸", RC::Green),
        PermissionMode::BypassPermissions => ("bypass (allow all)", "⚡", RC::Red),
    }
}

fn cycle_mode(mode: PermissionMode) -> PermissionMode {
    match mode {
        PermissionMode::Default           => PermissionMode::AcceptEdits,
        PermissionMode::AcceptEdits       => PermissionMode::Plan,
        PermissionMode::Plan              => PermissionMode::BypassPermissions,
        PermissionMode::BypassPermissions => PermissionMode::Default,
    }
}

fn cycle_mode_back(mode: PermissionMode) -> PermissionMode {
    match mode {
        PermissionMode::Default           => PermissionMode::BypassPermissions,
        PermissionMode::AcceptEdits       => PermissionMode::Default,
        PermissionMode::Plan              => PermissionMode::AcceptEdits,
        PermissionMode::BypassPermissions => PermissionMode::Plan,
    }
}

fn truncate_str(s: &str, max: usize) -> String {
    let chars: Vec<char> = s.chars().collect();
    if chars.len() <= max {
        s.to_string()
    } else {
        format!("{}…", chars[..max.saturating_sub(1)].iter().collect::<String>())
    }
}
