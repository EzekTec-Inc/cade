//! Shared, visual-row based presentation for questions and permission decisions.
use crate::app::ActiveQuestionDrawState;
use crate::colors::{ThemeColors, ThemeColorsExt};
use ratatui::{
    Frame,
    layout::{Alignment, Rect},
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Clear, Paragraph},
};

#[derive(Debug, Clone, Default)]
pub(crate) struct DialogGeometry {
    pub panel: Rect,
    pub details: Rect,
    pub choices: Vec<(usize, Rect)>,
    pub detail_max: u16,
    cache_key: Option<(u16, u64, u64)>,
    detail_lines: Vec<Line<'static>>,
}

pub(crate) fn is_spacious(area: Rect) -> bool {
    area.width >= 100 && area.height >= 28
}

pub(crate) fn question_height(aq: &ActiveQuestionDrawState, height: u16) -> u16 {
    let choices = aq.total_items.saturating_mul(2);
    let desired = choices
        .saturating_add(aq.question.text.lines().count().clamp(1, 5))
        .saturating_add(4);
    (desired.min(u16::MAX as usize) as u16)
        .min(height / 2)
        .max(6.min(height))
}

/// Beautify JSON without changing the decision payload. Markdown/code fences
/// use the existing themed parser; plain commands can be supplied as bash fences.
fn formatted_details(text: &str, width: u16, colors: &ThemeColors) -> Vec<Line<'static>> {
    let formatted = serde_json::from_str::<serde_json::Value>(text)
        .ok()
        .filter(|value| value.is_object() || value.is_array())
        .and_then(|value| serde_json::to_string_pretty(&value).ok())
        .map(|json| format!("```json\n{json}\n```"));
    crate::markdown::parse_markdown_lines_with_theme(
        formatted.as_deref().unwrap_or(text),
        colors,
        width as usize,
        true,
    )
    .into_iter()
    .flat_map(|line| super::super::timeline::wrap_line(line, width))
    .collect()
}

/// Wrap the body separately from its prefix, so every continuation aligns with
/// the label, including double-width text and two-digit choice numbers.
fn append_choice(
    lines: &mut Vec<Line<'static>>,
    text: &str,
    prefix: &str,
    width: u16,
    style: Style,
    accent: Style,
) {
    use unicode_width::UnicodeWidthStr;
    // On tiny panels, labels take priority over decorative numbering/radios.
    // Reserving width-1 for a prefix would make the label one column wide.
    let indent_budget = if width < 16 {
        width / 3
    } else {
        width.saturating_sub(1)
    };
    let indent = UnicodeWidthStr::width(prefix).min(indent_budget as usize);
    let prefix: String = prefix.chars().take(indent).collect();
    for (index, line) in super::super::timeline::wrap_line(
        Line::from(Span::styled(text.to_owned(), style)),
        width.saturating_sub(indent as u16).max(1),
    )
    .into_iter()
    .enumerate()
    {
        let mut spans = vec![Span::styled(
            if index == 0 {
                prefix.clone()
            } else {
                " ".repeat(indent)
            },
            accent,
        )];
        spans.extend(line.spans);
        lines.push(Line::from(spans).style(style));
    }
}

pub(crate) fn render_question_inline(
    frame: &mut Frame,
    aq: &ActiveQuestionDrawState,
    area: Rect,
    colors: &ThemeColors,
) {
    render_panel(frame, aq, area, colors);
}

pub(crate) fn render_question_modal(
    frame: &mut Frame,
    aq: &ActiveQuestionDrawState,
    full: Rect,
    colors: &ThemeColors,
) {
    let area = if is_spacious(full) {
        let width = full.width.saturating_sub(8).min(96);
        let height = full.height.saturating_sub(6).min(26);
        Rect::new(
            full.x.saturating_add(full.width.saturating_sub(width) / 2),
            full.y
                .saturating_add(full.height.saturating_sub(height) / 2),
            width,
            height,
        )
    } else {
        let height = question_height(aq, full.height);
        Rect::new(
            full.x,
            full.bottom().saturating_sub(height),
            full.width,
            height,
        )
    };
    render_panel(frame, aq, area, colors);
}

