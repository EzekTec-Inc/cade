use crate::app::*;
use crate::colors::ThemeColors as TC;
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Cell, Row};

// region:    --- @ file picker

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
        colors.border_base(),
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
        Span::styled(no_match, colors.text_muted()),
    ]));

    // Match entries — fill remaining rows (minus sep + header already pushed)
    let max_entries = (area.height as usize).saturating_sub(lines.len());
    for (i, m) in pk.matches.iter().take(max_entries).enumerate() {
        let selected = i == pk.cursor;
        let (glyph, style) = if selected {
            ("❯", colors.text_primary().add_modifier(Modifier::BOLD))
        } else {
            (" ", colors.text_muted())
        };
        lines.push(Line::from(Span::styled(format!(" {glyph} {m}"), style)));
    }

    frame.render_widget(
        Paragraph::new(lines).style(Style::default().bg(colors.bg_surface1)),
        area,
    );
}

// endregion: --- @ file picker

// region:    --- Theme picker

/// The five swatch colors rendered as coloured block characters before each
/// theme name in the picker. Gives instant visual recognition.

/// Build the 5-cell swatch spans for a `ThemeColors`.
/// Returns a `Vec<Span>` of coloured `█` characters.
fn theme_swatches(tc: &TC) -> Vec<Span<'static>> {
    [tc.primary, tc.success, tc.error, tc.warning, tc.bg_surface2]
        .iter()
        .map(|&fg| Span::styled("█", Style::default().fg(fg)))
        .collect()
}

/// Resolve a built-in theme name to a `ThemeColors`.
/// Falls back to `dark()` for unknown names (custom JSON themes).
fn builtin_colors(name: &str) -> TC {
    match name {
        "light"              => TC::light(),
        "catppuccin-mocha"   => TC::catppuccin_mocha(),
        "catppuccin-latte"   => TC::catppuccin_latte(),
        "tokyo-night"        => TC::tokyo_night(),
        _                    => TC::dark(),
    }
}


/// One theme row: `  ▶/  <swatches> <name>  <description>`.
fn theme_row<'a>(
    t: &cade_core::resources::themes::Theme,
    is_sel: bool,
    colors: &ThemeColors,
    tc: &TC,
) -> Row<'a> {
    let cursor_span = Span::styled(
        if is_sel { " ❯ " } else { "   " },
        Style::default().fg(if is_sel { colors.primary } else { colors.text_dim }),
    );

    // Swatch cell
    let mut swatch_spans = vec![cursor_span];
    swatch_spans.extend(theme_swatches(tc));
    swatch_spans.push(Span::raw(" "));
    let swatch_line = ratatui::text::Text::from(Line::from(swatch_spans));

    // Name cell
    let name_style = if is_sel {
        Style::default()
            .fg(colors.text_primary)
            .add_modifier(Modifier::BOLD)
    } else {
        colors.text_primary()
    };
    let name_cell = Cell::from(Span::styled(t.name.clone(), name_style));

    // Description cell
    let desc = t.description.as_deref().unwrap_or("").to_string();
    let desc_cell = Cell::from(Span::styled(desc, colors.text_muted()));

    let row_style = if is_sel {
        Style::default().bg(colors.bg_surface1)
    } else {
        Style::default()
    };

    Row::new(vec![
        Cell::from(swatch_line),
        name_cell,
        desc_cell,
    ])
    .style(row_style)
}

