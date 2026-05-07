use crate::colors::ThemeColorsExt;
use crate::app::*;
use crate::colors::ThemeColors as TC;
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Cell, Row};

// region:    --- @ file picker

/// Render the `@` file picker as a floating overlay.
pub(crate) fn render_picker(frame: &mut Frame, pk: &PickerState, area: Rect, colors: &ThemeColors) {
    if area.height == 0 {
        return;
    }
    
    // Draw a proper shell overlay centered on screen
    let inner_area = crate::overlay::render_overlay_shell(frame, area, "Select File", colors);

    let mut lines: Vec<Line<'static>> = Vec::new();

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
                .fg(colors.c_thinking_minimal())
                .add_modifier(Modifier::BOLD),
        ),
        Span::styled(no_match, colors.text_muted()),
    ]));

    // Separator under header
    lines.push(Line::from(Span::styled(
        "╌".repeat(inner_area.width as usize),
        colors.border_muted(),
    )));

    // Match entries
    let max_entries = (inner_area.height as usize).saturating_sub(lines.len());
    for (i, m) in pk.matches.iter().take(max_entries).enumerate() {
        let selected = i == pk.cursor;
        let (glyph, style) = if selected {
            ("❯", Style::default().bg(colors.c_bg_surface1()).fg(colors.c_primary()).add_modifier(Modifier::BOLD))
        } else {
            (" ", colors.text_muted())
        };
        // Fill width for background selection effect
        let text = format!(" {glyph} {m}");
        let padded_text = format!("{:width$}", text, width = inner_area.width as usize);
        lines.push(Line::from(Span::styled(padded_text, style)));
    }

    frame.render_widget(
        Paragraph::new(lines),
        inner_area,
    );
}

// endregion: --- @ file picker

// region:    --- Theme picker

/// The five swatch colors rendered as coloured block characters before each
/// theme name in the picker. Gives instant visual recognition.

/// Build the 5-cell swatch spans for a `ThemeColors`.
/// Returns a `Vec<Span>` of coloured `█` characters.
fn theme_swatches(tc: &TC) -> Vec<Span<'static>> {
    [tc.c_primary(), tc.c_success(), tc.c_error(), tc.c_warning(), tc.c_bg_surface2()]
        .iter()
        .map(|&fg| Span::styled("█", Style::default().fg(fg)))
        .collect()
}

