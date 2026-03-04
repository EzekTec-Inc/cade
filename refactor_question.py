import re

# 1. Update src/ui/app.rs
with open('src/ui/app.rs', 'r') as f:
    app_src = f.read()

# Add RenderLine::QuestionResult
app_src = app_src.replace(
    "Blank,\n}",
    "Blank,\n    /// Interactive question completed result.\n    QuestionResult { header: String, answer: String },\n}"
)

# Add RenderLine rendering logic
render_line_match = """        RenderLine::Table { headers, rows } => {"""
render_question_result = """        RenderLine::QuestionResult { header, answer } => {
            out.push(Line::from(vec![
                Span::styled("● ", Style::default().fg(RC::Green).add_modifier(Modifier::BOLD)),
                Span::styled(format!("{header}: "), Style::default().fg(RC::Cyan).add_modifier(Modifier::BOLD)),
                Span::styled(answer.clone(), Style::default().fg(RC::White)),
            ]));
        }
        RenderLine::Table { headers, rows } => {"""
app_src = app_src.replace(render_line_match, render_question_result)

# Add ActiveQuestionState
state_struct = """// ── ActiveQuestionState ───────────────────────────────────────────────────────
pub struct ActiveQuestionState<'a> {
    pub question: &'a crate::ui::question::Question<'a>,
    pub cursor_pos: usize,
    pub custom_text: &'a str,
    pub checked: &'a [bool],
    pub n_real: usize,
    pub has_other: bool,
    pub has_submit: bool,
    pub total_items: usize,
    pub other_idx: usize,
    pub submit_idx: usize,
}

// ── TuiApp ────────────────────────────────────────────────────────────────────"""
app_src = app_src.replace("// ── TuiApp ────────────────────────────────────────────────────────────────────", state_struct)

# Add ask_question method to TuiApp
ask_method = """    // ── Interactive Question ──────────────────────────────────────────────

    pub fn ask_question(&mut self, question: &crate::ui::question::Question<'_>) -> Result<Option<crate::ui::question::QuestionAnswer>> {
        let n_real     = question.options.len();
        let has_other  = question.allow_other;
        let has_submit = question.multi_select;
        let total_items = n_real + usize::from(has_other) + usize::from(has_submit);

        let other_idx  = if has_other  { n_real } else { usize::MAX };
        let submit_idx = if has_submit { n_real + usize::from(has_other) } else { usize::MAX };

        let mut cursor_pos: usize = 0;
        let mut custom_text = String::new();
        let mut checked: Vec<bool> = vec![false; n_real];

        // snap to bottom when asking
        self.scroll = 0;

        let answer: Option<crate::ui::question::QuestionAnswer> = 'widget: loop {
            let aq = ActiveQuestionState {
                question,
                cursor_pos,
                custom_text: &custom_text,
                checked: &checked,
                n_real,
                has_other,
                has_submit,
                total_items,
                other_idx,
                submit_idx,
            };

            self.draw_impl(Some(&aq))?;

            if !event::poll(std::time::Duration::from_millis(50))? {
                continue;
            }
            match event::read()? {
                Event::Key(KeyEvent { code, modifiers, .. }) => {
                    match (code, modifiers) {
                        (KeyCode::Esc, _) | (KeyCode::Char('c'), KeyModifiers::CONTROL) => {
                            break 'widget None;
                        }
                        (KeyCode::Up, _) => { if cursor_pos > 0 { cursor_pos -= 1; } }
                        (KeyCode::Down, _) => { if cursor_pos + 1 < total_items { cursor_pos += 1; } }
                        (KeyCode::Tab, _) => { cursor_pos = (cursor_pos + 1) % total_items; }
                        (KeyCode::BackTab, _) => { cursor_pos = if cursor_pos == 0 { total_items - 1 } else { cursor_pos - 1 }; }
                        (KeyCode::Char(c), KeyModifiers::NONE) if c.is_ascii_digit() && c != '0' => {
                            let idx = (c as usize) - ('0' as usize) - 1;
                            if idx < total_items {
                                if question.multi_select {
                                    if idx < n_real {
                                        checked[idx] = !checked[idx];
                                        cursor_pos = idx;
                                    }
                                } else if idx != other_idx {
                                    let label = question.options[idx].label.clone();
                                    break 'widget Some(crate::ui::question::QuestionAnswer::Single(label));
                                } else {
                                    cursor_pos = idx;
                                }
                            }
                        }
                        (KeyCode::Backspace, _) if cursor_pos == other_idx => { custom_text.pop(); }
                        (KeyCode::Enter, _) => {
                            if question.multi_select {
                                if cursor_pos == submit_idx {
                                    let selected: Vec<String> = checked.iter().enumerate()
                                        .filter(|(_, &c)| c)
                                        .map(|(i, _)| question.options[i].label.clone())
                                        .collect();
                                    if selected.is_empty() { continue; }
                                    break 'widget Some(crate::ui::question::QuestionAnswer::Multi(selected));
                                } else if cursor_pos == other_idx {
                                    if !custom_text.is_empty() {
                                        break 'widget Some(crate::ui::question::QuestionAnswer::Multi(vec![custom_text.clone()]));
                                    }
                                } else if cursor_pos < n_real {
                                    checked[cursor_pos] = !checked[cursor_pos];
                                }
                            } else if cursor_pos == other_idx {
                                if !custom_text.is_empty() {
                                    break 'widget Some(crate::ui::question::QuestionAnswer::Single(custom_text.clone()));
                                }
                            } else {
                                let label = question.options[cursor_pos].label.clone();
                                break 'widget Some(crate::ui::question::QuestionAnswer::Single(label));
                            }
                        }
                        (KeyCode::Char(c), m) if cursor_pos == other_idx && (m == KeyModifiers::NONE || m == KeyModifiers::SHIFT) => {
                            custom_text.push(c);
                        }
                        _ => {}
                    }
                }
                _ => {}
            }
        };

        if let Some(ref ans) = answer {
            self.push(RenderLine::QuestionResult {
                header: question.header.to_string(),
                answer: ans.as_str(),
            })?;
        } else {
            self.draw()?; // clear question ui on cancel
        }

        Ok(answer)
    }

    // ── Input loop ────────────────────────────────────────────────────────"""
