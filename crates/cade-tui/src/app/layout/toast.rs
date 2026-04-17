use crate::app::*;
pub(crate) fn render_toast(
    frame: &mut Frame,
    main_area: Rect,
    toast: &Toast,
    colors: &ThemeColors,
) {
    let width = (toast.message.chars().count() as u16 + 6)
        .clamp(20, main_area.width.saturating_sub(2).max(20));
    let rect = Rect {
        x: main_area.x + main_area.width.saturating_sub(width),
        y: main_area.y,
        width,
        height: 3,
    };
    let (fg, border) = match toast.level {
        ToastLevel::Info => (colors.text_primary, colors.primary),
        ToastLevel::Success => (colors.text_primary, colors.success),
        ToastLevel::Warning => (colors.text_primary, colors.warning),
        ToastLevel::Error => (colors.text_primary, colors.error),
    };
    frame.render_widget(ratatui::widgets::Clear, rect);
    frame.render_widget(
        Paragraph::new(Line::from(vec![
            Span::raw(" "),
            Span::styled(
                truncate_str(&toast.message, rect.width.saturating_sub(4) as usize),
                Style::default().fg(fg),
            ),
        ]))
        .block(
            Block::default()
                .borders(Borders::ALL)
                .border_type(colors.border_style.to_ratatui())
                .border_style(Style::default().fg(border))
                .style(Style::default().bg(colors.bg_surface2)),
        ),
        rect,
    );
}

pub(crate) fn context_severity_color(context_pct: Option<u8>, colors: &ThemeColors) -> RC {
    match context_pct {
        Some(p) if p >= 90 => colors.error,
        Some(p) if p >= 80 => colors.warning,
        Some(_) => colors.text_muted,
        None => colors.text_dim,
    }
}

// -- Input helpers (ported from input.rs)
