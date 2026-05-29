use crate::app::layout::cursor::input_mode_badge;
use crate::app::layout::helpers::mode_sep_color;
use crate::app::layout::toast::context_severity_color;
use crate::app::*;
use crate::colors::ThemeColorsExt;

/// All data the sidebar needs to render — constructed once per frame in
/// `render_frame` and passed by reference to `render_sidebar`.
/// Eliminates the 21-argument free-function signature.
pub(crate) struct SidebarState<'a> {
    pub mode: PermissionMode,
    pub input_mode: InputMode,
    pub agent_name: &'a str,
    pub model: &'a str,
    pub reasoning_effort: Option<&'a str>,
    pub cwd: &'a str,
    pub context_pct: Option<u8>,
    pub turn_count: u32,
    pub token_history: &'a [u8],
    pub queued_count: usize,
    pub thinking_text: Option<&'a str>,
    pub thinking_elapsed: Option<std::time::Duration>,
    pub active_plan: Option<&'a PlanState>,
    pub mouse_capture_disabled: bool,
    pub session_tokens: (u64, u64),
}

impl<'a> SidebarState<'a> {
    /// Format the activity / thinking status line.
    pub(crate) fn format_activity(&self) -> String {
        if let Some(elapsed) = self.thinking_elapsed {
            let secs = elapsed.as_secs();
            format!(
                "{} · {}s",
                self.thinking_text.unwrap_or("thinking…"),
                secs.max(1)
            )
        } else if self.queued_count > 0 {
            format!("idle · {} queued", self.queued_count)
        } else {
            "idle".to_string()
        }
    }

    /// Format the plan summary line.
    pub(crate) fn format_plan_summary(&self) -> String {
        match self.active_plan {
            Some(plan) => {
                let done = plan.steps.iter().filter(|s| s.is_done).count();
                let total = plan.steps.len();
                if total > 0 {
                    format!("{done}/{total} complete")
                } else {
                    "none".to_string()
                }
            }
            None => "none".to_string(),
        }
    }

    /// Format the context-window percentage label.
    pub(crate) fn format_context(&self) -> String {
        self.context_pct
            .map(|p| format!("{p}%"))
            .unwrap_or_else(|| "—".to_string())
    }
}