app_src = app_src.replace("    // ── Input loop ────────────────────────────────────────────────────────", ask_method)

# Modify draw -> draw_impl
app_src = app_src.replace(
    "    pub fn draw(&mut self) -> Result<()> {",
    "    pub fn draw(&mut self) -> Result<()> { self.draw_impl(None) }\n\n    pub fn draw_impl(&mut self, active_question: Option<&ActiveQuestionState<'_>>) -> Result<()> {"
)

# Modify render_frame call
app_src = app_src.replace(
    "                thinking_elapsed,\n            );\n        })?;\n        Ok(())",
    "                thinking_elapsed,\n                active_question,\n            );\n        })?;\n        Ok(())"
)

# Modify render_frame signature
app_src = app_src.replace(
    "    thinking_elapsed: Option<std::time::Duration>,\n) {",
    "    thinking_elapsed: Option<std::time::Duration>,\n    active_question: Option<&ActiveQuestionState<'_>>,\n) {"
)

# Append question lines in render_frame
render_text_lines = """    if let Some(s) = streaming {
        render_assistant_lines(s, w, &mut text_lines);
    }

    if let Some(aq) = active_question {
        render_active_question(aq, w, &mut text_lines);
    }"""
app_src = app_src.replace("    if let Some(s) = streaming {\n        render_assistant_lines(s, w, &mut text_lines);\n    }", render_text_lines)