pub(crate) fn render_theme_picker(
    frame: &mut ratatui::Frame,
    tp: &ThemePickerState,
    area: ratatui::layout::Rect,
    colors: &ThemeColors,
) {
    use ratatui::layout::{Constraint, Direction, Layout};
    use ratatui::widgets::{Block, Borders, Clear, Paragraph, Table};

    if area.height == 0 {
        return;
    }

    frame.render_widget(Clear, area);

    // Split into table area + filter box
    let [table_area, filter_area] = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Min(4), Constraint::Length(3)])
        .areas(area);

    // -- Outer block
    let total = tp.filtered_indices.len();
    let title = format!(
        " Themes ({} of {}) · live preview active ",
        total,
        tp.themes.len()
    );
    let outer_block = Block::default()
        .borders(Borders::ALL)
        .border_type(colors.border_style.to_ratatui())
        .title(Span::styled(
            title,
            Style::default()
                .fg(colors.primary)
                .add_modifier(Modifier::BOLD),
        ))
        .border_style(colors.border_base())
        .style(Style::default().bg(colors.bg_surface0));

    let inner_table_area = outer_block.inner(table_area);
    frame.render_widget(outer_block, table_area);

    // -- Group themes: built-in first, then custom
    let builtin_names = ["dark", "light", "catppuccin-mocha", "catppuccin-latte", "tokyo-night"];
    let mut builtin_rows: Vec<(usize, Row)> = Vec::new();
    let mut custom_rows:  Vec<(usize, Row)> = Vec::new();

    // Track which flat cursor index maps to which theme for TableState
    let mut flat_cursor: Option<usize> = None;
    let mut flat_idx = 0usize;

    // Built-in group
    for &orig_idx in &tp.filtered_indices {
        let t = &tp.themes[orig_idx];
        if builtin_names.contains(&t.name.as_str()) {
            let is_sel = builtin_rows.len() + 1 /* header */ == tp.cursor && custom_rows.is_empty()
                || flat_cursor.is_none() && {
                    // count position in full filtered list
                    let pos = tp.filtered_indices.iter().position(|&i| i == orig_idx).unwrap_or(usize::MAX);
                    pos == tp.cursor
                };
            if flat_cursor.is_none() && tp.filtered_indices.iter().position(|&i| i == orig_idx) == Some(tp.cursor) {
                flat_cursor = Some(flat_idx);
            }
            let tc = builtin_colors(&t.name);
            builtin_rows.push((flat_idx, theme_row(t, is_sel, colors, &tc)));
            flat_idx += 1;
        }
    }

    // Custom group
    for &orig_idx in &tp.filtered_indices {
        let t = &tp.themes[orig_idx];
        if !builtin_names.contains(&t.name.as_str()) {
            let is_sel = tp.filtered_indices.iter().position(|&i| i == orig_idx) == Some(tp.cursor);
            if flat_cursor.is_none() && is_sel {
                flat_cursor = Some(flat_idx);
            }
            let tc = builtin_colors(&t.name); // falls back to dark() for custom
            custom_rows.push((flat_idx, theme_row(t, is_sel, colors, &tc)));
            flat_idx += 1;
        }
    }

    // Assemble rows with section headers
    let mut all_rows: Vec<Row> = Vec::new();
    if !builtin_rows.is_empty() {
        all_rows.push(
            Row::new(vec![
                Cell::from(Span::styled(
                    "  Built-in",
                    Style::default()
                        .fg(colors.accent_dim)
                        .add_modifier(Modifier::BOLD | Modifier::DIM),
                )),
                Cell::from(""),
                Cell::from(""),
            ])
            .style(Style::default().bg(colors.bg_surface0)),
        );
        all_rows.extend(builtin_rows.into_iter().map(|(_, r)| r));
    }
    if !custom_rows.is_empty() {
        all_rows.push(
            Row::new(vec![
                Cell::from(Span::styled(
                    "  Custom",
                    Style::default()
                        .fg(colors.accent_dim)
                        .add_modifier(Modifier::BOLD | Modifier::DIM),
                )),
                Cell::from(""),
                Cell::from(""),
            ])
            .style(Style::default().bg(colors.bg_surface0)),
        );
        all_rows.extend(custom_rows.into_iter().map(|(_, r)| r));
    }

    // swatch cell width = 3 (cursor) + 5 (swatches) + 1 (space) = 9
    let table = Table::new(
        all_rows,
        [
            Constraint::Length(9),
            Constraint::Length(22),
            Constraint::Min(10),
        ],
    )
    .column_spacing(1)
    .style(Style::default().bg(colors.bg_surface0));

    let mut ts = ratatui::widgets::TableState::default()
        .with_selected(flat_cursor);
    frame.render_stateful_widget(table, inner_table_area, &mut ts);

    // -- Filter box
    let filter_block = Block::default()
        .borders(Borders::ALL)
        .border_type(colors.border_style.to_ratatui())
        .title(Span::styled(
            " Filter — type to search  ↑↓ navigate  Enter confirm  Esc cancel ",
            Style::default().fg(colors.text_muted).add_modifier(Modifier::DIM),
        ))
        .border_style(colors.border_base())
        .style(Style::default().bg(colors.bg_surface1));
    let filter_text = Paragraph::new(format!("> {}█", tp.query))
        .block(filter_block)
        .style(colors.text_primary());
    frame.render_widget(filter_text, filter_area);
}

// endregion: --- Theme picker

// region:    --- Tests

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_theme_swatches_count() {
        let tc = TC::dark();
        let swatches = theme_swatches(&tc);
        assert_eq!(swatches.len(), 5); // primary, success, error, warning, bg_surface2
    }

    #[test]
    fn test_theme_swatches_all_colored() {
        use ratatui::style::Color;
        let tc = TC::dark();
        let swatches = theme_swatches(&tc);
        for s in &swatches {
            assert_ne!(
                s.style.fg.unwrap_or(Color::Reset),
                Color::Reset,
                "swatch must have an explicit fg color"
            );
        }
    }

    #[test]
    fn test_builtin_colors_dark() {
        let tc = builtin_colors("dark");
        assert_ne!(tc.primary, ratatui::style::Color::Reset);
    }

    #[test]
    fn test_builtin_colors_unknown_falls_back_to_dark() {
        let tc = builtin_colors("totally-unknown-theme");
        let dark = TC::dark();
        assert_eq!(tc.primary, dark.primary);
    }

    #[test]
    fn test_builtin_colors_all_named() {
        for name in ["dark", "light", "catppuccin-mocha", "catppuccin-latte", "tokyo-night"] {
            let tc = builtin_colors(name);
            assert_ne!(tc.primary, ratatui::style::Color::Reset, "theme {name} primary must not be Reset");
        }
    }
}

// endregion: --- Tests
