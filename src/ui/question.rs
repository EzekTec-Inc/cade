/// Interactive question/selection widget for CADE.
///
/// Used by both the `ask_user_question` tool handler and the tool-approval dialog.
///
/// # Layout (single-select example)
/// ```text
/// ──────────────────────────────────────────  ← full-width separator (DarkGray)
/// Auth method                                 ← header chip (Bold White)
///                                             ← blank
/// Which auth method should we use?            ← question text (White)
///                                             ← blank
/// Question 1 of 2                             ← progress (DarkGray) — when progress is Some
///                                             ← blank
/// ❯ 1. JWT Tokens                             ← selected: ❯ Green + number + label Bold White
///      Use stateless JWT for all API routes   ← description (5-space indent, DarkGray)
///   2. Session Cookies                        ← unselected (2-space indent, label White)
///      Traditional cookie-based sessions      ← description (5-space indent, DarkGray)
///   3. Type something.█                       ← custom text option (DarkGray Italic)
///
/// Enter to select · ↑↓ navigate · 1-N quick select · Esc to cancel
/// ```
///
/// # Layout (multi-select example)
/// ```text
///   1. [✓] JWT Tokens           ← checked (Green checkbox)
///   2. [ ] Session Cookies      ← unchecked
/// ❯ 3.    Submit                ← Submit item (Bold, Green when selected)
///
/// Enter to toggle · ↑↓ navigate · Enter on Submit to confirm · Esc to cancel
/// ```

use std::io::{self, Write};

use anyhow::Result;
use crossterm::{
    cursor,
    event::{self, Event, KeyCode, KeyEvent, KeyModifiers},
    execute,
    terminal::{self, ClearType, EnterAlternateScreen, LeaveAlternateScreen},
};
use ratatui::{
    Terminal, TerminalOptions, Viewport,
    backend::CrosstermBackend,
    layout::{Constraint, Direction, Layout},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::Paragraph,
};

use super::input::RawModeGuard;

// ── Public types ──────────────────────────────────────────────────────────────

/// One labelled option in a question.
#[derive(Debug, Clone)]
pub struct QuestionOption {
    pub label: String,
    pub description: String,
}

/// A single question to present to the user.
#[derive(Debug)]
pub struct Question<'a> {
    /// Short chip/tag label shown above the question text (≤12 chars).
    pub header: &'a str,
    /// Full question text.
    pub text: &'a str,
    /// 2–N answer options.
    pub options: &'a [QuestionOption],
    /// If true, checkboxes are shown and a "Submit" option is appended.
    pub multi_select: bool,
    /// Append a free-text "Type something." option (omit for approval dialogs).
    pub allow_other: bool,
    /// Optional progress indicator `(current, total)` — shows "Question N of M".
    pub progress: Option<(usize, usize)>,
}

/// The answer returned by the widget.
#[derive(Debug, Clone)]
pub enum QuestionAnswer {
    /// Single option selected (label or custom typed text).
    Single(String),
    /// Multiple options selected (multi-select mode).
    Multi(Vec<String>),
}

impl QuestionAnswer {
    /// Flat string representation (multi joined with ", ").
    pub fn as_str(&self) -> String {
        match self {
            Self::Single(s)  => s.clone(),
            Self::Multi(v)   => v.join(", "),
        }
    }
}

// ── QuestionWidget ────────────────────────────────────────────────────────────

pub struct QuestionWidget;

