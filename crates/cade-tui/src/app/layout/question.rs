use crate::app::ActiveQuestionDrawState;
use crate::colors::{ThemeColors, ThemeColorsExt};
use ratatui::{
    Frame,
    layout::{Alignment, Rect},
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Paragraph, Wrap},
};
use std::borrow::Cow;

/// Reserve an inline decision panel without taking over the conversation.
/// The details and choices have separate viewports, so neither can displace
/// the keyboard hints or the focused choice.
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
        .max(6)
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::question::{Question, QuestionOption};
    use ratatui::{Terminal, backend::TestBackend};

    fn draw(state: &ActiveQuestionDrawState, width: u16, height: u16) -> String {
        let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
        terminal
            .draw(|frame| {
                render_question_inline(
                    frame,
                    state,
                    Rect::new(0, 0, width, height),
                    &ThemeColors::default(),
                );
            })
            .unwrap();
        terminal
            .backend()
            .buffer()
            .content()
            .iter()
            .map(|cell| cell.symbol())
            .collect()
    }

    #[test]
    fn approval_keeps_focused_choice_and_hints_visible_with_long_details() {
        let question = Question {
            header: "Approve file.write".into(),
            text: (0..20)
                .map(|i| format!("Argument {i}: a long value\n"))
                .collect(),
            options: (0..12)
                .map(|i| QuestionOption {
                    label: format!("Choice {i}"),
                    description: format!("Description {i}"),
                })
                .collect(),
            multi_select: false,
            allow_other: false,
            progress: None,
        };
        let mut state = ActiveQuestionDrawState::new(question);
        state.cursor_pos = 11;

        let screen = draw(&state, 60, 13);
        assert!(screen.contains("Approve file.write"));
        assert!(screen.contains("Choice 11"));
        assert!(screen.contains("Esc deny"));
        assert!(!screen.contains("Choice 0"));

        let mut scrolled = state.clone();
        scrolled.detail_scroll = 18;
        assert!(draw(&scrolled, 60, 13).contains("Argument 18"));
        assert!(draw(&state, 42, 13).contains("Esc deny"));
    }

    #[test]
    fn long_custom_answer_keeps_the_cursor_and_latest_text_visible() {
        let question = Question {
            header: "Ask".into(),
            text: "What should we name it?".into(),
            options: vec![],
            multi_select: false,
            allow_other: true,
            progress: None,
        };
        let mut state = ActiveQuestionDrawState::new(question);
        state.custom_text = format!("{}end", "a".repeat(100));
        let screen = draw(&state, 42, 10);
        assert!(screen.contains("…"));
        assert!(screen.contains("end▌"));
    }
}
