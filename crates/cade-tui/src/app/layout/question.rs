use crate::app::ActiveQuestionDrawState;
use crate::colors::{ThemeColors, ThemeColorsExt};
use ratatui::{
    Frame,
    layout::{Alignment, Rect},
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Clear, Paragraph, Wrap},
};
use std::borrow::Cow;

/// Reserve room for the decision in the input region while keeping the
/// conversation visible. Details and choices scroll independently.
pub(crate) fn question_height(aq: &ActiveQuestionDrawState, content_height: u16) -> u16 {
    let q = &aq.question;
    let choices = q.options.iter().fold(0usize, |rows, option| {
        rows + 1 + usize::from(!option.description.is_empty())
    }) + usize::from(aq.has_other)
        + usize::from(aq.has_submit);
    let details = q.text.lines().take(5).count().max(2);
    let desired = details + choices + 2; // separator and keyboard hints
    (desired.min(u16::MAX as usize) as u16)
        .min(content_height / 2)
        .max(6.min(content_height))
}

pub(crate) fn render_question_inline(
    frame: &mut Frame,
    aq: &ActiveQuestionDrawState,
    area: Rect,
    colors: &ThemeColors,
) {
    if area.width < 4 || area.height < 4 {
        return;
    }

    let q = &aq.question;
    let approval = q.header.starts_with("Approve");
    let accent = if approval {
        colors.c_warning()
    } else {
        colors.c_primary()
    };
    let title = Line::from(vec![
        Span::styled(" ◆ ", Style::default().fg(accent)),
        Span::styled(
            q.header.as_str(),
            Style::default()
                .fg(colors.c_text_primary())
                .add_modifier(Modifier::BOLD),
        ),
        Span::raw(" "),
    ]);
    let mut block = Block::default()
        .borders(Borders::ALL)
        .border_type(colors.c_border_style())
        .border_style(Style::default().fg(accent))
        .style(colors.style_surface0())
        .title(title);
    if let Some((current, total)) = q.progress {
        block = block.title(
            Line::from(format!(" {current}/{total} "))
                .alignment(Alignment::Right)
                .style(colors.text_muted()),
        );
    }

    let inner = block.inner(area);
    frame.render_widget(block, area);
    if inner.width == 0 || inner.height < 3 {
        return;
    }
    let body = Rect::new(
        inner.x.saturating_add(1),
        inner.y,
        inner.width.saturating_sub(2),
        inner.height,
    );
    if body.width == 0 {
        return;
    }

    let detail_rows = q.text.lines().take(5).count().max(2) as u16;
    let detail_height = detail_rows.min(body.height.saturating_sub(3)).max(1);
    let detail_area = Rect::new(body.x, body.y, body.width, detail_height);
    frame.render_widget(
        Paragraph::new(q.text.as_str())
            .style(colors.text_primary())
            .wrap(Wrap { trim: false })
            .scroll((aq.detail_scroll, 0)),
        detail_area,
    );

    let rule_y = detail_area.bottom();
    frame.render_widget(
        Paragraph::new(Line::from(Span::styled(
            "─".repeat(body.width as usize),
            colors.border_muted(),
        ))),
        Rect::new(body.x, rule_y, body.width, 1),
    );

    let options_area = Rect::new(
        body.x,
        rule_y.saturating_add(1),
        body.width,
        body.height.saturating_sub(detail_height + 2),
    );
    // Keep the insertion point in view when the free-text answer grows past
    // the width of the panel. Regular choices borrow their labels unchanged.
    let custom_width = body.width.saturating_sub(10) as usize;
    let custom_preview: Cow<'_, str> =
        if aq.has_other && aq.custom_text.chars().count() > custom_width {
            let tail: String = aq
                .custom_text
                .chars()
                .rev()
                .take(custom_width.saturating_sub(1))
                .collect::<Vec<_>>()
                .into_iter()
                .rev()
                .collect();
            Cow::Owned(format!("…{tail}"))
        } else {
            Cow::Borrowed(&aq.custom_text)
        };
    let mut lines = Vec::with_capacity(aq.total_items.saturating_mul(2));
    let mut focused_row = 0usize;
    for idx in 0..aq.total_items {
        let focused = idx == aq.cursor_pos;
        if focused {
            focused_row = lines.len();
        }
        let marker = if focused { "›" } else { " " };
        let (label, description) = if idx == aq.submit_idx {
            ("Confirm selection", "")
        } else if idx == aq.other_idx {
            (
                if aq.custom_text.is_empty() {
                    if approval {
                        "Type your instructions"
                    } else {
                        "Type your own answer"
                    }
                } else {
                    custom_preview.as_ref()
                },
                "",
            )
        } else {
            let option = &q.options[idx];
            (option.label.as_str(), option.description.as_str())
        };
        let choice = if q.multi_select && idx < aq.n_real {
            if aq.checked[idx] { "[✓] " } else { "[ ] " }
        } else {
            ""
        };
        let style = if focused {
            Style::default()
                .fg(colors.c_text_primary())
                .bg(colors.c_bg_surface2())
                .add_modifier(Modifier::BOLD)
        } else {
            colors.text_primary()
        };
        lines.push(
            Line::from(vec![
                Span::styled(
                    format!(" {marker} {}. ", idx + 1),
                    Style::default().fg(accent),
                ),
                Span::styled(choice, Style::default().fg(colors.c_success())),
                Span::styled(label, style),
                Span::styled(
                    if focused && idx == aq.other_idx {
                        "▌"
                    } else {
                        ""
                    },
                    Style::default().fg(accent),
                ),
            ])
            .style(style),
        );
        if !description.is_empty() {
            lines.push(Line::from(vec![
                Span::raw("     "),
                Span::styled(description, colors.text_muted()),
            ]));
        }
    }
    if options_area.height > 0 {
        // Keep the active choice visible even with long argument previews or
        // more options than the terminal can display. Each choice occupies one
        // row; its description occupies a second row when present.
        let focused_description = aq.cursor_pos < aq.n_real
            && !q.options[aq.cursor_pos].description.is_empty()
            && options_area.height >= 2;
        let last_row = focused_row + 1 + usize::from(focused_description);
        let offset = last_row.saturating_sub(options_area.height as usize);
        frame.render_widget(
            Paragraph::new(lines).scroll((offset.min(u16::MAX as usize) as u16, 0)),
            options_area,
        );
    }

    let cancel = if approval { "deny" } else { "cancel" };
    let verb = if q.multi_select { "toggle" } else { "select" };
    let hint = if body.width < 52 {
        format!("↑↓ move  Enter {verb}  Esc {cancel}")
    } else if body.width < 85 {
        format!("↑↓ move · Enter {verb} · Esc {cancel} · PgDn details")
    } else if q.multi_select {
        format!("↑↓ navigate  ·  enter toggle/confirm  ·  esc {cancel}  ·  pgup/pgdn details")
    } else {
        format!(
            "↑↓ navigate  ·  enter select  ·  1-9 quick pick  ·  esc {cancel}  ·  pgup/pgdn details"
        )
    };
    frame.render_widget(
        Paragraph::new(hint).style(colors.text_muted()),
        Rect::new(body.x, body.bottom().saturating_sub(1), body.width, 1),
    );
}