pub(crate) fn render_sidebar(
    frame: &mut Frame,
    area: Rect,
    state: &SidebarState<'_>,
    colors: &ThemeColors,
) {
    let inner = Block::default()
        .borders(Borders::LEFT)
        .border_style(colors.border_base())
        .style(Style::default().bg(colors.c_bg_surface0()))
        .padding(Padding::new(1, 1, 0, 0))
        .inner(area);
    frame.render_widget(
        Block::default()
            .borders(Borders::LEFT)
            .border_style(colors.border_base())
            .style(Style::default().bg(colors.c_bg_surface0())),
        area,
    );

    let (input_badge, _) = input_mode_badge(state.input_mode, colors);
    let mode_name = format!("{}", state.mode);
    let context_text = state.format_context();
    let think_text = state.format_activity();
    let plan_summary = state.format_plan_summary();

    // B7: Derive truncation width from actual inner area instead of
    // hardcoded 28.  Label prefix is 9 chars (" agent   "), so the
    // value column gets the rest.
    let val_w = (inner.width as usize).saturating_sub(10);

    // Calculate cost and budget gauge
    let (prompt_tok, comp_tok) = state.session_tokens;
    // Standard blended rate ($5/M prompt, $15/M completion)
    let cost = (prompt_tok as f64 * 5.0 / 1_000_000.0) + (comp_tok as f64 * 15.0 / 1_000_000.0);
    let cost_limit: f64 = std::env::var("CADE_MAX_SESSION_COST_USD")
        .ok()
        .and_then(|v| v.parse::<f64>().ok())
        .unwrap_or(2.0); // default $2.00
    let cost_pct = if cost_limit > 0.0 {
        ((cost / cost_limit) * 100.0).clamp(0.0, 100.0)
    } else {
        0.0
    };

    let bar_width = val_w.min(16).max(8);
    let filled_chars = ((cost_pct / 100.0) * bar_width as f64).round() as usize;
    let empty_chars = bar_width.saturating_sub(filled_chars);
    let bar_color = if cost_pct < 50.0 {
        colors.c_success()
    } else if cost_pct < 85.0 {
        colors.c_warning()
    } else {
        colors.c_error()
    };

    let lines = vec![
        Line::from(Span::styled(
            " Session ",
            Style::default()
                .fg(colors.c_primary())
                .add_modifier(Modifier::BOLD),
        )),
        Line::from(vec![
            Span::styled(" agent   ", colors.text_muted()),
            Span::styled(truncate_str(state.agent_name, val_w), colors.text_primary()),
        ]),
        Line::from(vec![
            Span::styled(" model   ", colors.text_muted()),
            Span::styled(truncate_str(state.model, val_w), colors.text_primary()),
        ]),
        Line::from(vec![
            Span::styled(" cwd     ", colors.text_muted()),
            Span::styled(truncate_str(state.cwd, val_w), colors.text_primary()),
        ]),
        Line::from(vec![
            Span::styled(" cost    ", colors.text_muted()),
            Span::styled(format!("${:.4}", cost), colors.text_primary()),
        ]),
        Line::from(vec![
            Span::styled(" budget  ", colors.text_muted()),
            Span::styled("[", colors.text_muted()),
            Span::styled("█".repeat(filled_chars), Style::default().fg(bar_color)),
            Span::styled("░".repeat(empty_chars), colors.text_dim()),
            Span::styled("]", colors.text_muted()),
            Span::styled(format!(" {:.0}%", cost_pct), colors.text_muted()),
        ]),
        Line::from(""),
        Line::from(Span::styled(
            " Status ",
            Style::default()
                .fg(colors.c_primary())
                .add_modifier(Modifier::BOLD),
        )),
        Line::from(vec![
            Span::styled(" mode    ", colors.text_muted()),
            Span::styled(
                mode_name,
                Style::default().fg(mode_sep_color(state.mode, colors)),
            ),
        ]),
        Line::from(vec![
            Span::styled(" input   ", colors.text_muted()),
            Span::styled(
                input_badge,
                colors.text_primary().bg(colors.c_bg_surface2()),
            ),
        ]),
        Line::from(vec![
            Span::styled(" context ", colors.text_muted()),
            Span::styled(
                context_text,
                Style::default().fg(context_severity_color(state.context_pct, colors)),
            ),
        ]),
        Line::from(vec![
            Span::styled(" queue   ", colors.text_muted()),
            Span::styled(state.queued_count.to_string(), colors.text_primary()),
        ]),
        Line::from(vec![
            Span::styled(" turns   ", colors.text_muted()),
            Span::styled(state.turn_count.to_string(), colors.text_primary()),
        ]),
        Line::from(vec![
            Span::styled(" mouse   ", colors.text_muted()),
            Span::styled(
                if state.mouse_capture_disabled {
                    "FREE"
                } else {
                    "CAPTURED"
                },
                Style::default().fg(if state.mouse_capture_disabled {
                    colors.c_success()
                } else {
                    colors.c_text_dim()
                }),
            ),
        ]),
        if let Some(reason) = state.reasoning_effort {
            Line::from(vec![
                Span::styled(" reason  ", colors.text_muted()),
                Span::styled(reason.to_string(), colors.warning()),
            ])
        } else {
            Line::from(vec![
                Span::styled(" reason  ", colors.text_muted()),
                Span::styled("default", colors.warning()),
            ])
        },
        Line::from(""),
        Line::from(Span::styled(
            " Activity ",
            Style::default()
                .fg(colors.c_primary())
                .add_modifier(Modifier::BOLD),
        )),
        Line::from(Span::styled(
            truncate_str(&think_text, val_w),
            colors.text_muted(),
        )),
        Line::from(""),
        Line::from(Span::styled(
            " Plan ",
            Style::default()
                .fg(colors.c_primary())
                .add_modifier(Modifier::BOLD),
        )),
        Line::from(vec![
            Span::styled(" todos   ", colors.text_muted()),
            Span::styled(plan_summary, colors.text_primary()),
        ]),
        Line::from(""),
        Line::from(Span::styled(
            " Keys ",
            Style::default()
                .fg(colors.c_primary())
                .add_modifier(Modifier::BOLD),
        )),
        Line::from(Span::styled(" Ctrl+C abort / clear", colors.text_muted())),
        Line::from(Span::styled(
            " Ctrl+O expand/collapse all",
            colors.text_muted(),
        )),
        Line::from(Span::styled(" Tab cycle permissions", colors.text_muted())),
        Line::from(Span::styled(" ↑/↓ command history", colors.text_muted())),
        Line::from(Span::styled(" @ file picker", colors.text_muted())),
        Line::from(Span::styled(" / commands menu", colors.text_muted())),
        Line::from(Span::styled(" Ctrl+P command palette", colors.text_muted())),
        Line::from(Span::styled(" Ctrl+T toggle plan", colors.text_muted())),
    ];

    // Split inner into text content area + sparkline area at bottom.
    let sparkline_h: u16 = if state.token_history.len() >= 2 { 4 } else { 0 };
    let [text_area, spark_area] = ratatui::layout::Layout::vertical([
        ratatui::layout::Constraint::Fill(1),
        ratatui::layout::Constraint::Length(sparkline_h),
    ])
    .areas(inner);

    frame.render_widget(Paragraph::new(lines).wrap(Wrap { trim: false }), text_area);

    // -- Sparkline: context window usage trend
    if sparkline_h > 0 {
        let data: Vec<u64> = state.token_history.iter().map(|&p| p as u64).collect();
        let spark = ratatui::widgets::Sparkline::default()
            .block(
                Block::default()
                    .borders(Borders::TOP)
                    .border_style(colors.border_base())
                    .title(Span::styled(
                        " Context % ",
                        Style::default()
                            .fg(colors.c_primary())
                            .add_modifier(Modifier::BOLD),
                    ))
                    .padding(Padding::new(1, 1, 0, 0)),
            )
            .data(&data)
            .max(100)
            .style(Style::default().fg(context_severity_color(state.context_pct, colors)));
        frame.render_widget(spark, spark_area);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_state<'a>() -> SidebarState<'a> {
        SidebarState {
            mode: PermissionMode::Default,
            input_mode: InputMode::Regular,
            agent_name: "test-agent",
            model: "claude-opus-4",
            reasoning_effort: None,
            cwd: "/home/user/project",
            context_pct: Some(42),
            turn_count: 7,
            token_history: &[10, 20, 42],
            queued_count: 0,
            thinking_text: None,
            thinking_elapsed: None,
            active_plan: None,
            mouse_capture_disabled: true,
            session_tokens: (1000, 200),
        }
    }

    #[test]
    fn format_context_shows_percentage() {
        let s = make_state();
        assert_eq!(s.format_context(), "42%");
    }

    #[test]
    fn format_context_none_shows_dash() {
        let mut s = make_state();
        s.context_pct = None;
        assert_eq!(s.format_context(), "—");
    }

    #[test]
    fn format_activity_idle() {
        let s = make_state();
        assert_eq!(s.format_activity(), "idle");
    }

    #[test]
    fn format_activity_queued() {
        let mut s = make_state();
        s.queued_count = 3;
        assert_eq!(s.format_activity(), "idle · 3 queued");
    }

    #[test]
    fn format_activity_thinking() {
        let mut s = make_state();
        s.thinking_elapsed = Some(std::time::Duration::from_secs(5));
        s.thinking_text = Some("calling tool");
        assert_eq!(s.format_activity(), "calling tool · 5s");
    }

    #[test]
    fn format_activity_thinking_clamps_zero_to_one() {
        let mut s = make_state();
        s.thinking_elapsed = Some(std::time::Duration::from_millis(100));
        s.thinking_text = None;
        assert_eq!(s.format_activity(), "thinking… · 1s");
    }

    #[test]
    fn format_plan_summary_none() {
        let s = make_state();
        assert_eq!(s.format_plan_summary(), "none");
    }
}
