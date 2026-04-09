use crate::app::*;
/// Render the `@` file picker as a floating overlay at the bottom of `area`.
pub(crate) fn render_picker(frame: &mut Frame, pk: &PickerState, area: Rect, colors: &ThemeColors) {
    if area.height == 0 {
        return;
    }
    let w = area.width as usize;
    let mut lines: Vec<Line<'static>> = Vec::new();

    // Top dashed separator (matches question-panel style)
    lines.push(Line::from(Span::styled(
        "╌".repeat(w),
        Style::default().fg(colors.border),
    )));

    // Header: "@ <query>" + no-match hint
    let no_match = if pk.matches.is_empty() && !pk.query.is_empty() {
        "  (no matches)"
    } else {
        ""
    };
    lines.push(Line::from(vec![
        Span::styled(
            format!(" @ {}", pk.query),
            Style::default()
                .fg(colors.thinking_minimal)
                .add_modifier(Modifier::BOLD),
        ),
        Span::styled(no_match, Style::default().fg(colors.muted)),
    ]));

    // Match entries — fill remaining rows (minus sep + header already pushed)
    let max_entries = (area.height as usize).saturating_sub(lines.len());
    for (i, m) in pk.matches.iter().take(max_entries).enumerate() {
        let selected = i == pk.cursor;
        let (glyph, style) = if selected {
            (
                "❯",
                Style::default().fg(colors.text).add_modifier(Modifier::BOLD),
            )
        } else {
            (" ", Style::default().fg(colors.muted))
        };
        lines.push(Line::from(Span::styled(format!(" {glyph} {m}"), style)));
    }

    frame.render_widget(
        Paragraph::new(lines).style(Style::default().bg(colors.tool_pending_bg)),
        area,
    );
}

pub(crate) fn render_theme_picker(
    frame: &mut ratatui::Frame,
    tp: &ThemePickerState,
    area: ratatui::layout::Rect,
    colors: &crate::colors::ThemeColors,
) {
    use ratatui::layout::Constraint;
    use ratatui::style::{Modifier, Style};
    use ratatui::widgets::{Block, Borders, Cell, Paragraph, Row, Table};

    if area.height == 0 {
        return;
    }

    let hint = " ↑↓ Navigate  Enter Select  Esc/q Cancel ".to_string();
    let rows: Vec<Row> = tp
        .filtered_indices
        .iter()
        .enumerate()
        .map(|(i, &original_idx)| {
            let t = &tp.themes[original_idx];
            let is_sel = i == tp.cursor;

            let style = if is_sel {
                Style::default()
                    .bg(colors.overlay_selected_bg)
                    .add_modifier(Modifier::BOLD)
            } else {
                Style::default()
            };

            Row::new(vec![
                Cell::from(ratatui::text::Span::styled(
                    if is_sel { "▶ " } else { "  " },
                    Style::default().fg(if is_sel {
                        colors.overlay_selected_fg
                    } else {
                        colors.overlay_hint
                    }),
                )),
                Cell::from(ratatui::text::Span::styled(
                    t.name.clone(),
                    Style::default().fg(if is_sel {
                        crate::colors::ThemeColors::dark().text
                    } else {
                        colors.text
                    }),
                )),
                Cell::from(ratatui::text::Span::styled(
                    format!("{:?}", t.source),
                    Style::default().fg(colors.overlay_hint),
                )),
            ])
            .style(style)
        })
        .collect();

    let table = Table::new(
        rows,
        [
            Constraint::Length(3),
            Constraint::Length(25),
            Constraint::Min(20),
        ],
    )
    .header(
        Row::new(vec!["", "Theme", "Source"]).style(
            Style::default()
                .fg(colors.overlay_title)
                .add_modifier(Modifier::BOLD),
        ),
    )
    .block(
        Block::default()
            .borders(Borders::ALL)
            .title(format!(" Themes {hint}"))
            .border_style(Style::default().fg(colors.overlay_border)),
    );

    let mut ts = ratatui::widgets::TableState::default().with_selected(Some(tp.cursor));

    let main_chunks = ratatui::layout::Layout::default()
        .direction(ratatui::layout::Direction::Vertical)
        .constraints([Constraint::Min(5), Constraint::Length(3)].as_ref())
        .split(area);

    frame.render_widget(ratatui::widgets::Clear, area);
    frame.render_stateful_widget(table, main_chunks[0], &mut ts);

    let filter_block = Block::default()
        .borders(Borders::ALL)
        .title(" Filter (Type to search) ")
        .border_style(Style::default().fg(colors.overlay_border));
    let filter_text = Paragraph::new(format!("> {}█", tp.query))
        .block(filter_block)
        .style(Style::default().fg(colors.text));
    frame.render_widget(filter_text, main_chunks[1]);
}