/// One theme row: `  ▶/  <swatches> <name>  <description>`.
fn theme_row<'a>(
    t: &opaline::Theme,
    is_sel: bool,
    colors: &ThemeColors,
) -> Row<'a> {
    let cursor_span = Span::styled(
        if is_sel { " ❯ " } else { "   " },
        Style::default().fg(if is_sel { colors.c_primary() } else { colors.c_text_dim() }),
    );

    // Swatch cell
    let mut swatch_spans = vec![cursor_span];
    swatch_spans.extend(theme_swatches(t));
    swatch_spans.push(Span::raw(" "));
    let swatch_line = ratatui::text::Text::from(Line::from(swatch_spans));

    // Name cell
    let name_style = if is_sel {
        Style::default()
            .fg(colors.c_text_primary())
            .add_modifier(Modifier::BOLD)
    } else {
        colors.text_primary()
    };
    let name_cell = Cell::from(Span::styled(t.meta.name.clone(), name_style));

    // U2: variant badge after name
    let variant_badge = match t.meta.variant {
        opaline::ThemeVariant::Dark => " [dark]",
        opaline::ThemeVariant::Light => " [light]",
    };
    let badge_cell = Cell::from(Span::styled(
        variant_badge.to_string(),
        Style::default().fg(colors.c_text_dim()).add_modifier(Modifier::DIM),
    ));

    // Description cell
    let desc = t.meta.description.as_deref().unwrap_or("").to_string();
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

    // -- B5/A2: derive builtin names from the single source of truth
    let builtin_names: Vec<String> = opaline::list_available_themes()
        .into_iter()
        .map(|info| info.name)
        .collect();

    let w = (area.width / 2).max(40).min(area.width.saturating_sub(4));
    let has_builtins = tp.filtered_indices.iter().any(|&i| builtin_names.contains(&tp.themes[i].meta.name));
    let has_custom = tp.filtered_indices.iter().any(|&i| !builtin_names.contains(&tp.themes[i].meta.name));
    let header_rows = has_builtins as u16 + has_custom as u16;
    let max_visible = area.height.saturating_sub(8);
    let n = (tp.filtered_indices.len() as u16 + header_rows).max(1).min(max_visible);
    let h = (n + 4).clamp(5, area.height.saturating_sub(4));

    let r = ratatui::layout::Rect {
        x: area.x + (area.width.saturating_sub(w)) / 2,
        y: area.y + (area.height.saturating_sub(h)) / 2,
        width: w,
        height: h,
    };

    frame.render_widget(Clear, r);

    // Split into table area + filter box
    let [table_area, filter_area] = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Min(4), Constraint::Length(3)])
        .areas(r);

    // -- Outer block
    let total = tp.filtered_indices.len();
    let title = format!(
        " Themes ({} of {}) · live preview active ",
        total,
        tp.themes.len()
    );
    let outer_block = Block::default()
        .borders(Borders::ALL)
        .border_type(colors.c_border_style())
        .title(Span::styled(
            title,
            Style::default()
                .fg(colors.c_primary())
                .add_modifier(Modifier::BOLD),
        ))
        .border_style(colors.border_accent())
        .style(Style::default().bg(colors.c_bg_surface0()));

    let inner_table_area = outer_block.inner(table_area);
    frame.render_widget(outer_block, table_area);

    // -- B2+A1: simplified selection + flat_cursor that accounts for header rows.
    // We iterate filtered_indices once, partitioning into built-in and custom,
    // computing is_sel purely from tp.cursor (an index into filtered_indices).
    let mut builtin_rows: Vec<Row> = Vec::new();
    let mut custom_rows: Vec<Row> = Vec::new();

    for (fi_pos, &orig_idx) in tp.filtered_indices.iter().enumerate() {
        let t = &tp.themes[orig_idx];
        let is_sel = fi_pos == tp.cursor;
        let row = theme_row(t, is_sel, colors);
        if builtin_names.contains(&t.meta.name) {
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
                        .fg(colors.c_text_dim())
                        .add_modifier(Modifier::BOLD | Modifier::DIM),
                )),
                Cell::from(""),
                Cell::from(""),
                Cell::from(""),
            ])
            .style(Style::default().bg(colors.c_bg_surface0())),
        );
        flat_idx += 1; // header row

        // Find selected row among builtins
        let mut bi = 0usize;
        for (fi_pos, &orig_idx) in tp.filtered_indices.iter().enumerate() {
            if builtin_names.contains(&tp.themes[orig_idx].meta.name) {
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
                        .fg(colors.c_text_dim())
                        .add_modifier(Modifier::BOLD | Modifier::DIM),
                )),
                Cell::from(""),
                Cell::from(""),
                Cell::from(""),
            ])
            .style(Style::default().bg(colors.c_bg_surface0())),
        );
        flat_idx += 1; // header row

        // Find selected row among custom
        if flat_cursor.is_none() {
            let mut ci = 0usize;
            for (fi_pos, &orig_idx) in tp.filtered_indices.iter().enumerate() {
                if !builtin_names.contains(&tp.themes[orig_idx].meta.name) {
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
    .style(Style::default().bg(colors.c_bg_surface0()));

    let mut ts = ratatui::widgets::TableState::default()
        .with_selected(flat_cursor);
    frame.render_stateful_widget(table, inner_table_area, &mut ts);

    // -- Filter box
    let filter_block = Block::default()
        .borders(Borders::ALL)
        .border_type(colors.c_border_style())
        // U3: shortened title to fit narrow pickers
        .title(Span::styled(
            " ↑↓ nav · Enter ok · Esc cancel · type to filter ",
            Style::default().fg(colors.c_text_muted()).add_modifier(Modifier::DIM),
        ))
        .border_style(colors.border_accent())
        .style(Style::default().bg(colors.c_bg_surface1()));
    let filter_text = Paragraph::new(format!("> {}█", tp.query))
        .block(filter_block)
        .style(colors.text_primary());
    frame.render_widget(filter_text, filter_area);
}

// endregion: --- Theme picker

// region:    --- Tests


