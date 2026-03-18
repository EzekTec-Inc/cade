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

use anyhow::Result;
use crossterm::event::{self, Event, KeyCode, KeyEvent, KeyModifiers};
use ratatui::{
    DefaultTerminal,
    layout::{Constraint, Direction, Layout},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::Paragraph,
};

// ── Public types ──────────────────────────────────────────────────────────────

/// One labelled option in a question.
#[derive(Debug, Clone)]
pub struct QuestionOption {
    pub label:       String,
    pub description: String,
}

/// A single question to present to the user.
#[derive(Debug, Clone)]
pub struct Question {
    /// Short chip/tag label shown above the question text (≤12 chars).
    pub header: String,
    /// Full question text.
    pub text: String,
    /// 2–N answer options.
    pub options: Vec<QuestionOption>,
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
            Self::Single(s) => s.clone(),
            Self::Multi(v)  => v.join(", "),
        }
    }
}

// ── QuestionWidget ────────────────────────────────────────────────────────────

pub struct QuestionWidget;

impl QuestionWidget {
    /// Present `question` interactively using the provided terminal.
    ///
    /// The question is rendered over the full screen.  Caller is responsible for
    /// redrawing the normal CADE UI after this returns.  Raw mode must already be
    /// active (guaranteed when called from TuiApp context).
    pub fn ask(
        terminal: &mut DefaultTerminal,
        question: &Question,
    ) -> Result<Option<QuestionAnswer>> {
        // ── Build the effective options list ──────────────────────────────────
        let n_real     = question.options.len();
        let has_other  = question.allow_other;
        let has_submit = question.multi_select;
        let total_items = n_real + usize::from(has_other) + usize::from(has_submit);

        let other_idx  = if has_other  { n_real } else { usize::MAX };
        let submit_idx = if has_submit { n_real + usize::from(has_other) } else { usize::MAX };

        // ── State ─────────────────────────────────────────────────────────────
        let mut cursor_pos: usize = 0;
        let mut custom_text = String::new();
        let mut checked: Vec<bool> = vec![false; n_real];

        let answer: Option<QuestionAnswer> = 'widget: loop {
            // ── Draw ──────────────────────────────────────────────────────────
            let (term_w, _) = crossterm::terminal::size().unwrap_or((80, 24));
            let sep = "─".repeat(term_w as usize);

            terminal.draw(|frame| {
                let area = frame.area();
                let mut lines: Vec<Line<'static>> = Vec::new();

                // Separator
                lines.push(Line::from(Span::styled(
                    sep.clone(),
                    Style::default().fg(Color::DarkGray),
                )));

                // Header chip
                lines.push(Line::from(Span::styled(
                    question.header.to_string(),
                    Style::default().fg(Color::White).add_modifier(Modifier::BOLD),
                )));
                lines.push(Line::from(""));

                // Question text
                lines.push(Line::from(Span::styled(
                    question.text.to_string(),
                    Style::default().fg(Color::White),
                )));
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
                    let selector    = if is_selected { "❯" } else { " " };

                    if idx == submit_idx {
                        let label_style = if is_selected {
                            Style::default().fg(Color::Green).add_modifier(Modifier::BOLD)
                        } else {
                            Style::default().fg(Color::DarkGray)
                        };
                        lines.push(Line::from(Span::styled(
                            format!("{selector} {}.    Submit", idx + 1),
                            label_style,
                        )));
                        lines.push(Line::from(""));
                        continue;
                    }

                    if idx == other_idx {
                        let display = if cursor_pos == idx {
                            if custom_text.is_empty() {
                                "Type something.█".to_string()
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

                        let prefix = format!(" {}.    ", idx + 1);
                        let max_len = (term_w as usize).saturating_sub(prefix.len() + 3).max(10);
                        
                        let mut chars: Vec<char> = display.chars().collect();
                        let mut chunks = Vec::new();
                        while !chars.is_empty() {
                            let chunk_size = chars.len().min(max_len);
                            let chunk: String = chars.drain(..chunk_size).collect();
                            chunks.push(chunk);
                        }

                        for (i, chunk) in chunks.into_iter().enumerate() {
                            if i == 0 {
                                lines.push(Line::from(vec![
                                    Span::styled(selector.to_string(), Style::default().fg(Color::Green)),
                                    Span::styled(format!("{}{}", prefix, chunk), other_style),
                                ]));
                            } else {
                                let padding = " ".repeat(prefix.len() + 1);
                                lines.push(Line::from(vec![
                                    Span::raw(padding),
                                    Span::styled(chunk, other_style),
                                ]));
                            }
                        }
                        lines.push(Line::from(""));
                        continue;
                    }

                    // Regular option
                    let opt = &question.options[idx];
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

                // Render: each line gets one row; clip to terminal height.
                let max_lines = area.height as usize;
                let lines = lines.into_iter().take(max_lines).collect::<Vec<_>>();
                let constraints: Vec<Constraint> =
                    lines.iter().map(|_| Constraint::Length(1)).collect();
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
                        (KeyCode::Esc, _) | (KeyCode::Char('c'), KeyModifiers::CONTROL) => {
                            break 'widget None;
                        }
                        // Navigation
                        (KeyCode::Up, _) => {
                            if cursor_pos > 0 { cursor_pos -= 1; }
                        }
                        (KeyCode::Down, _) => {
                            if cursor_pos + 1 < total_items { cursor_pos += 1; }
                        }
                        (KeyCode::Tab, _) => {
                            cursor_pos = (cursor_pos + 1) % total_items;
                        }
                        (KeyCode::BackTab, _) => {
                            cursor_pos = if cursor_pos == 0 { total_items - 1 } else { cursor_pos - 1 };
                        }
                        // Number quick-select
                        (KeyCode::Char(c), KeyModifiers::NONE)
                            if c.is_ascii_digit() && c != '0' =>
                        {
                            let idx = (c as usize) - ('0' as usize) - 1;
                            if idx < total_items {
                                if question.multi_select {
                                    if idx < n_real {
                                        checked[idx] = !checked[idx];
                                        cursor_pos = idx;
                                    }
                                } else if idx != other_idx {
                                    let label = question.options[idx].label.clone();
                                    break 'widget Some(QuestionAnswer::Single(label));
                                } else {
                                    cursor_pos = idx;
                                }
                            }
                        }
                        // Backspace for custom text
                        (KeyCode::Backspace, _) if cursor_pos == other_idx => {
                            custom_text.pop();
                        }
                        // Enter
                        (KeyCode::Enter, _) => {
                            if question.multi_select {
                                if cursor_pos == submit_idx {
                                    let selected: Vec<String> = checked.iter().enumerate()
                                        .filter(|(_, c)| **c)
                                        .map(|(i, _)| question.options[i].label.clone())
                                        .collect();
                                    if selected.is_empty() { continue; }
                                    break 'widget Some(QuestionAnswer::Multi(selected));
                                } else if cursor_pos == other_idx {
                                    if !custom_text.is_empty() {
                                        break 'widget Some(QuestionAnswer::Multi(vec![custom_text.clone()]));
                                    }
                                } else if cursor_pos < n_real {
                                    checked[cursor_pos] = !checked[cursor_pos];
                                }
                            } else if cursor_pos == other_idx {
                                if !custom_text.is_empty() {
                                    break 'widget Some(QuestionAnswer::Single(custom_text.clone()));
                                }
                            } else {
                                let label = question.options[cursor_pos].label.clone();
                                break 'widget Some(QuestionAnswer::Single(label));
                            }
                        }
                        // Regular character input for "Type something." row
                        (KeyCode::Char(c), m)
                            if cursor_pos == other_idx
                                && (m == KeyModifiers::NONE || m == KeyModifiers::SHIFT) =>
                        {
                            custom_text.push(c);
                        }
                        _ => {}
                    }
                }
                _ => {}
            }
        };

        Ok(answer)
    }
}
