use crate::colors::ThemeColors;
use ratatui::{
    Frame,
    layout::{Constraint, Layout, Rect},
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::{Block, BorderType, Borders, Clear, Paragraph},
};

/// Draw a standard full-screen overlay shell and return the inner content area.
pub fn render_overlay_shell(
    frame: &mut Frame,
    area: Rect,
    title: &str,
    colors: &ThemeColors,
) -> Rect {
    frame.render_widget(Clear, area);
    let block = Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .style(Style::default().bg(colors.overlay_bg))
        .border_style(Style::default().fg(colors.overlay_border))
        .title(Line::from(vec![
            Span::raw(" "),
            Span::styled(
                title.to_string(),
                Style::default()
                    .fg(colors.overlay_title)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::raw(" "),
        ]));
    let inner = block.inner(area);
    frame.render_widget(block, area);
    inner
}

/// Split the overlay interior into body + footer hint rows.
pub fn split_overlay_body(area: Rect, footer_height: u16) -> (Rect, Rect) {
    let [body, footer] =
        Layout::vertical([Constraint::Min(1), Constraint::Length(footer_height)]).areas(area);
    (body, footer)
}

/// Render a dim hint/status row at the bottom of an overlay.
pub fn render_overlay_hint(frame: &mut Frame, area: Rect, hint: &str, colors: &ThemeColors) {
    frame.render_widget(
        Paragraph::new(Line::from(vec![
            Span::raw(" "),
            Span::styled(
                hint.to_string(),
                Style::default()
                    .fg(colors.overlay_hint)
                    .add_modifier(Modifier::DIM),
            ),
        ])),
        area,
    );
}

pub fn overlay_selected_style(colors: &ThemeColors) -> Style {
    Style::default()
        .bg(colors.overlay_selected_bg)
        .fg(colors.overlay_selected_fg)
}

pub fn overlay_section_style(colors: &ThemeColors) -> Style {
    Style::default()
        .fg(colors.overlay_section)
        .add_modifier(Modifier::BOLD)
}

pub fn overlay_muted_style(colors: &ThemeColors) -> Style {
    Style::default().fg(colors.overlay_hint)
}

pub fn overlay_badge_style(colors: &ThemeColors) -> Style {
    Style::default()
        .fg(colors.badge_fg)
        .bg(colors.badge_bg)
        .add_modifier(Modifier::BOLD)
}