impl QuestionWidget {
    /// Present `question` interactively.
    ///
    /// Returns `Some(answer)` on submission or `None` if the user pressed Esc / Ctrl+C.
    ///
    /// The widget renders in the **alternate screen buffer** so the main screen
    /// (content, ThinkingBar, etc.) is restored exactly when the modal closes — no
    /// blank rows are created and no blank-row bookkeeping is required from the caller.
    pub fn ask(question: &Question<'_>) -> Result<Option<QuestionAnswer>> {
        // ── Build the effective options list ──────────────────────────────────
        // Slots: provided options + optional "Type something." + optional "Submit"
        let n_real = question.options.len();
        let has_other  = question.allow_other;
        let has_submit = question.multi_select;
        // Total selectable items in the list
        let total_items = n_real + usize::from(has_other) + usize::from(has_submit);

        // Helper: index of "Type something." option (-1 if absent)
        let other_idx  = if has_other  { n_real }                    else { usize::MAX };
        // Helper: index of "Submit" option (-1 if absent)
        let submit_idx = if has_submit { n_real + usize::from(has_other) } else { usize::MAX };

        // ── State ─────────────────────────────────────────────────────────────
        let mut cursor_pos: usize = 0;
        let mut custom_text: String = String::new();
        // Multi-select: which real option indices are checked
        let mut checked: Vec<bool> = vec![false; n_real];

        // ── Viewport height calculation ───────────────────────────────────────
        // separator(1) + header(1) + blank(1) + question(1) + blank(1)
        // + progress(1) + blank(1) if progress is Some, else 0
        // + options * 2 (label + description)
        // + "Submit" item(1) if multi_select
        // + blank(1) + footer(1)
        let progress_rows = if question.progress.is_some() { 2 } else { 0 };
        let option_rows   = total_items * 2;
        let viewport_height = (1 + 1 + 1 + 1 + 1 + progress_rows + option_rows + 1 + 1) as u16;

        // ── Terminal setup ─────────────────────────────────────────────────────
        // Switch to the alternate screen buffer.  The main screen (content rows,
        // ThinkingBar, cursor position) is frozen and will be restored exactly by
        // LeaveAlternateScreen — no blank rows are created in the main screen.
        let mut out = io::stdout();
        execute!(out, EnterAlternateScreen)?;

        let (_, init_h) = terminal::size().unwrap_or((80, 24));
        // Position the cursor at the bottom; Viewport::Inline will scroll the
        // (blank) alternate screen to create the viewport without disturbing anything.
        let _ = execute!(out, cursor::MoveToRow(init_h.saturating_sub(1)));

        let backend = CrosstermBackend::new(io::stdout());
        let mut term = Terminal::with_options(
            backend,
            TerminalOptions { viewport: Viewport::Inline(viewport_height) },
        )?;

        let _raw = RawModeGuard::enable()?;

        let answer: Option<QuestionAnswer> = 'widget: loop {
            // ── Draw ──────────────────────────────────────────────────────────
            let (term_w, _) = terminal::size().unwrap_or((80, 24));
            let sep = "─".repeat(term_w as usize);

            term.draw(|frame| {
                let area = frame.area();
                // Build all lines
                let mut lines: Vec<Line<'static>> = Vec::new();

                // Separator
                lines.push(Line::from(Span::styled(sep.clone(), Style::default().fg(Color::DarkGray))));

                // Header chip
                lines.push(Line::from(Span::styled(
                    question.header.to_string(),
                    Style::default().fg(Color::White).add_modifier(Modifier::BOLD),
                )));

                // Blank
                lines.push(Line::from(""));

                // Question text
                lines.push(Line::from(Span::styled(
                    question.text.to_string(),
                    Style::default().fg(Color::White),
                )));

                // Blank
                lines.push(Line::from(""));

                // Progress indicator
                if let Some((cur, tot)) = question.progress {
                    lines.push(Line::from(Span::styled(
                        format!("Question {cur} of {tot}"),
                        Style::default().fg(Color::DarkGray),
                    )));
                    lines.push(Line::from(""));
                }

                // Options
                for idx in 0..total_items {
                    let is_selected = cursor_pos == idx;
                    let selector = if is_selected { "❯" } else { " " };

                    if idx == submit_idx {
                        // "Submit" row
                        let label_style = if is_selected {
                            Style::default().fg(Color::Green).add_modifier(Modifier::BOLD)
                        } else {
                            Style::default().fg(Color::DarkGray)
                        };
                        lines.push(Line::from(vec![
                            Span::styled(
                                format!("{selector} {idx_num}.    Submit",
                                    idx_num = idx + 1),
                                label_style,
                            ),
                        ]));
                        lines.push(Line::from(""));
                        continue;
                    }

                    if idx == other_idx {
                        // "Type something." row
                        let display = if cursor_pos == idx {
                            if custom_text.is_empty() {
                                format!("Type something.█")
                            } else {
                                format!("{}█", custom_text)
                            }
                        } else if !custom_text.is_empty() {
                            custom_text.clone()
                        } else {
                            "Type something.".to_string()
                        };
                        let other_style = Style::default()
                            .fg(Color::DarkGray)
                            .add_modifier(Modifier::ITALIC);
                        lines.push(Line::from(vec![
                            Span::styled(selector.to_string(), Style::default().fg(Color::Green)),
                            Span::styled(
                                format!(" {}.    {display}", idx + 1),
                                other_style,
                            ),
                        ]));
                        lines.push(Line::from(""));
                        continue;
                    }

                    // Regular option
                    let opt = &question.options[idx];

                    // Checkbox prefix for multi-select
                    let checkbox = if question.multi_select {
                        if checked[idx] { "[✓] " } else { "[ ] " }
                    } else {
                        ""
                    };

                    let label_style = if is_selected {
                        Style::default().fg(Color::White).add_modifier(Modifier::BOLD)
                    } else {
                        Style::default().fg(Color::White)
                    };
                    let num_style = if is_selected {
                        Style::default().fg(Color::Green)
                    } else {
                        Style::default().fg(Color::DarkGray)
                    };

                    lines.push(Line::from(vec![
                        Span::styled(selector.to_string(), Style::default().fg(Color::Green)),
                        Span::styled(format!(" {}. ", idx + 1), num_style),
                        Span::styled(checkbox.to_string(), Style::default().fg(Color::Green)),
                        Span::styled(opt.label.clone(), label_style),
                    ]));
                    lines.push(Line::from(Span::styled(
                        format!("     {}", opt.description),
                        Style::default().fg(Color::DarkGray),
                    )));
                }

                // Footer hint
                let hint = if question.multi_select {
                    "Enter to toggle · ↑↓ navigate · Enter on Submit to confirm · Esc to cancel"
                } else {
                    "Enter to select · ↑↓ navigate · 1-N quick select · Esc to cancel"
                };
                lines.push(Line::from(Span::styled(
                    hint.to_string(),
                    Style::default().fg(Color::DarkGray).add_modifier(Modifier::DIM),
                )));

                // Render all lines into the viewport
                let constraints: Vec<Constraint> = lines.iter()
                    .map(|_| Constraint::Length(1))
                    .collect();
                let chunks = Layout::default()
                    .direction(Direction::Vertical)
                    .constraints(constraints)
                    .split(area);
                for (i, line) in lines.into_iter().enumerate() {
                    if i < chunks.len() {
                        frame.render_widget(Paragraph::new(line), chunks[i]);
                    }
                }
            })?;

            // ── Event ──────────────────────────────────────────────────────────
            if !event::poll(std::time::Duration::from_millis(50))? {
                continue;
            }

            match event::read()? {
                Event::Key(KeyEvent { code, modifiers, .. }) => {
                    match (code, modifiers) {
                        // Cancel
                        (KeyCode::Esc, _)
                        | (KeyCode::Char('c'), KeyModifiers::CONTROL) => {
                            break 'widget None;
                        }

                        // Navigation up
                        (KeyCode::Up, _) => {
                            if cursor_pos > 0 {
                                cursor_pos -= 1;
                            }
                        }

                        // Navigation down
                        (KeyCode::Down, _) => {
                            if cursor_pos + 1 < total_items {
                                cursor_pos += 1;
                            }
                        }

                        // Tab forward
                        (KeyCode::Tab, _) => {
                            cursor_pos = (cursor_pos + 1) % total_items;
                        }

                        // Number quick-select (1–9)
                        (KeyCode::Char(c), KeyModifiers::NONE)
                            if c.is_ascii_digit() && c != '0' =>
                        {
                            let n = (c as usize) - ('0' as usize);
                            let idx = n.saturating_sub(1);
                            if idx < total_items {
                                if question.multi_select {
                                    // Toggle checkbox if it's a real option
                                    if idx < n_real {
                                        checked[idx] = !checked[idx];
                                        cursor_pos = idx;
                                    }
                                } else {
                                    // Immediate submit
                                    cursor_pos = idx;
                                    if idx == other_idx {
                                        // Move to other but don't submit yet
                                    } else {
                                        let label = question.options[idx].label.clone();
                                        break 'widget Some(QuestionAnswer::Single(label));
                                    }
                                }
                            }
                        }

                        // Backspace (custom text input)
                        (KeyCode::Backspace, _) if cursor_pos == other_idx => {
                            custom_text.pop();
                        }

                        // Enter
                        (KeyCode::Enter, _) => {
                            if question.multi_select {
                                if cursor_pos == submit_idx {
                                    // Collect checked options
                                    let selected: Vec<String> = checked
                                        .iter()
                                        .enumerate()
                                        .filter(|(_, &c)| c)
                                        .map(|(i, _)| question.options[i].label.clone())
                                        .collect();
                                    if selected.is_empty() {
                                        // Nothing checked yet — don't submit
                                        continue;
                                    }
                                    break 'widget Some(QuestionAnswer::Multi(selected));
                                } else if cursor_pos == other_idx {
                                    // Toggle custom text option (treated as a checkbox)
                                    // If text is non-empty, include it in multi
                                    if !custom_text.is_empty() {
                                        break 'widget Some(QuestionAnswer::Multi(vec![custom_text.clone()]));
                                    }
                                } else if cursor_pos < n_real {
                                    // Toggle checkbox
                                    checked[cursor_pos] = !checked[cursor_pos];
                                }
                            } else {
                                // Single-select submit
                                if cursor_pos == other_idx {
                                    if !custom_text.is_empty() {
                                        break 'widget Some(QuestionAnswer::Single(custom_text.clone()));
                                    }
                                    // No text typed yet — stay
                                } else {
                                    let label = question.options[cursor_pos].label.clone();
                                    break 'widget Some(QuestionAnswer::Single(label));
                                }
                            }
                        }

                        // Regular character input (for "Type something." row)
                        (KeyCode::Char(c), mods)
                            if cursor_pos == other_idx
                                && (mods == KeyModifiers::NONE || mods == KeyModifiers::SHIFT) =>
                        {
                            custom_text.push(c);
                        }

                        _ => {}
                    }
                }

                _ => {}
            }
        };

        // ── Cleanup: drop raw mode and terminal, leave alternate screen ────────
        // LeaveAlternateScreen restores the main screen buffer exactly as it was
        // before EnterAlternateScreen — content, ThinkingBar, cursor all intact.
        drop(_raw);
        drop(term);
        let mut out = io::stdout();
        execute!(out, LeaveAlternateScreen)?;
        let _ = out.flush();

        Ok(answer)
    }
}