/// Render the centered question modal dialog on the overlay stack.
/// Draws a high-visibility bordered card with dimmed backdrop,
/// radio markers for single-select (or checkboxes for multi-select),
/// and keyboard navigation hints.
pub(crate) fn render_question_modal(
    frame: &mut Frame,
    aq: &ActiveQuestionDrawState,
    full_area: Rect,
    colors: &ThemeColors,
) {
    let q = &aq.question;

    // 1. Calculate responsive modal dimensions
    let mut desired_w: u16 = 50;
    let header_len = q.header.chars().count() as u16 + 8;
    desired_w = desired_w.max(header_len);
    for l in q.text.lines() {
        desired_w = desired_w.max(l.chars().count() as u16 + 6);
    }
    for opt in &q.options {
        let opt_len = opt.label.chars().count() as u16 + 14;
        desired_w = desired_w.max(opt_len);
        if !opt.description.is_empty() {
            desired_w = desired_w.max(opt.description.chars().count() as u16 + 14);
        }
    }

    let modal_w = desired_w
        .min(full_area.width.saturating_sub(2))
        .min(76)
        .max(20.min(full_area.width));

    // Calculate height
    let mut rows: u16 = 2; // top & bottom borders
    rows += q.text.lines().count().max(1) as u16;
    rows += 1; // blank line after question text

    if q.progress.is_some() {
        rows += 2; // "Question N of M" + blank line
    }

    for idx in 0..aq.total_items {
        if idx == aq.submit_idx || idx == aq.other_idx {
            rows += 2;
        } else {
            rows += 1;
            if idx < q.options.len() && !q.options[idx].description.is_empty() {
                rows += 1;
            }
        }
    }
    rows += 2; // blank + hint line

    let modal_h = rows
        .min(full_area.height.saturating_sub(2))
        .max(6.min(full_area.height));

    let x = full_area.x + (full_area.width.saturating_sub(modal_w)) / 2;
    let y = full_area.y + (full_area.height.saturating_sub(modal_h)) / 2;
    let modal_area = Rect::new(x, y, modal_w, modal_h);

    // 2. Clear underlying area
    frame.render_widget(Clear, modal_area);

    // 3. Render bordered card block
    let title_text = if q.header.is_empty() {
        "Question".to_string()
    } else {
        q.header.clone()
    };
    let title = Line::from(vec![
        Span::raw(" "),
        Span::styled(
            format!("❓ {title_text}"),
            Style::default()
                .fg(colors.c_md_heading())
                .add_modifier(Modifier::BOLD),
        ),
        Span::raw(" "),
    ]);

    let block = Block::default()
        .borders(Borders::ALL)
        .border_type(colors.c_border_style())
        .style(Style::default().bg(colors.c_bg_surface2()))
        .border_style(colors.border_accent())
        .title(title);

    let inner = block.inner(modal_area);
    frame.render_widget(block, modal_area);

    // 4. Render inner lines
    let mut lines: Vec<Line<'static>> = Vec::new();

    // Question text
    for l in q.text.lines().skip(aq.detail_scroll as usize).take(5) {
        if !l.trim().is_empty() || q.text.lines().count() == 1 {
            lines.push(Line::from(Span::styled(
                l.to_string(),
                colors.text_primary_bold(),
            )));
        }
    }
    lines.push(Line::from(""));

    // Progress indicator
    if let Some((cur, tot)) = q.progress {
        lines.push(Line::from(Span::styled(
            format!("Question {cur} of {tot}"),
            colors.text_muted(),
        )));
        lines.push(Line::from(""));
    }

    let mut selected_line_idx: usize = 0;

    // Options
    for idx in 0..aq.total_items {
        let is_selected = aq.cursor_pos == idx;
        let selector = if is_selected { "❯" } else { " " };
        if is_selected {
            selected_line_idx = lines.len();
        }

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
                format!(" {selector} [Submit]"),
                style,
            )));
            lines.push(Line::from(""));
            continue;
        }

        // Free-text "Other" item
        if idx == aq.other_idx {
            let display = if is_selected {
                if aq.custom_text.is_empty() {
                    "Type something...█".to_string()
                } else {
                    let chars: Vec<char> = aq.custom_text.chars().collect();
                    let pos = aq.custom_cursor_pos.min(chars.len());
                    let mut s = String::new();
                    for (i, c) in chars.iter().enumerate() {
                        if i == pos {
                            s.push('█');
                        }
                        s.push(*c);
                    }
                    if pos == chars.len() {
                        s.push('█');
                    }
                    s
                }
            } else if !aq.custom_text.is_empty() {
                aq.custom_text.clone()
            } else {
                "Type something...".to_string()
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
        let indicator = if q.multi_select {
            if aq.checked[idx] { "[✓] " } else { "[ ] " }
        } else if is_selected {
            "(•) "
        } else {
            "( ) "
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
        let indicator_style = if is_selected {
            Style::default()
                .fg(colors.c_success())
                .add_modifier(Modifier::BOLD)
        } else {
            colors.text_muted()
        };

        let mut label_lines = opt.label.lines();
        if let Some(first) = label_lines.next() {
            lines.push(Line::from(vec![
                Span::styled(format!(" {selector} "), colors.success()),
                Span::styled(format!("{}. ", idx + 1), num_style),
                Span::styled(indicator.to_string(), indicator_style),
                Span::styled(first.to_string(), label_style),
            ]));
        }
        for l in label_lines {
            lines.push(Line::from(vec![
                Span::raw("         "),
                Span::styled(l.to_string(), label_style),
            ]));
        }

        if !opt.description.is_empty() {
            for l in opt.description.lines() {
                lines.push(Line::from(Span::styled(
                    format!("         {}", l),
                    colors.text_muted(),
                )));
            }
        }
    }

    // Keep the keyboard hint outside the scrolling choices so an approval
    // remains actionable even when the question or option list is very long.
    let cancel = if q.header.starts_with("Approve") {
        "deny"
    } else {
        "cancel"
    };
    let hint = if q.multi_select && inner.width < 70 {
        format!("Space toggle · 1-N toggle · Esc {cancel}")
    } else if q.multi_select {
        format!("Space toggle · 1-N toggle · ↑↓/Tab navigate · Enter on [Submit] · Esc {cancel}")
    } else if inner.width < 70 {
        format!("1-N quick pick · Enter select · Esc {cancel}")
    } else {
        format!("1-N quick pick · ↑↓/Tab navigate · Enter select · Esc {cancel}")
    };
    let body = Rect::new(
        inner.x,
        inner.y,
        inner.width,
        inner.height.saturating_sub(1),
    );
    if inner.height > 0 {
        frame.render_widget(
            Paragraph::new(hint).style(colors.text_dim().add_modifier(Modifier::DIM)),
            Rect::new(inner.x, inner.bottom().saturating_sub(1), inner.width, 1),
        );
    }

    let total_lines = lines.len();
    let visible_height = body.height as usize;

    let scroll_y = if total_lines > visible_height && visible_height > 0 {
        if selected_line_idx < aq.scroll_offset as usize {
            selected_line_idx as u16
        } else if selected_line_idx >= (aq.scroll_offset as usize) + visible_height {
            (selected_line_idx + 1).saturating_sub(visible_height) as u16
        } else {
            aq.scroll_offset
        }
    } else {
        0
    };

    frame.render_widget(
        Paragraph::new(lines)
            .wrap(Wrap { trim: false })
            .scroll((scroll_y, 0))
            .style(Style::default()),
        body,
    );
}