fn format_header_spans(
    header: &str,
    accent: Style,
    colors: &ThemeColors,
) -> Vec<Span<'static>> {
    let mut spans = Vec::new();
    spans.push(Span::styled(" ◆ ", accent.add_modifier(Modifier::BOLD)));

    if !header.contains('[') {
        spans.push(Span::styled(
            format!("{header} "),
            accent.add_modifier(Modifier::BOLD),
        ));
        return spans;
    }

    let mut remaining = header;
    while let Some(open) = remaining.find('[') {
        if open > 0 {
            spans.push(Span::styled(
                remaining[..open].to_string(),
                accent.add_modifier(Modifier::BOLD),
            ));
        }
        if let Some(close) = remaining[open..].find(']') {
            let badge_content = &remaining[open..=open + close];
            let badge_style = if badge_content.contains("High")
                || badge_content.contains("Sensitive")
                || badge_content.contains("Critical")
            {
                colors.error().add_modifier(Modifier::BOLD)
            } else if badge_content.contains("Elevated")
                || badge_content.contains("Uncertain")
                || badge_content.contains("Drift")
            {
                colors.warning().add_modifier(Modifier::BOLD)
            } else if badge_content.contains("Low")
                || badge_content.contains("Minimal")
                || badge_content.contains("Aligned")
            {
                colors.success().add_modifier(Modifier::BOLD)
            } else {
                colors.primary_bold()
            };

            spans.push(Span::styled(badge_content.to_string(), badge_style));
            remaining = &remaining[open + close + 1..];
        } else {
            spans.push(Span::styled(
                remaining.to_string(),
                accent.add_modifier(Modifier::BOLD),
            ));
            remaining = "";
            break;
        }
    }
    if !remaining.is_empty() {
        spans.push(Span::styled(
            remaining.to_string(),
            accent.add_modifier(Modifier::BOLD),
        ));
    }
    spans.push(Span::raw(" "));
    spans
}

