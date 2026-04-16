//! Render the command palette overlay.

use crate::app::command_palette::CommandPaletteState;
use crate::colors::ThemeColors;
use ratatui::{
    Frame,
    layout::{Constraint, Layout, Rect},
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::{Block, BorderType, Borders, Clear, Paragraph},
};

/// Render the command palette as a centered overlay.
pub(crate) fn render_command_palette(
    frame: &mut Frame,
    cp: &CommandPaletteState,
    area: Rect,
    colors: &ThemeColors,
) {
    // Size the overlay: 60% width, up to 50 chars wide min, max 80
    let w = (area.width * 3 / 5).max(40).min(area.width.saturating_sub(4)).min(80);
    let max_visible = 12usize;
    let item_count = cp.filtered.len().min(max_visible);
    // Height = 3 (border top + search row + border/padding) + items + 2 (hint + border bottom)
    let h = ((item_count as u16) + 5).clamp(7, area.height.saturating_sub(4));

    let r = Rect {
        x: area.x + (area.width.saturating_sub(w)) / 2,
        y: area.y + (area.height.saturating_sub(h)) / 3,  // bias toward top 1/3
        width: w,
        height: h,
    };

    // Clear the background
    frame.render_widget(Clear, r);

    // Outer block
    let block = Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .title(Line::from(vec![
            Span::raw(" "),
            Span::styled(
                "Command Palette",
                Style::default()
                    .fg(colors.primary)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::raw(" "),
        ]))
        .border_style(colors.border_base())
        .style(Style::default().bg(colors.bg_surface2));

    let inner = block.inner(r);
    frame.render_widget(block, r);

    if inner.height < 3 || inner.width < 10 {
        return;
    }

    // Split inner: search bar (1 row) + separator (1 row) + results (fill) + hint (1 row)
    let [search_area, sep_area, results_area, hint_area] = Layout::vertical([
        Constraint::Length(1),
        Constraint::Length(1),
        Constraint::Fill(1),
        Constraint::Length(1),
    ])
    .areas(inner);

    // -- Search input
    let search_text = format!("> {}█", cp.query);
    let no_match = if cp.filtered.is_empty() && !cp.query.is_empty() {
        "  (no matches)"
    } else {
        ""
    };
    frame.render_widget(
        Paragraph::new(Line::from(vec![
            Span::styled(
                search_text,
                Style::default()
                    .fg(colors.text_primary)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled(no_match, colors.text_muted()),
        ])),
        search_area,
    );

    // -- Separator
    let sep = "─".repeat(sep_area.width as usize);
    frame.render_widget(
        Paragraph::new(Span::styled(sep, colors.border_base())),
        sep_area,
    );

    // -- Results list
    let visible_results = results_area.height as usize;
    // Calculate window to keep cursor in view
    let (start, end) = scroll_window(cp.cursor, cp.filtered.len(), visible_results);

    let mut lines: Vec<Line<'static>> = Vec::with_capacity(visible_results);
    for (display_idx, &cmd_idx) in cp.filtered[start..end].iter().enumerate() {
        let absolute_idx = start + display_idx;
        let is_selected = absolute_idx == cp.cursor;
        let cmd = &cp.commands[cmd_idx];

        let max_label_w = (results_area.width as usize).saturating_sub(4) / 2;
        let max_desc_w = (results_area.width as usize).saturating_sub(max_label_w + 6);

        let label = if cmd.label.len() > max_label_w {
            format!("{}…", &cmd.label[..max_label_w - 1])
        } else {
            cmd.label.to_string()
        };

        let desc = if cmd.description.len() > max_desc_w {
            format!("{}…", &cmd.description[..max_desc_w - 1])
        } else {
            cmd.description.to_string()
        };

        let glyph = if is_selected { "▶" } else { " " };

        let (label_style, desc_style, glyph_style) = if is_selected {
            (
                Style::default()
                    .fg(colors.primary)
                    .bg(colors.bg_surface1)
                    .add_modifier(Modifier::BOLD),
                Style::default()
                    .fg(colors.primary)
                    .bg(colors.bg_surface1),
                Style::default()
                    .fg(colors.primary)
                    .bg(colors.bg_surface1),
            )
        } else {
            (
                colors.primary(),
                colors.text_muted(),
                colors.text_dim(),
            )
        };

        // Pad label to fixed width for alignment
        let label_padded = format!("{:<width$}", label, width = max_label_w);

        // Section tag — shown when query is active to help orient results
        let section_tag = if !cp.query.is_empty() {
            format!("  [{}]", cmd.section)
        } else {
            String::new()
        };
        let section_style = if is_selected {
            Style::default()
                .fg(colors.text_dim)
                .bg(colors.bg_surface1)
        } else {
            colors.text_dim()
        };

        lines.push(Line::from(vec![
            Span::styled(format!(" {} ", glyph), glyph_style),
            Span::styled(label_padded, label_style),
            Span::styled("  ", if is_selected {
                Style::default().bg(colors.bg_surface1)
            } else {
                Style::default()
            }),
            Span::styled(desc, desc_style),
            Span::styled(section_tag, section_style),
        ]));
    }

    frame.render_widget(Paragraph::new(lines), results_area);

    // -- Hint bar
    let total = cp.commands.len();
    let shown = cp.filtered.len();
    let hint = if shown < total {
        format!(" ↑↓ Navigate  Enter Select  Esc Cancel  ({}/{})", shown, total)
    } else {
        format!(" ↑↓ Navigate  Enter Select  Esc Cancel  ({} commands)", total)
    };
    frame.render_widget(
        Paragraph::new(Span::styled(
            hint,
            Style::default()
                .fg(colors.text_muted)
                .add_modifier(Modifier::DIM),
        )),
        hint_area,
    );
}

/// Calculate a scroll window that keeps `cursor` visible within `visible` rows.
fn scroll_window(cursor: usize, total: usize, visible: usize) -> (usize, usize) {
    if total <= visible {
        return (0, total);
    }
    let half = visible / 2;
    let start = if cursor <= half {
        0
    } else if cursor + half >= total {
        total.saturating_sub(visible)
    } else {
        cursor.saturating_sub(half)
    };
    let end = (start + visible).min(total);
    (start, end)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_scroll_window_small_list() {
        assert_eq!(scroll_window(0, 5, 10), (0, 5));
        assert_eq!(scroll_window(3, 5, 10), (0, 5));
    }

    #[test]
    fn test_scroll_window_cursor_at_start() {
        assert_eq!(scroll_window(0, 20, 10), (0, 10));
        assert_eq!(scroll_window(2, 20, 10), (0, 10));
    }

    #[test]
    fn test_scroll_window_cursor_at_end() {
        let (start, end) = scroll_window(19, 20, 10);
        assert_eq!(end, 20);
        assert_eq!(start, 10);
    }

    #[test]
    fn test_scroll_window_cursor_middle() {
        let (start, end) = scroll_window(10, 30, 10);
        assert!(start <= 10);
        assert!(end >= 10);
        assert_eq!(end - start, 10);
    }
}
