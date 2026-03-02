/// Ratatui-based input widget for the CADE REPL.
///
/// Replaces the raw-crossterm `read_line()` with a proper inline ratatui
/// input box that shows:
///   - A separator line
///   - A bordered input area with mode indicator
///   - A status line: model · agent · [mode] · tokens
///
/// Layout (Viewport::Inline(4)):
/// ```
///  ─────────────────────────────────────────────── (separator)
///  ╭─ cade [yolo] ──────────────────────────────╮
///  │ > Type a message…  (Shift+Enter for newline) │
///  ╰─────────────────────────────────────────────╯
///   claude-opus · agent-xyz · in: 1,234  out: 567
/// ```

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
    widgets::{Block, Borders, Paragraph, Wrap},
};

use crate::permissions::PermissionMode;
use crate::ui::output::CONTENT_PAD;

// ── RawModeGuard ──────────────────────────────────────────────────────────────

/// RAII guard that enables raw mode on construction and disables it on drop.
///
/// Replaces the scattered `enable_raw_mode()` / `disable_raw_mode()` call pairs
/// across repl.rs and input.rs. Guarantees the terminal is always restored even
/// if code paths panic or early-return between enable/disable.
pub struct RawModeGuard;

impl RawModeGuard {
    /// Enable raw mode and return the guard.
    /// The terminal will be restored to cooked mode when the guard is dropped.
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
    /// Current input buffer.
    buf: String,
    /// Cursor position (byte offset into `buf`).
    cursor_pos: usize,
    /// Terminal width for drawing.
    term_width: u16,
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
        }
    }

    /// Refresh terminal width (call on resize events).
    pub fn update_width(&mut self) {
        if let Ok((w, _)) = crossterm::terminal::size() {
            self.term_width = w;
        }
    }

    /// Show the ratatui input box and read one user message.
    ///
    /// Returns `None` on Ctrl+D (exit signal).
    pub fn read(
        &mut self,
        history: &mut Vec<String>,
        hist_idx: &mut Option<usize>,
        mode: PermissionMode,
        permissions: &crate::permissions::PermissionManager,
        agent_name: &str,
        model: &str,
        in_tokens: u64,
        out_tokens: u64,
    ) -> Result<Option<String>> {
        self.buf.clear();
        self.cursor_pos = 0;

        // 4 rows: separator + top-border + input-line + bottom-border
        // Plus 1 for status line = 5 total
        let viewport_height: u16 = 5;
        // Anchor the cursor to the terminal bottom before creating the viewport.
        //
        // with_insert_before does this same anchor on every call, so on turns 2+
        // the cursor is already there. But on the FIRST call (right after the banner),
        // the cursor is wherever the banner left it (mid-screen). Without anchoring,
        // Viewport::Inline(5) renders mid-screen and old terminal content from the
        // previous session is visible below the input box.
        //
        // Once anchored, Viewport::Inline(5) always lands at exactly term_h-5..term_h-1
        // and viewport_start_row is deterministic — no need to call cursor::position().
        let (_, term_h) = terminal::size().unwrap_or((80, 24));
        let _ = execute!(io::stdout(), cursor::MoveToRow(term_h.saturating_sub(1)));
        let viewport_start_row = term_h.saturating_sub(viewport_height);
        let backend = CrosstermBackend::new(io::stdout());
        let mut term = Terminal::with_options(
            backend,
            TerminalOptions {
                viewport: Viewport::Inline(viewport_height),
            },
        )?;

        let _raw = RawModeGuard::enable()?;

        let result: Result<Option<String>> = (|| {
            loop {
                // ── Draw ──────────────────────────────────────────────────────
                let buf_snapshot = self.buf.clone();
                let cursor_pos = self.cursor_pos;
                let mode_tag = mode_title(mode);
                let agent_name = agent_name.to_string();
                let model = model.to_string();

                term.draw(|frame| {
                    let area = frame.area();

                    // Horizontal padding: shrink content area by CONTENT_PAD on each side
                    let cols = Layout::default()
                        .direction(Direction::Horizontal)
                        .constraints([
                            Constraint::Length(CONTENT_PAD),
                            Constraint::Min(0),
                            Constraint::Length(CONTENT_PAD),
                        ])
                        .split(area);

                    let chunks = Layout::default()
                        .direction(Direction::Vertical)
                        .constraints([
                            Constraint::Length(1), // separator
                            Constraint::Length(3), // input box (border + 1 content row)
                            Constraint::Length(1), // status bar
                        ])
                        .split(cols[1]);

                    // ── Separator ─────────────────────────────────────────────
                    let sep = "─".repeat(chunks[0].width as usize);
                    let sep_para = Paragraph::new(Span::styled(
                        sep,
                        Style::default().fg(RC::DarkGray),
                    ));
                    frame.render_widget(sep_para, chunks[0]);

                    // ── Input box ─────────────────────────────────────────────
                    let border_color = mode_color(mode);
                    let block = Block::default()
                        .borders(Borders::ALL)
                        .border_style(Style::default().fg(border_color))
                        .title(Span::styled(
                            format!(" cade{mode_tag} "),
                            Style::default()
                                .fg(border_color)
                                .add_modifier(Modifier::BOLD),
                        ));
                    let inner = block.inner(chunks[1]);

                    // Show placeholder when empty
                    let display = if buf_snapshot.is_empty() {
                        Line::from(vec![
                            Span::raw("> "),
                            Span::styled(
                                "Type a message…  (Shift+Enter for newline)",
                                Style::default().fg(RC::DarkGray),
                            ),
                        ])
                    } else {
                        Line::from(vec![
                            Span::raw("> "),
                            Span::styled(
                                buf_snapshot.replace('\n', "↵ "),
                                Style::default().fg(RC::White),
                            ),
                        ])
                    };

                    let input_para = Paragraph::new(display)
                        .wrap(Wrap { trim: false });
                    frame.render_widget(block, chunks[1]);
                    frame.render_widget(input_para, inner);

                    // Cursor position: after "> " prefix + cursor_pos chars
                    // For multi-line input, count newlines before cursor_pos to find the row.
                    let before_cursor = &buf_snapshot[..cursor_pos.min(buf_snapshot.len())];
                    let line_idx = before_cursor.chars().filter(|&c| c == '\n').count() as u16;
                    let last_line_start = before_cursor.rfind('\n').map(|i| i + 1).unwrap_or(0);
                    let col_in_line = before_cursor[last_line_start..].chars().count() as u16;
                    let cursor_col = inner.x + 2 + col_in_line.min(inner.width.saturating_sub(3));
                    let cursor_row = (inner.y + line_idx).min(inner.y + inner.height.saturating_sub(1));
                    frame.set_cursor_position((cursor_col, cursor_row));

                    // ── Status bar ────────────────────────────────────────────
                    let (mode_icon, mode_label) = mode_status(mode);
                    let tok_in = fmt_tokens(in_tokens);
                    let tok_out = fmt_tokens(out_tokens);
                    let status = Line::from(vec![
                        Span::styled(
                            format!(" {model}"),
                            Style::default().fg(RC::Cyan),
                        ),
                        Span::styled(" · ", Style::default().fg(RC::DarkGray)),
                        Span::styled(
                            agent_name.clone(),
                            Style::default().fg(RC::DarkGray),
                        ),
                        Span::styled(
                            format!("  {mode_icon} {mode_label}"),
                            Style::default()
                                .fg(border_color)
                                .add_modifier(Modifier::BOLD),
                        ),
                        Span::styled(
                            format!("  in:{tok_in}  out:{tok_out}"),
                            Style::default().fg(RC::DarkGray),
                        ),
                        Span::styled(
                            "  Shift+Tab: cycle mode",
                            Style::default().fg(RC::DarkGray),
                        ),
                    ]);
                    frame.render_widget(
                        Paragraph::new(status),
                        chunks[2],
                    );
                })?;

                // ── Event ─────────────────────────────────────────────────────
                if !event::poll(std::time::Duration::from_millis(50))? {
                    continue;
                }

                match event::read()? {
                    Event::Key(KeyEvent { code, modifiers, .. }) => {
                        match (code, modifiers) {
                            // ── Mode cycling (Shift+Tab) ──────────────────────
                            (KeyCode::BackTab, _) => {
                                let next = cycle_mode(mode);
                                permissions.set_mode(next);
                                // Re-enter loop with updated mode (drawn next frame)
                                // We can't mutate `mode` here (it's captured by value),
                                // so we return a sentinel to the caller — but instead we
                                // just continue; the caller will re-call read() if needed.
                                // For now, reflect mode via a re-draw on next iteration.
                                // The caller passes the current mode; after mode change the
                                // next redraw iteration will use the new mode automatically
                                // since the mode comes from permissions.mode() in the caller.
                                // This works because mode is re-read by the caller each turn.
                                // Signal via an empty input (caller skips empty lines):
                                return Ok(Some(String::new()));
                            }

                            // ── Submit (Enter) ────────────────────────────────
                            (KeyCode::Enter, m) if m == KeyModifiers::SHIFT => {
                                self.buf.insert(self.cursor_pos, '\n');
                                self.cursor_pos += 1;
                            }
                            (KeyCode::Enter, _) => {
                                let line = self.buf.clone();
                                return Ok(Some(line));
                            }

                            // ── Exit (Ctrl+D on empty) ────────────────────────
                            (KeyCode::Char('d'), KeyModifiers::CONTROL)
                                if self.buf.is_empty() =>
                            {
                                return Ok(None);
                            }

                            // ── Clear (Ctrl+C) ────────────────────────────────
                            (KeyCode::Char('c'), KeyModifiers::CONTROL) => {
                                self.buf.clear();
                                self.cursor_pos = 0;
                                return Ok(Some(String::new()));
                            }

                            // ── Esc: clear buffer ─────────────────────────────
                            (KeyCode::Esc, _) => {
                                self.buf.clear();
                                self.cursor_pos = 0;
                            }

                            // ── Kill line (Ctrl+U) ────────────────────────────
                            (KeyCode::Char('u'), KeyModifiers::CONTROL) => {
                                self.buf.drain(..self.cursor_pos);
                                self.cursor_pos = 0;
                            }

                            // ── Delete word (Ctrl+W) ──────────────────────────
                            (KeyCode::Char('w'), KeyModifiers::CONTROL) => {
                                // Find start of previous word
                                let end = self.cursor_pos;
                                let start = self.buf[..end]
                                    .rfind(|c: char| !c.is_whitespace())
                                    .and_then(|p| self.buf[..p].rfind(char::is_whitespace).map(|q| q + 1))
                                    .unwrap_or(0);
                                self.buf.drain(start..end);
                                self.cursor_pos = start;
                            }

                            // ── Home / Ctrl+A ─────────────────────────────────
                            (KeyCode::Home, _)
                            | (KeyCode::Char('a'), KeyModifiers::CONTROL) => {
                                self.cursor_pos = 0;
                            }

                            // ── End / Ctrl+E ──────────────────────────────────
                            (KeyCode::End, _)
                            | (KeyCode::Char('e'), KeyModifiers::CONTROL) => {
                                self.cursor_pos = self.buf.len();
                            }

                            // ── Cursor left ───────────────────────────────────
                            (KeyCode::Left, _) if self.cursor_pos > 0 => {
                                // Move one char (UTF-8 safe)
                                self.cursor_pos -= self.buf[..self.cursor_pos]
                                    .chars()
                                    .last()
                                    .map(|c| c.len_utf8())
                                    .unwrap_or(1);
                            }

                            // ── Cursor right ──────────────────────────────────
                            (KeyCode::Right, _) if self.cursor_pos < self.buf.len() => {
                                self.cursor_pos += self.buf[self.cursor_pos..]
                                    .chars()
                                    .next()
                                    .map(|c| c.len_utf8())
                                    .unwrap_or(1);
                            }

                            // ── History up ────────────────────────────────────
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

                            // ── History down ──────────────────────────────────
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

                            // ── Backspace ─────────────────────────────────────
                            (KeyCode::Backspace, _) if self.cursor_pos > 0 => {
                                let char_len = self.buf[..self.cursor_pos]
                                    .chars()
                                    .last()
                                    .map(|c| c.len_utf8())
                                    .unwrap_or(1);
                                self.cursor_pos -= char_len;
                                self.buf.remove(self.cursor_pos);
                            }

                            // ── Delete ────────────────────────────────────────
                            (KeyCode::Delete, _)
                                if self.cursor_pos < self.buf.len() =>
                            {
                                self.buf.remove(self.cursor_pos);
                            }

                            // ── Regular character ─────────────────────────────
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

        drop(_raw); // restore cooked mode before returning
        // Drop the ratatui terminal before touching the cursor so its internal
        // state is released first.
        drop(term);
        // Selectively clear the input box rows:
        //   Row 0 (viewport_start_row)     = separator ─────  → KEEP as turn divider
        //   Rows 1-4 (box borders+content) = ┌──┐ content └──┘ + status → CLEAR
        //
        // If we cleared all 5 rows (including the separator), the history would
        // have blank gaps between responses. Keeping the separator gives a clean
        // visual divider between conversation turns while hiding the box borders.
        let mut out = io::stdout();
        for row in (viewport_start_row + 1)..viewport_start_row.saturating_add(viewport_height) {
            let _ = execute!(out, cursor::MoveTo(0, row), terminal::Clear(ClearType::CurrentLine));
        }
        // Leave cursor on the separator row; with_insert_before re-anchors to
        // terminal bottom on every call so this position doesn't need to be exact.
        let _ = execute!(out, cursor::MoveTo(0, viewport_start_row));
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

fn mode_title(mode: PermissionMode) -> &'static str {
    match mode {
        PermissionMode::Default           => "",
        PermissionMode::AcceptEdits       => " [edits]",
        PermissionMode::Plan              => " [plan]",
        PermissionMode::BypassPermissions => " [yolo]",
    }
}

fn mode_color(mode: PermissionMode) -> RC {
    match mode {
        PermissionMode::Default           => RC::Green,
        PermissionMode::AcceptEdits       => RC::Magenta,
        PermissionMode::Plan              => RC::Cyan,
        PermissionMode::BypassPermissions => RC::Yellow,
    }
}

fn mode_status(mode: PermissionMode) -> (&'static str, &'static str) {
    match mode {
        PermissionMode::Default           => ("✅", "default"),
        PermissionMode::AcceptEdits       => ("📝", "acceptEdits"),
        PermissionMode::Plan              => ("📖", "plan"),
        PermissionMode::BypassPermissions => ("⚡", "yolo"),
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

fn fmt_tokens(n: u64) -> String {
    if n == 0 {
        "0".to_string()
    } else if n >= 1_000_000 {
        format!("{:.1}M", n as f64 / 1_000_000.0)
    } else if n >= 1_000 {
        format!("{:.1}k", n as f64 / 1_000.0)
    } else {
        n.to_string()
    }
}
