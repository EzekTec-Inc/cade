use crate::colors::{ThemeColorsExt, ColorDefExt, BorderStyleExt};
use crate::colors::ThemeColors;
use ratatui::{
    Frame,
    layout::{Constraint, Layout, Rect},
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Clear, Paragraph},
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
        .border_type(colors.border_style.to_ratatui())
        .style(Style::default().bg(colors.bg_surface2.to_ratatui()))
        .border_style(colors.border_base())
        .title(Line::from(vec![
            Span::raw(" "),
            Span::styled(
                title.to_string(),
                Style::default()
                    .fg(colors.primary.to_ratatui())
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
                    .fg(colors.text_muted.to_ratatui())
                    .add_modifier(Modifier::DIM),
            ),
        ])),
        area,
    );
}

pub fn overlay_selected_style(colors: &ThemeColors) -> Style {
    Style::default()
        .bg(colors.bg_surface1.to_ratatui())
        .fg(colors.primary.to_ratatui())
}

pub fn overlay_section_style(colors: &ThemeColors) -> Style {
    Style::default()
        .fg(colors.md_heading.to_ratatui())
        .add_modifier(Modifier::BOLD)
}

pub fn overlay_muted_style(colors: &ThemeColors) -> Style {
    colors.text_muted()
}

pub fn overlay_badge_style(colors: &ThemeColors) -> Style {
    Style::default()
        .fg(colors.text_primary.to_ratatui())
        .bg(colors.bg_surface2.to_ratatui())
        .add_modifier(Modifier::BOLD)
}
