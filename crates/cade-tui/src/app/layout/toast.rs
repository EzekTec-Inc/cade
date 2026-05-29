use crate::app::*;
use crate::colors::ThemeColorsExt;
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
        ToastLevel::Info => (colors.c_text_primary(), colors.c_primary()),
        ToastLevel::Success => (colors.c_text_primary(), colors.c_success()),
        ToastLevel::Warning => (colors.c_text_primary(), colors.c_warning()),
        ToastLevel::Error => (colors.c_text_primary(), colors.c_error()),
    };

    let text_area = Rect {
        x: rect.x,
        y: rect.y,
        width: rect.width,
        height: 2,
    };
    let progress_area = Rect {
        x: rect.x,
        y: rect.y + 2,
        width: rect.width,
        height: 1,
    };

    frame.render_widget(ratatui::widgets::Clear, rect);

    // Render top, left, and right borders of the toast card
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
                .borders(Borders::TOP | Borders::LEFT | Borders::RIGHT)
                .border_type(colors.c_border_style())
                .border_style(Style::default().fg(border))
                .style(Style::default().bg(colors.c_bg_surface2())),
        ),
        text_area,
    );

    // Calculate time-decay remaining percentage
    let elapsed = toast.created_at.elapsed();
    let total = toast.ttl;
    let pct_remaining = if total.as_secs_f64() > 0.0 {
        ((total.as_secs_f64() - elapsed.as_secs_f64()) / total.as_secs_f64()).clamp(0.0, 1.0)
    } else {
        0.0
    };

    let border_type = colors.c_border_style();
    let (bl, br) = match border_type {
        ratatui::widgets::BorderType::Rounded => ("╰", "╯"),
        ratatui::widgets::BorderType::Double => ("╚", "╝"),
        ratatui::widgets::BorderType::Thick => ("┗", "┛"),
        _ => ("└", "┘"),
    };

    let width_inner = rect.width.saturating_sub(2) as usize;
    let filled_w = (pct_remaining * width_inner as f64).round() as usize;
    let empty_w = width_inner.saturating_sub(filled_w);

    let progress_line = Line::from(vec![
        Span::styled(bl, Style::default().fg(border)),
        Span::styled("█".repeat(filled_w), Style::default().fg(border)),
        Span::styled("─".repeat(empty_w), Style::default().fg(colors.c_text_dim())),
        Span::styled(br, Style::default().fg(border)),
    ]);

    // Draw the bottom border as a decaying progress bar
    frame.render_widget(
        Paragraph::new(progress_line).style(Style::default().bg(colors.c_bg_surface2())),
        progress_area,
    );
}

pub(crate) fn context_severity_color(context_pct: Option<u8>, colors: &ThemeColors) -> RC {
    match context_pct {
        Some(p) if p >= 90 => colors.c_error(),
        Some(p) if p >= 80 => colors.c_warning(),
        Some(_) => colors.c_text_muted(),
        None => colors.c_text_dim(),
    }
}

// -- Input helpers (ported from input.rs)