# Add render_active_question helper
helper_src = """fn render_active_question(aq: &ActiveQuestionState<'_>, _width: usize, lines: &mut Vec<Line<'static>>) {
    let q = aq.question;
    lines.push(Line::from(""));
    let sep = "─".repeat(50);
    lines.push(Line::from(Span::styled(sep, Style::default().fg(RC::DarkGray))));
    lines.push(Line::from(Span::styled(q.header.to_string(), Style::default().fg(RC::White).add_modifier(Modifier::BOLD))));
    lines.push(Line::from(""));
    lines.push(Line::from(Span::styled(q.text.to_string(), Style::default().fg(RC::White))));
    lines.push(Line::from(""));

    if let Some((cur, tot)) = q.progress {
        lines.push(Line::from(Span::styled(format!("Question {cur} of {tot}"), Style::default().fg(RC::DarkGray))));
        lines.push(Line::from(""));
    }

    for idx in 0..aq.total_items {
        let is_selected = aq.cursor_pos == idx;
        let selector    = if is_selected { "❯" } else { " " };

        if idx == aq.submit_idx {
            let label_style = if is_selected { Style::default().fg(RC::Green).add_modifier(Modifier::BOLD) } else { Style::default().fg(RC::DarkGray) };
            lines.push(Line::from(Span::styled(format!("{selector} {}.    Submit", idx + 1), label_style)));
            lines.push(Line::from(""));
            continue;
        }

        if idx == aq.other_idx {
            let display = if aq.cursor_pos == idx {
                if aq.custom_text.is_empty() { "Type something.█".to_string() } else { format!("{}█", aq.custom_text) }
            } else if !aq.custom_text.is_empty() { aq.custom_text.to_string() } else { "Type something.".to_string() };
            let other_style = Style::default().fg(RC::DarkGray).add_modifier(Modifier::ITALIC);
            lines.push(Line::from(vec![
                Span::styled(selector.to_string(), Style::default().fg(RC::Green)),
                Span::styled(format!(" {}.    {display}", idx + 1), other_style),
            ]));
            lines.push(Line::from(""));
            continue;
        }

        let opt = &q.options[idx];
        let checkbox = if q.multi_select { if aq.checked[idx] { "[✓] " } else { "[ ] " } } else { "" };
        let label_style = if is_selected { Style::default().fg(RC::White).add_modifier(Modifier::BOLD) } else { Style::default().fg(RC::White) };
        let num_style = if is_selected { Style::default().fg(RC::Green) } else { Style::default().fg(RC::DarkGray) };

        lines.push(Line::from(vec![
            Span::styled(selector.to_string(), Style::default().fg(RC::Green)),
            Span::styled(format!(" {}. ", idx + 1), num_style),
            Span::styled(checkbox.to_string(), Style::default().fg(RC::Green)),
            Span::styled(opt.label.clone(), label_style),
        ]));
        lines.push(Line::from(Span::styled(format!("     {}", opt.description), Style::default().fg(RC::DarkGray))));
    }

    let hint = if q.multi_select { "Enter to toggle · ↑↓ navigate · Enter on Submit to confirm · Esc to cancel" }
               else { "Enter to select · ↑↓ navigate · 1-N quick select · Esc to cancel" };
    lines.push(Line::from(Span::styled(hint.to_string(), Style::default().fg(RC::DarkGray).add_modifier(Modifier::DIM))));
}

// ── Line renderers ────────────────────────────────────────────────────────────"""
app_src = app_src.replace("// ── Line renderers ────────────────────────────────────────────────────────────", helper_src)

with open('src/ui/app.rs', 'w') as f:
    f.write(app_src)

print("Updated src/ui/app.rs")

# 2. Update src/cli/repl.rs
with open('src/cli/repl.rs', 'r') as f:
    repl_src = f.read()

# Replace all QuestionWidget::ask with app.ask_question
repl_src = re.sub(r'crate::ui::question::QuestionWidget::ask\(&mut app\.terminal, &([^)]+)\)', r'app.ask_question(&\1)', repl_src)
repl_src = re.sub(r'QuestionWidget::ask\(&mut app\.terminal, &([^)]+)\)', r'app.ask_question(&\1)', repl_src)

# Remove the manual app.scroll = 0 and app.draw() after ask_question
repl_src = re.sub(
    r'let (r|result|res) = app\.ask_question\(&([^)]+)\)\?;\s+app\.scroll = 0;\s+let _ = app\.draw\(\);\s+(r|result|res)',
    r'app.ask_question(&\2)?',
    repl_src
)
repl_src = re.sub(
    r'let (r|result|res) = app\.ask_question\(&([^)]+)\)\?;\s+app\.scroll = 0;\s+// snap to bottom after approval modal\s+let _ = app\.draw\(\);\s+(r|result|res)',
    r'app.ask_question(&\2)?',
    repl_src
)
repl_src = re.sub(
    r'let (r|result|res) = app\.ask_question\(&([^)]+)\)\?;\s+app\.scroll = 0;\s+// snap to bottom after question modal\s+let _ = app\.draw\(\);\s+(r|result|res)',
    r'app.ask_question(&\2)?',
    repl_src
)


with open('src/cli/repl.rs', 'w') as f:
    f.write(repl_src)

print("Updated src/cli/repl.rs")
