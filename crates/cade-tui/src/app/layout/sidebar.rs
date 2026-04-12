use crate::app::*;
use crate::app::layout::cursor::input_mode_badge;
use crate::app::layout::toast::context_severity_color;
#[allow(clippy::too_many_arguments)]
pub(crate) fn render_sidebar(
    frame: &mut Frame,
    area: Rect,
    mode: PermissionMode,
    input_mode: InputMode,
    agent_name: &str,
    model: &str,
    reasoning_effort: Option<&str>,
    cwd: &str,
    context_pct: Option<u8>,
    turn_count: u32,
    token_history: &[u8],
    queued_count: usize,
    thinking_text: Option<&str>,
    thinking_elapsed: Option<std::time::Duration>,
    active_plan: Option<&PlanState>,
    copy_mode: bool,
    colors: &ThemeColors,
) {
    let inner = Block::default()
        .borders(Borders::LEFT)
        .border_style(Style::default().fg(colors.border))
        .padding(Padding::new(1, 1, 0, 0))
        .inner(area);
    frame.render_widget(
        Block::default()
            .borders(Borders::LEFT)
            .border_style(Style::default().fg(colors.border)),
        area,
    );

    let (input_badge, _) = input_mode_badge(input_mode, colors);
    let mode_name = format!("{mode}");
    let context_text = context_pct
        .map(|p| format!("{p}%"))
        .unwrap_or_else(|| "—".to_string());
    let think_text = if let Some(elapsed) = thinking_elapsed {
        let secs = elapsed.as_secs();
        format!(
            "{} · {}s",
            thinking_text.unwrap_or("thinking…"),
            secs.max(1)
        )
    } else if queued_count > 0 {
        format!("idle · {queued_count} queued")
    } else {
        "idle".to_string()
    };
    let plan_summary = if let Some(plan) = active_plan {
        let done = plan.steps.iter().filter(|s| s.is_done).count();
        let total = plan.steps.len();
        if total > 0 {
            format!("{done}/{total} complete")
        } else {
            "none".to_string()
        }
    } else {
        "none".to_string()
    };

    let lines = vec![
        Line::from(Span::styled(
            " Session ",
            Style::default()
                .fg(colors.overlay_title)
                .add_modifier(Modifier::BOLD),
        )),
        Line::from(vec![
            Span::styled(" agent   ", Style::default().fg(colors.muted)),
            Span::styled(
                truncate_str(agent_name, 28),
                Style::default().fg(colors.text),
            ),
        ]),
        Line::from(vec![
            Span::styled(" model   ", Style::default().fg(colors.muted)),
            Span::styled(truncate_str(model, 28), Style::default().fg(colors.text)),
        ]),
        Line::from(vec![
            Span::styled(" cwd     ", Style::default().fg(colors.muted)),
            Span::styled(truncate_str(cwd, 28), Style::default().fg(colors.text)),
        ]),
        Line::from(""),
        Line::from(Span::styled(
            " Status ",
            Style::default()
                .fg(colors.overlay_title)
                .add_modifier(Modifier::BOLD),
        )),
        Line::from(vec![
            Span::styled(" mode    ", Style::default().fg(colors.muted)),
            Span::styled(mode_name, Style::default().fg(mode_sep_color(mode, colors))),
        ]),
        Line::from(vec![
            Span::styled(" input   ", Style::default().fg(colors.muted)),
            Span::styled(
                input_badge,
                Style::default().fg(colors.badge_fg).bg(colors.badge_bg),
            ),
        ]),
        Line::from(vec![
            Span::styled(" context ", Style::default().fg(colors.muted)),
            Span::styled(
                context_text,
                Style::default().fg(context_severity_color(context_pct, colors)),
            ),
        ]),
        Line::from(vec![
            Span::styled(" queue   ", Style::default().fg(colors.muted)),
            Span::styled(queued_count.to_string(), Style::default().fg(colors.text)),
        ]),
        Line::from(vec![
            Span::styled(" turns   ", Style::default().fg(colors.muted)),
            Span::styled(turn_count.to_string(), Style::default().fg(colors.text)),
        ]),
        Line::from(vec![
            Span::styled(" copy    ", Style::default().fg(colors.muted)),
            Span::styled(
                if copy_mode { "ON" } else { "OFF" },
                Style::default().fg(if copy_mode {
                    colors.success
                } else {
                    colors.dim
                }),
            ),
        ]),
        if let Some(reason) = reasoning_effort {
            Line::from(vec![
                Span::styled(" reason  ", Style::default().fg(colors.muted)),
                Span::styled(reason.to_string(), Style::default().fg(colors.warning)),
            ])
        } else {
            Line::from(vec![
                Span::styled(" reason  ", Style::default().fg(colors.muted)),
                Span::styled("default", Style::default().fg(colors.warning)),
            ])
        },
        Line::from(""),
        Line::from(Span::styled(
            " Activity ",
            Style::default()
                .fg(colors.overlay_title)
                .add_modifier(Modifier::BOLD),
        )),
        Line::from(Span::styled(
            truncate_str(&think_text, 36),
            Style::default().fg(colors.thinking_text),
        )),
        Line::from(""),
        Line::from(Span::styled(
            " Plan ",
            Style::default()
                .fg(colors.overlay_title)
                .add_modifier(Modifier::BOLD),
        )),
        Line::from(vec![
            Span::styled(" todos   ", Style::default().fg(colors.muted)),
            Span::styled(plan_summary, Style::default().fg(colors.text)),
        ]),
        Line::from(""),
        Line::from(Span::styled(
            " Keys ",
            Style::default()
                .fg(colors.overlay_title)
                .add_modifier(Modifier::BOLD),
        )),
        Line::from(Span::styled(
            " Ctrl+C abort / clear",
            Style::default().fg(colors.muted),
        )),
        Line::from(Span::styled(
            " Ctrl+O expand/collapse all",
            Style::default().fg(colors.muted),
        )),
        Line::from(Span::styled(
            " Tab cycle permissions",
            Style::default().fg(colors.muted),
        )),
        Line::from(Span::styled(
            " ↑/↓ command history",
            Style::default().fg(colors.muted),
        )),
        Line::from(Span::styled(
            " @ file picker",
            Style::default().fg(colors.muted),
        )),
        Line::from(Span::styled(
            " / commands menu",
            Style::default().fg(colors.muted),
        )),
        Line::from(Span::styled(
            " Ctrl+P command palette",
            Style::default().fg(colors.muted),
        )),
    ];

    // Split inner into text content area + sparkline area at bottom.
    let sparkline_h: u16 = if token_history.len() >= 2 { 4 } else { 0 };
    let [text_area, spark_area] = ratatui::layout::Layout::vertical([
        ratatui::layout::Constraint::Fill(1),
        ratatui::layout::Constraint::Length(sparkline_h),
    ])
    .areas(inner);

    frame.render_widget(Paragraph::new(lines).wrap(Wrap { trim: false }), text_area);

    // -- Sparkline: context window usage trend
    if sparkline_h > 0 {
        let data: Vec<u64> = token_history.iter().map(|&p| p as u64).collect();
        let spark = ratatui::widgets::Sparkline::default()
            .block(
                Block::default()
                    .borders(Borders::TOP)
                    .border_style(Style::default().fg(colors.border_muted))
                    .title(Span::styled(
                        " Context % ",
                        Style::default()
                            .fg(colors.overlay_title)
                            .add_modifier(Modifier::BOLD),
                    ))
                    .padding(Padding::new(1, 1, 0, 0)),
            )
            .data(&data)
            .max(100)
            .style(Style::default().fg(context_severity_color(context_pct, colors)));
        frame.render_widget(spark, spark_area);
    }
}