fn render_panel(frame: &mut Frame, aq: &ActiveQuestionDrawState, area: Rect, colors: &ThemeColors) {
    let area = area.intersection(frame.area());
    // Geometry is replaced on each draw, including after resize.
    let mut geometry = aq.geometry.lock().unwrap_or_else(|e| e.into_inner());
    geometry.details = Rect::default();
    geometry.panel = area;
    geometry.choices.clear();
    geometry.detail_max = 0;
    frame.render_widget(Clear, area);
    if area.width == 0 || area.height == 0 {
        return;
    }

    let approval =
        aq.question.header.starts_with("Approve") || aq.question.header.starts_with("Permission");
    let accent = if approval {
        colors.warning()
    } else {
        colors.primary()
    };
    let title = Line::from(format_header_spans(&aq.question.header, accent, colors));
    let mut block = Block::default()
        .borders(Borders::ALL)
        .border_type(colors.c_border_style())
        .border_style(accent)
        .style(colors.style_surface0())
        .title(title);
    if let Some((current, total)) = aq.question.progress {
        block = block.title(
            Line::from(format!(" {current}/{total} "))
                .alignment(Alignment::Right)
                .style(colors.text_muted()),
        );
    }
    let inner = block.inner(area);
    frame.render_widget(block, area);
    if inner.width == 0 || inner.height == 0 {
        return;
    }
    let body = if inner.width > 8 {
        Rect::new(
            inner.x.saturating_add(1),
            inner.y,
            inner.width.saturating_sub(2),
            inner.height,
        )
    } else {
        inner
    };

    use std::hash::{Hash, Hasher};
    let mut content_hash = std::collections::hash_map::DefaultHasher::new();
    aq.question.text.hash(&mut content_hash);
    let key = (
        body.width,
        crate::colors::theme_fingerprint(colors),
        content_hash.finish(),
    );
    if geometry.cache_key.as_ref() != Some(&key) {
        geometry.detail_lines = formatted_details(&aq.question.text, body.width, colors);
        geometry.cache_key = Some(key);
    }
    // Always reserve an actionable choice and the hint before assigning details.
    let detail_h = (geometry.detail_lines.len().min(6) as u16).min(body.height.saturating_sub(3));
    let detail_area = Rect::new(body.x, body.y, body.width, detail_h);
    let detail_max = geometry
        .detail_lines
        .len()
        .saturating_sub(detail_h as usize)
        .min(u16::MAX as usize) as u16;
    geometry.details = detail_area;
    geometry.detail_max = detail_max;
    frame.render_widget(
        Paragraph::new(geometry.detail_lines.clone()).scroll((aq.detail_scroll.min(detail_max), 0)),
        detail_area,
    );

    let divider_h = u16::from(detail_h > 0);
    if divider_h > 0 {
        frame.render_widget(
            Paragraph::new("─".repeat(body.width as usize)).style(colors.border_muted()),
            Rect::new(body.x, detail_area.bottom(), body.width, 1),
        );
    }
    let choices_area = Rect::new(
        body.x,
        detail_area.bottom().saturating_add(divider_h),
        body.width,
        body.height.saturating_sub(detail_h + divider_h + 1),
    );
    let digits = aq.total_items.max(1).to_string().len();
    let mut lines = Vec::new();
    let mut ranges = Vec::new();
    let mut focus = (0, 0);
    for index in 0..aq.total_items {
        let focused = index == aq.cursor_pos;
        let style = if focused {
            colors.text_primary_bold().bg(colors.c_bg_surface2())
        } else {
            colors.text_primary()
        };
        let indicator = if aq.question.multi_select && index < aq.n_real {
            if aq.checked[index] { "[✓]" } else { "[ ]" }
        } else if focused {
            "(•)"
        } else {
            "( )"
        };
        let prefix = format!(
            "{} {:>digits$}. {indicator} ",
            if focused { "›" } else { " " },
            index + 1
        );
        let start = lines.len();
        let label = if index == aq.submit_idx {
            "Confirm selection".to_string()
        } else if index == aq.other_idx {
            if aq.custom_text.is_empty() {
                let mut placeholder = if approval {
                    "Type your instructions"
                } else {
                    "Type your own answer"
                }
                .to_string();
                if focused {
                    placeholder.push('▌');
                }
                placeholder
            } else {
                let byte = aq
                    .custom_text
                    .char_indices()
                    .nth(aq.custom_cursor_pos)
                    .map(|(i, _)| i)
                    .unwrap_or(aq.custom_text.len());
                let mut text = aq.custom_text.clone();
                if focused {
                    text.insert(byte, '▌');
                }
                text
            }
        } else {
            aq.question.options[index].label.clone()
        };
        append_choice(&mut lines, &label, &prefix, body.width, style, accent);
        let label_end = lines.len();
        if index < aq.n_real && !aq.question.options[index].description.is_empty() {
            append_choice(
                &mut lines,
                &aq.question.options[index].description,
                &" ".repeat(prefix.chars().count()),
                body.width,
                colors.text_muted(),
                accent,
            );
        }
        if focused {
            // For long free text keep the actual insertion cursor in view.
            let cursor_row = if index == aq.other_idx {
                lines[start..label_end]
                    .iter()
                    .position(|line| line.to_string().contains('▌'))
                    .unwrap_or(0)
            } else {
                0
            };
            focus = (start + cursor_row, label_end);
        }
        ranges.push((index, start, lines.len()));
    }
    let visible = choices_area.height as usize;
    let offset = if visible == 0 {
        0
    } else {
        let last = if focus.1.saturating_sub(focus.0) <= visible {
            focus.1
        } else {
            focus.0 + 1
        };
        last.saturating_sub(visible)
            .min(lines.len().saturating_sub(visible))
    };
    for (index, start, end) in ranges {
        let top = start.max(offset);
        let bottom = end.min(offset + visible);
        if top < bottom {
            geometry.choices.push((
                index,
                Rect::new(
                    choices_area.x,
                    choices_area.y + (top - offset) as u16,
                    choices_area.width,
                    (bottom - top) as u16,
                ),
            ));
        }
    }
    frame.render_widget(
        Paragraph::new(lines).scroll((offset.min(u16::MAX as usize) as u16, 0)),
        choices_area,
    );
    let cancel = if approval { "deny" } else { "cancel" };
    let hint = if body.width < 42 {
        format!("↑↓ Enter · Esc {cancel}")
    } else if body.width < 70 {
        format!("↑↓ Enter · Esc {cancel} · PgUp/PgDn details")
    } else if aq.question.multi_select {
        format!("Space toggle · Enter confirm · Esc {cancel} · PgUp/PgDn details")
    } else {
        format!("↑↓/Tab · 1-N quick pick · Enter select · Esc {cancel} · PgUp/PgDn details")
    };
    frame.render_widget(
        Paragraph::new(hint).style(colors.text_muted()),
        Rect::new(body.x, body.bottom().saturating_sub(1), body.width, 1),
    );
}
