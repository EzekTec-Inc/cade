use crate::app::*;
use crate::colors::ThemeColorsExt;
/// Calculate the number of rows needed for the inline question panel.
/// Counts: 1 header + 1 blank + wrapped-question-rows + 1 blank
///       + per-option rows (label + optional description)
///       + submit row (multi-select) + other row + 1 blank + 1 hint.
/// Clamped to at most half the content viewport so content is never fully hidden.
pub(crate) fn question_height(aq: &ActiveQuestionDrawState, content_height: u16) -> u16 {
    let q = &aq.question;

    // Fixed rows: separator-row is accounted for by the caller (inline_h - 1 for body).
    // Here we return the total including the separator row.
    let mut rows: u16 = 0;

    // header chip (no blank)
    rows += q.header.lines().count().max(1) as u16;
    // question text (treat as 1 row; long questions word-wrap but we keep it simple)
    rows += q.text.lines().count().max(1) as u16;

    // progress indicator
    if q.progress.is_some() {
        rows += 2; // "Question N of M" + blank
    }

    // options: label row always, description row only if non-empty
    for idx in 0..aq.total_items {
        if idx == aq.submit_idx {
            rows += 2; // label + blank
        } else if idx == aq.other_idx {
            rows += 2; // label + blank
        } else {
            rows += q.options[idx].label.lines().count().max(1) as u16; // label
            if idx < q.options.len() && !q.options[idx].description.is_empty() {
                rows += q.options[idx].description.lines().count().max(1) as u16; // description
            }
        }
    }

    // blank + hint
    rows += 2;

    // +1 for the dashed separator row itself
    rows += 1;

    rows.min(content_height / 2).max(6)
}

/// Render the inline question panel — no border box, anchored to the bottom
/// of the content viewport via the layout split in `render_frame`.
/// `sep_area`  — the single row reserved for the dashed separator (chunks[1]).
/// `body_area` — the panel body rows (chunks[2]).
pub(crate) fn render_question_inline(
    frame: &mut Frame,
    aq: &ActiveQuestionDrawState,
    sep_area: Rect,
    body_area: Rect,
    colors: &ThemeColors,
) {
    let q = &aq.question;

    // -- Dashed separator
    // Use a dimmer, shorter dash to visually distinguish from the hard ─ separators.
    let dash_w = sep_area.width as usize;
    let dash_str = "╌".repeat(dash_w);
    frame.render_widget(
        Paragraph::new(Line::from(Span::styled(dash_str, colors.border_muted()))),
        sep_area,
    );

    // -- Panel body
    let mut lines: Vec<Line<'static>> = Vec::new();

    // Header chip — left-aligned, yellow bold with a diamond glyph
    let mut header_lines = q.header.lines();
    if let Some(first) = header_lines.next() {
        lines.push(Line::from(vec![
            Span::styled("◆ ", colors.md_heading()),
            Span::styled(
                first.to_string(),
                Style::default()
                    .fg(colors.c_md_heading())
                    .add_modifier(Modifier::BOLD),
            ),
        ]));
    }
    for l in header_lines {
        lines.push(Line::from(vec![
            Span::raw("  "),
            Span::styled(
                l.to_string(),
                Style::default()
                    .fg(colors.c_md_heading())
                    .add_modifier(Modifier::BOLD),
            ),
        ]));
    }

    // Question text
    for l in q.text.lines() {
        if !l.trim().is_empty() || q.text.lines().count() == 1 {
            lines.push(Line::from(Span::styled(
                l.to_string(),
                colors.text_primary(),
            )));
        }
    }

    // Progress indicator
    if let Some((cur, tot)) = q.progress {
        lines.push(Line::from(Span::styled(
            format!("Question {cur} of {tot}"),
            colors.text_muted(),
        )));
        lines.push(Line::from(""));
    }

    // Options
    for idx in 0..aq.total_items {
        let is_selected = aq.cursor_pos == idx;
        let selector = if is_selected { "❯" } else { " " };

        // Submit item (multi-select only)
        if idx == aq.submit_idx {
            let style = if is_selected {
                Style::default()
                    .fg(colors.c_success())
                    .add_modifier(Modifier::BOLD)
            } else {
                colors.text_muted()
            };
            lines.push(Line::from(Span::styled(
                format!(" {selector} {}.  Submit", idx + 1),
                style,
            )));
            lines.push(Line::from(""));
            continue;
        }

        // Free-text "Other" item
        if idx == aq.other_idx {
            let display = if is_selected {
                if aq.custom_text.is_empty() {
                    "Type something.█".to_string()
                } else {
                    format!("{}█", aq.custom_text)
                }
            } else if !aq.custom_text.is_empty() {
                aq.custom_text.clone()
            } else {
                "Type something.".to_string()
            };
            lines.push(Line::from(vec![
                Span::styled(
                    format!(" {selector} {}.  ", idx + 1),
                    Style::default().fg(if is_selected {
                        colors.c_success()
                    } else {
                        colors.c_text_muted()
                    }),
                ),
                Span::styled(
                    display,
                    Style::default()
                        .fg(colors.c_text_dim())
                        .add_modifier(Modifier::ITALIC),
                ),
            ]));
            lines.push(Line::from(""));
            continue;
        }

        // Regular option
        let opt = &q.options[idx];
        let checkbox = if q.multi_select {
            if aq.checked[idx] { "[✓] " } else { "[ ] " }
        } else {
            ""
        };
        let num_style = if is_selected {
            colors.success()
        } else {
            colors.text_muted()
        };
        let label_style = if is_selected {
            Style::default()
                .fg(colors.c_text_primary())
                .add_modifier(Modifier::BOLD)
        } else {
            colors.text_primary()
        };

        let mut label_lines = opt.label.lines();
        if let Some(first) = label_lines.next() {
            lines.push(Line::from(vec![
                Span::styled(format!(" {selector} "), colors.success()),
                Span::styled(format!("{}. ", idx + 1), num_style),
                Span::styled(checkbox.to_string(), colors.success()),
                Span::styled(first.to_string(), label_style),
            ]));
        }
        for l in label_lines {
            lines.push(Line::from(vec![
                Span::raw("       "),
                Span::styled(l.to_string(), label_style),
            ]));
        }

        if !opt.description.is_empty() {
            for l in opt.description.lines() {
                lines.push(Line::from(Span::styled(
                    format!("       {}", l),
                    colors.text_muted(),
                )));
            }
        }
    }

    // Hint line
    lines.push(Line::from(""));
    let hint = if q.multi_select {
        "Enter toggle · ↑↓ navigate · Enter on Submit to confirm · Esc cancel"
    } else {
        "Enter select · ↑↓ navigate · 1-N quick-pick · Esc cancel"
    };
    lines.push(Line::from(Span::styled(
        hint,
        colors.text_dim().add_modifier(Modifier::DIM),
    )));

    frame.render_widget(
        Paragraph::new(lines)
            .wrap(Wrap { trim: false })
            .style(Style::default()),
        body_area,
    );
}
