/// Ratatui-based input widget for the CADE REPL.
///
/// Layout matches Letta Code's InputRich component (Viewport::Inline(4)):
/// ```
///  ─────────────────────────────────────────────── (dim separator, full width)
///  > user types here                               (prompt char + text field)
///  ─────────────────────────────────────────────── (dim separator, full width)
///  plan (read-only) mode ⏸       AgentName [model] (footer: left=mode, right=agent/model)
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
    widgets::{Paragraph, Wrap},
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
    /// Completion summary shown in the status row after the agent finishes a turn.
    /// Example: "✻ Considered for 3s · ↑1200 ↓800 tokens"
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
        _in_tokens: u64,
        _out_tokens: u64,
    ) -> Result<Option<String>> {
        self.buf.clear();
        self.cursor_pos = 0;

        // 5 rows: status | separator | input-line | separator | footer
        let viewport_height: u16 = 5;
        // Anchor to terminal bottom, then pre-scroll `viewport_height` rows upward
        // so the input widget never overwrites the last lines of agent output.
        //
        // Strategy:
        //  1. MoveToRow(term_h-1) — anchor without printing.
        //  2. Create a zero-height viewport and call insert_before(viewport_height)
        //     with an empty render.  This scrolls content up by viewport_height rows
        //     and leaves viewport_height blank rows at the terminal bottom.
        //  3. Re-anchor, then create the real Viewport::Inline(viewport_height)
        //     which renders the input widget into those blank rows.
        let (_, term_h) = terminal::size().unwrap_or((80, 24));
        let _ = execute!(io::stdout(), cursor::MoveToRow(term_h.saturating_sub(1)));
        {
            let pre_backend = CrosstermBackend::new(io::stdout());
            let mut pre_term = Terminal::with_options(
                pre_backend,
                TerminalOptions { viewport: Viewport::Inline(0) },
            )?;
            let _ = pre_term.insert_before(viewport_height, |_buf| {});
        }
        // Re-anchor after the pre-scroll.
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
                let agent_name = agent_name.to_string();
                let model = model.to_string();

                let last_status_snapshot = self.last_status.clone();
                term.draw(|frame| {
                    let area = frame.area();

                    // 5-row layout:
                    //  row 0: status line (completion summary or blank)
                    //  row 1: top separator
                    //  row 2: prompt + text input
                    //  row 3: bottom separator
                    //  row 4: footer (mode | agent [model])
                    let chunks = Layout::default()
                        .direction(Direction::Vertical)
                        .constraints([
                            Constraint::Length(1), // row 0: status
                            Constraint::Length(1), // row 1: separator
                            Constraint::Length(1), // row 2: prompt + text
                            Constraint::Length(1), // row 3: separator
                            Constraint::Length(1), // row 4: footer
                        ])
                        .split(area);

                    // ── Status row (row 0) ────────────────────────────────────
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

                    // ── Separators (rows 1 and 3) ─────────────────────────────
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

                    // ── Prompt + text input (row 2) ───────────────────────────
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

                    // Cursor: col = 2 ("> " prefix) + col in current line
                    let before_cursor = &buf_snapshot[..cursor_pos.min(buf_snapshot.len())];
                    let line_idx = before_cursor.chars().filter(|&c| c == '\n').count() as u16;
                    let last_line_start = before_cursor.rfind('\n').map(|i| i + 1).unwrap_or(0);
                    let col_in_line = before_cursor[last_line_start..].chars().count() as u16;
                    let cursor_col = chunks[2].x + 2 + col_in_line.min(chunks[2].width.saturating_sub(3));
                    let cursor_row = (chunks[2].y + line_idx).min(chunks[2].y + chunks[2].height.saturating_sub(1));
                    frame.set_cursor_position((cursor_col, cursor_row));

                    // ── Footer (row 4) ────────────────────────────────────────
                    // Left: mode info (or "Press / for commands" for Default)
                    // Right: AgentName [model]
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

                    // Right side: "AgentName [model]"
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

                // ── Event ─────────────────────────────────────────────────────
                if !event::poll(std::time::Duration::from_millis(50))? {
                    continue;
                }

                match event::read()? {
                    Event::Key(KeyEvent { code, modifiers, .. }) => {
                        match (code, modifiers) {
                            // ── Mode cycling ──────────────────────────────────
                            // Tab = forward cycle, Shift+Tab = backward cycle.
                            // Both signal the caller via empty return so the REPL
                            // re-reads the mode from PermissionManager on the next
                            // input_widget.read() call.
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
        // Clear ALL 4 input-widget rows so no stale prompt text lingers.
        // The turn separator is now rendered as the first line of user_message()
        // via with_insert_before, keeping it visually consistent with all other
        // output and eliminating blank rows between the separator and content.
        let mut out = io::stdout();
        for row in viewport_start_row..viewport_start_row.saturating_add(viewport_height) {
            let _ = execute!(out, cursor::MoveTo(0, row), terminal::Clear(ClearType::CurrentLine));
        }
        // Position cursor at terminal bottom so with_insert_before always works
        // correctly on the next output call.
        let _ = execute!(out, cursor::MoveToRow(term_h.saturating_sub(1)));
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

/// Separator line color — matches Letta Code's bash-mode red vs default dim.
fn mode_sep_color(mode: PermissionMode) -> RC {
    // Separator adopts the mode accent color (Letta Code: input.border = textDisabled for default,
    // otherwise the mode's accent color as a subtle visual hint)
    match mode {
        PermissionMode::Default           => RC::Rgb(70, 72, 74),   // #46484A textDisabled
        PermissionMode::AcceptEdits       => RC::Rgb(140, 140, 249), // #8C8CF9 purple
        PermissionMode::Plan              => RC::Green,
        PermissionMode::BypassPermissions => RC::Red,
    }
}

/// Footer left side: (label, glyph, color).
/// Default mode → "Press / for commands" (dim gray).
/// Other modes → mode name + glyph in their accent color.
fn mode_footer_left(mode: PermissionMode) -> (&'static str, &'static str, RC) {
    // Colors + glyphs matching Letta Code exactly (colors.ts)
    match mode {
        // Default: dim border color, hint text — no glyph
        PermissionMode::Default           => ("Press / for commands", "", RC::Rgb(70, 72, 74)),
        // AcceptEdits: purple #8C8CF9 + ⏵⏵ (double-play symbol)
        PermissionMode::AcceptEdits       => ("accept edits", "⏵⏵", RC::Rgb(140, 140, 249)),
        // Plan: green + ⏸ (pause)
        PermissionMode::Plan              => ("plan mode", "⏸", RC::Green),
        // Bypass: red + ⚡ (lightning)
        PermissionMode::BypassPermissions => ("bypass (allow all)", "⚡", RC::Red),
    }
}

/// Forward mode cycle: Default → AcceptEdits → Plan → BypassPermissions → Default.
fn cycle_mode(mode: PermissionMode) -> PermissionMode {
    match mode {
        PermissionMode::Default           => PermissionMode::AcceptEdits,
        PermissionMode::AcceptEdits       => PermissionMode::Plan,
        PermissionMode::Plan              => PermissionMode::BypassPermissions,
        PermissionMode::BypassPermissions => PermissionMode::Default,
    }
}

/// Backward mode cycle (Shift+Tab).
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
