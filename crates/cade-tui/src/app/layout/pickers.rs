use crate::colors::{ThemeColorsExt, ColorDefExt, BorderStyleExt};
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
                .fg(colors.thinking_minimal.to_ratatui())
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
        Paragraph::new(lines).style(Style::default().bg(colors.bg_surface1.to_ratatui())),
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
    [tc.primary.to_ratatui(), tc.success.to_ratatui(), tc.error.to_ratatui(), tc.warning.to_ratatui(), tc.bg_surface2.to_ratatui()]
        .iter()
        .map(|&fg| Span::styled("█", Style::default().fg(fg)))
        .collect()
}

/// Resolve theme colors for picker swatches.
///
/// Built-ins are resolved via `builtin_by_name` (single source of truth).
/// Custom themes use `from_theme` so their actual colors appear in swatches (B1).
fn resolve_theme_colors(t: &cade_core::resources::themes::Theme) -> TC {
    TC::builtin_by_name(&t.name)
        .unwrap_or_else(|| TC::from_theme(t))
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
        Style::default().fg(if is_sel { colors.primary.to_ratatui() } else { colors.text_dim.to_ratatui() }),
    );

    // Swatch cell
    let mut swatch_spans = vec![cursor_span];
    swatch_spans.extend(theme_swatches(tc));
    swatch_spans.push(Span::raw(" "));
    let swatch_line = ratatui::text::Text::from(Line::from(swatch_spans));

    // Name cell
    let name_style = if is_sel {
        Style::default()
            .fg(colors.text_primary.to_ratatui())
            .add_modifier(Modifier::BOLD)
    } else {
        colors.text_primary()
    };
    let name_cell = Cell::from(Span::styled(t.name.clone(), name_style));

    // U2: variant badge after name
    let variant_badge = match t.variant.as_deref() {
        Some("dark") => " [dark]",
        Some("light") => " [light]",
        _ => "",
    };
    let badge_cell = Cell::from(Span::styled(
        variant_badge.to_string(),
        Style::default().fg(colors.text_dim.to_ratatui()).add_modifier(Modifier::DIM),
    ));

    // Description cell
    let desc = t.description.as_deref().unwrap_or("").to_string();
    let desc_cell = Cell::from(Span::styled(desc, colors.text_muted()));

    let row_style = if is_sel {
        colors.selected_bg_style()
    } else {
        Style::default()
    };

    Row::new(vec![
        Cell::from(swatch_line),
        name_cell,
        badge_cell,
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

    // Dim backdrop behind the overlay
    super::helpers::render_backdrop(frame, area, colors);

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
                .fg(colors.primary.to_ratatui())
                .add_modifier(Modifier::BOLD),
        ))
        .border_style(colors.border_base())
        .style(Style::default().bg(colors.bg_surface0.to_ratatui()));

    let inner_table_area = outer_block.inner(table_area);
    frame.render_widget(outer_block, table_area);

    // -- B5/A2: derive builtin names from the single source of truth
    let builtin_names: Vec<&str> = cade_core::resources::themes::ThemeColors::builtin_listing()
        .iter()
        .map(|(n, _, _)| *n)
        .collect();

    // -- B2+A1: simplified selection + flat_cursor that accounts for header rows.
    // We iterate filtered_indices once, partitioning into built-in and custom,
    // computing is_sel purely from tp.cursor (an index into filtered_indices).
    let mut builtin_rows: Vec<Row> = Vec::new();
    let mut custom_rows: Vec<Row> = Vec::new();

    for (fi_pos, &orig_idx) in tp.filtered_indices.iter().enumerate() {
        let t = &tp.themes[orig_idx];
        let is_sel = fi_pos == tp.cursor;
        // B1: resolve actual theme colors — from_theme for custom themes
        let tc = resolve_theme_colors(t);
        let row = theme_row(t, is_sel, colors, &tc);
        if builtin_names.contains(&t.name.as_str()) {
            builtin_rows.push(row);
        } else {
            custom_rows.push(row);
        }
    }

    // Assemble rows with section headers, tracking the selected flat index
    let mut all_rows: Vec<Row> = Vec::new();
    let mut flat_cursor: Option<usize> = None;
    let mut flat_idx = 0usize;

    if !builtin_rows.is_empty() {
        all_rows.push(
            Row::new(vec![
                Cell::from(Span::styled(
                    "  Built-in",
                    Style::default()
                        .fg(colors.accent_dim.to_ratatui())
                        .add_modifier(Modifier::BOLD | Modifier::DIM),
                )),
                Cell::from(""),
                Cell::from(""),
                Cell::from(""),
            ])
            .style(Style::default().bg(colors.bg_surface0.to_ratatui())),
        );
        flat_idx += 1; // header row

        // Find selected row among builtins
        let mut bi = 0usize;
        for (fi_pos, &orig_idx) in tp.filtered_indices.iter().enumerate() {
            if builtin_names.contains(&tp.themes[orig_idx].name.as_str()) {
                if fi_pos == tp.cursor {
                    flat_cursor = Some(flat_idx + bi);
                }
                bi += 1;
            }
        }
        flat_idx += builtin_rows.len();
        all_rows.extend(builtin_rows);
    }
    if !custom_rows.is_empty() {
        all_rows.push(
            Row::new(vec![
                Cell::from(Span::styled(
                    "  Custom",
                    Style::default()
                        .fg(colors.accent_dim.to_ratatui())
                        .add_modifier(Modifier::BOLD | Modifier::DIM),
                )),
                Cell::from(""),
                Cell::from(""),
                Cell::from(""),
            ])
            .style(Style::default().bg(colors.bg_surface0.to_ratatui())),
        );
        flat_idx += 1; // header row

        // Find selected row among custom
        if flat_cursor.is_none() {
            let mut ci = 0usize;
            for (fi_pos, &orig_idx) in tp.filtered_indices.iter().enumerate() {
                if !builtin_names.contains(&tp.themes[orig_idx].name.as_str()) {
                    if fi_pos == tp.cursor {
                        flat_cursor = Some(flat_idx + ci);
                    }
                    ci += 1;
                }
            }
        }
        all_rows.extend(custom_rows);
    }

    // swatch cell width = 3 (cursor) + 5 (swatches) + 1 (space) = 9
    let table = Table::new(
        all_rows,
        [
            Constraint::Length(9),
            Constraint::Length(22),
            Constraint::Length(8),   // U2: variant badge
            Constraint::Min(10),
        ],
    )
    .column_spacing(1)
    .style(Style::default().bg(colors.bg_surface0.to_ratatui()));

    let mut ts = ratatui::widgets::TableState::default()
        .with_selected(flat_cursor);
    frame.render_stateful_widget(table, inner_table_area, &mut ts);

    // -- Filter box
    let filter_block = Block::default()
        .borders(Borders::ALL)
        .border_type(colors.border_style.to_ratatui())
        // U3: shortened title to fit narrow pickers
        .title(Span::styled(
            " ↑↓ nav · Enter ok · Esc cancel · type to filter ",
            Style::default().fg(colors.text_muted.to_ratatui()).add_modifier(Modifier::DIM),
        ))
        .border_style(colors.border_base())
        .style(Style::default().bg(colors.bg_surface1.to_ratatui()));
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
        let t = cade_core::resources::themes::Theme {
            name: "dark".to_string(),
            description: None,
            author: None,
            variant: None,
            vars: Default::default(),
            colors: Default::default(),
            source: std::path::PathBuf::new(),
        };
        let tc = resolve_theme_colors(&t);
        assert_ne!(tc.primary.to_ratatui(), ratatui::style::Color::Reset);
    }

    #[test]
    fn test_builtin_colors_unknown_falls_back_to_from_theme() {
        let t = cade_core::resources::themes::Theme {
            name: "totally-unknown-theme".to_string(),
            description: None,
            author: None,
            variant: None,
            vars: Default::default(),
            colors: Default::default(),
            source: std::path::PathBuf::new(),
        };
        // B1: custom themes use from_theme(), not dark() fallback
        let tc = resolve_theme_colors(&t);
        // from_theme on default tokens produces default colors — just verify it doesn't panic
        let _ = tc.primary.to_ratatui();
    }

    #[test]
    fn test_builtin_colors_all_named() {
        for (name, _, _) in cade_core::resources::themes::ThemeColors::builtin_listing() {
            let t = cade_core::resources::themes::Theme {
                name: name.to_string(),
                description: None,
                author: None,
                variant: None,
                vars: Default::default(),
                colors: Default::default(),
                source: std::path::PathBuf::new(),
            };
            let tc = resolve_theme_colors(&t);
            assert_ne!(tc.primary.to_ratatui(), ratatui::style::Color::Reset, "theme {name} primary must not be Reset");
        }
    }
}

// endregion: --- Tests
