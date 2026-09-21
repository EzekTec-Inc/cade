use crate::app::*;
use crate::colors::ThemeColors as TC;
use crate::colors::ThemeColorsExt;
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

    // Match entries with sliding scroll window
    let max_entries = (inner_area.height as usize)
        .saturating_sub(lines.len())
        .max(1);
    let (start_idx, end_idx) = picker_scroll_window(pk.cursor, pk.matches.len(), max_entries);

    for (abs_i, m) in pk.matches[start_idx..end_idx].iter().enumerate() {
        let current_idx = start_idx + abs_i;
        let selected = current_idx == pk.cursor;
        let (glyph, style) = if selected {
            (
                "❯",
                Style::default()
                    .bg(colors.c_bg_surface1())
                    .fg(colors.c_primary())
                    .add_modifier(Modifier::BOLD),
            )
        } else {
            (" ", colors.text_muted())
        };
        // Fill width for background selection effect
        let text = format!(" {glyph} {m}");
        let padded_text = format!("{:width$}", text, width = inner_area.width as usize);
        lines.push(Line::from(Span::styled(padded_text, style)));
    }

    frame.render_widget(Paragraph::new(lines), inner_area);
}

// endregion: --- @ file picker

// region:    --- Theme picker

/// The five swatch colors rendered as coloured block characters before each
/// theme name in the picker. Gives instant visual recognition.

/// Build the 5-cell swatch spans for a `ThemeColors`.
/// Returns a `Vec<Span>` of coloured `█` characters.
fn theme_swatches(tc: &TC) -> Vec<Span<'static>> {
    [
        tc.c_primary(),
        tc.c_success(),
        tc.c_error(),
        tc.c_warning(),
        tc.c_bg_surface2(),
    ]
    .iter()
    .map(|&fg| Span::styled("█", Style::default().fg(fg)))
    .collect()
}

/// One theme row: `  ▶/  <swatches> <name>  <description>`.
fn theme_row<'a>(t: &opaline::Theme, is_sel: bool, colors: &ThemeColors) -> Row<'a> {
    let cursor_span = Span::styled(
        if is_sel { " ❯ " } else { "   " },
        Style::default().fg(if is_sel {
            colors.c_primary()
        } else {
            colors.c_text_dim()
        }),
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
        Style::default()
            .fg(colors.c_text_dim())
            .add_modifier(Modifier::DIM),
    ));

    // Description cell
    let desc = t.meta.description.clone().unwrap_or_default();
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

    // -- B5/A2: derive builtin names from the single source of truth
    let builtin_names: Vec<String> = opaline::list_available_themes()
        .into_iter()
        .map(|info| info.name)
        .collect();

    let w = (area.width / 2).max(40).min(area.width.saturating_sub(4));
    let has_builtins = tp
        .filtered_indices
        .iter()
        .any(|&i| builtin_names.contains(&tp.themes[i].meta.name));
    let has_custom = tp
        .filtered_indices
        .iter()
        .any(|&i| !builtin_names.contains(&tp.themes[i].meta.name));
    let header_rows = has_builtins as u16 + has_custom as u16;
    let max_visible = area.height.saturating_sub(8);
    let n = (tp.filtered_indices.len() as u16 + header_rows)
        .max(1)
        .min(max_visible);
    let h = (n + 4).clamp(5, area.height.saturating_sub(4));

    let show_preview = area.width >= 80;
    let (picker_rect, preview_rect) = if show_preview {
        let total_w = (area.width.saturating_sub(4)).min(115);
        let left_w = (total_w * 54 / 100).max(42);
        let right_w = total_w.saturating_sub(left_w);
        let x = area.x + (area.width.saturating_sub(total_w)) / 2;
        let y = area.y + (area.height.saturating_sub(h)) / 2;
        (
            ratatui::layout::Rect {
                x,
                y,
                width: left_w,
                height: h,
            },
            Some(ratatui::layout::Rect {
                x: x + left_w,
                y,
                width: right_w,
                height: h,
            }),
        )
    } else {
        (
            ratatui::layout::Rect {
                x: area.x + (area.width.saturating_sub(w)) / 2,
                y: area.y + (area.height.saturating_sub(h)) / 2,
                width: w,
                height: h,
            },
            None,
        )
    };

    // Dim backdrop behind the overlay
    super::helpers::render_backdrop(frame, area, colors);

    frame.render_widget(Clear, picker_rect);

    // Split into table area + filter box
    let [table_area, filter_area] = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Min(4), Constraint::Length(3)])
        .areas(picker_rect);

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
            Constraint::Length(8), // U2: variant badge
            Constraint::Min(10),
        ],
    )
    .column_spacing(1)
    .style(Style::default().bg(colors.c_bg_surface0()));

    let mut ts = ratatui::widgets::TableState::default().with_selected(flat_cursor);
    frame.render_stateful_widget(table, inner_table_area, &mut ts);

    // -- Filter box
    let filter_block = Block::default()
        .borders(Borders::ALL)
        .border_type(colors.c_border_style())
        // U3: shortened title to fit narrow pickers
        .title(Span::styled(
            " ↑↓ nav · Enter ok · Esc cancel · type to filter ",
            Style::default()
                .fg(colors.c_text_muted())
                .add_modifier(Modifier::DIM),
        ))
        .border_style(colors.border_accent())
        .style(Style::default().bg(colors.c_bg_surface1()));
    let filter_text = Paragraph::new(format!("> {}█", tp.query))
        .block(filter_block)
        .style(colors.text_primary());
    frame.render_widget(filter_text, filter_area);

    // -- Visual Theme Preview Card (when wide enough)
    if let Some(pr) = preview_rect {
        let active_theme = if let Some(&orig_idx) = tp.filtered_indices.get(tp.cursor) {
            &tp.themes[orig_idx]
        } else {
            colors
        };
        frame.render_widget(Clear, pr);
        render_theme_preview(frame, pr, active_theme, colors);
    }
}

/// Render a comprehensive visual preview card of a theme demonstrating
/// typography, contrast, status cues, syntax, and tool card states.
fn render_theme_preview(
    frame: &mut ratatui::Frame,
    area: ratatui::layout::Rect,
    theme: &opaline::Theme,
    colors: &ThemeColors,
) {
    use ratatui::widgets::{Block, Borders, Paragraph};

    let preview_block = Block::default()
        .borders(Borders::ALL)
        .border_type(colors.c_border_style())
        .title(Span::styled(
            format!(" Preview: {} ", theme.meta.name),
            Style::default()
                .fg(theme.c_primary())
                .add_modifier(Modifier::BOLD),
        ))
        .border_style(theme.border_focus())
        .style(Style::default().bg(theme.c_bg_base()));

    let inner = preview_block.inner(area);
    frame.render_widget(preview_block, area);

    let contrast_ratio = if let (Some(fg), Some(bg)) =
        (theme.try_color("text.primary"), theme.try_color("bg.base"))
    {
        cade_core::resources::calculate_contrast_ratio((fg.r, fg.g, fg.b), (bg.r, bg.g, bg.b))
    } else {
        4.5
    };

    let mut lines = Vec::new();

    // 1. Accessibility & Health Status
    let contrast_span = if contrast_ratio >= 4.5 {
        Span::styled(
            format!("✓ WCAG AA ({:.1}:1)", contrast_ratio),
            theme.success(),
        )
    } else {
        Span::styled(
            format!("! Low Contrast ({:.1}:1)", contrast_ratio),
            theme.warning(),
        )
    };
    lines.push(Line::from(vec![
        Span::styled("Accessibility: ", theme.text_muted()),
        contrast_span,
    ]));

    // 2. Typography & Hierarchy
    lines.push(Line::from(vec![
        Span::styled("Text: ", theme.text_muted()),
        Span::styled("Primary ", theme.text_primary()),
        Span::styled("Muted ", theme.text_muted()),
        Span::styled("Dim", theme.text_dim()),
    ]));

    // 3. Status Badges & Non-color glyph cues
    lines.push(Line::from(vec![
        Span::styled("Status: ", theme.text_muted()),
        Span::styled("[✓] Ok ", theme.success()),
        Span::styled("[!] Warn ", theme.warning()),
        Span::styled("[✗] Error ", theme.error()),
    ]));

    // 4. Selection & Surfaces
    lines.push(Line::from(vec![
        Span::styled("Surface: ", theme.text_muted()),
        Span::styled(" Base ", theme.style_base()),
        Span::styled(" Panel ", theme.style_surface0()),
        Span::styled(" Selected ", theme.selected_bg_style()),
    ]));

    // 5. Diff Rows
    lines.push(Line::from(vec![
        Span::styled("Diff: ", theme.text_muted()),
        Span::styled("+ added ", theme.diff_added()),
        Span::styled("- removed ", theme.diff_removed()),
        Span::styled("  context", theme.diff_context()),
    ]));

    // 6. Code & Syntax Highlighting
    lines.push(Line::from(vec![
        Span::styled("Syntax: ", theme.text_muted()),
        Span::styled("fn ", theme.syntax_keyword()),
        Span::styled("run", theme.syntax_function()),
        Span::styled("() -> ", theme.syntax_punctuation()),
        Span::styled("Result", theme.syntax_type()),
        Span::styled(" { ", theme.syntax_punctuation()),
        Span::styled("\"ok\"", theme.syntax_string()),
        Span::styled(" }", theme.syntax_punctuation()),
    ]));

    // 7. Tool Results
    lines.push(Line::from(vec![
        Span::styled("Tools: ", theme.text_muted()),
        Span::styled(" bash ", theme.badge()),
        Span::styled(" ✓ success ", theme.tool_success_bg_style()),
        Span::styled(" ✗ fail ", theme.tool_error_bg_style()),
    ]));

    let p = Paragraph::new(lines).style(theme.style_base());
    frame.render_widget(p, inner);
}

/// Calculate a sliding scroll window that keeps `cursor` visible within `visible` rows.
pub fn picker_scroll_window(cursor: usize, total: usize, visible: usize) -> (usize, usize) {
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

// endregion: --- Theme picker

// region:    --- Tests

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_picker_scroll_window_small_list() {
        assert_eq!(picker_scroll_window(0, 5, 10), (0, 5));
        assert_eq!(picker_scroll_window(3, 5, 10), (0, 5));
    }

    #[test]
    fn test_picker_scroll_window_scrolling() {
        // Cursor at top
        assert_eq!(picker_scroll_window(0, 30, 8), (0, 8));
        // Cursor in middle
        assert_eq!(picker_scroll_window(10, 30, 8), (6, 14));
        // Cursor at bottom
        assert_eq!(picker_scroll_window(29, 30, 8), (22, 30));
    }

    #[test]
    fn test_theme_swatches_and_contrast() {
        let toml = r##"
        [meta]
        name = "preview-test"
        variant = "dark"
        [palette]
        bg = "#111111"
        fg = "#eeeeee"
        [tokens]
        "bg.base" = "bg"
        "text.primary" = "fg"
        "##;
        let theme = opaline::load_from_str(toml, None).unwrap();
        let swatches = theme_swatches(&theme);
        assert_eq!(
            swatches.len(),
            5,
            "theme_swatches should produce 5 color spans"
        );

        let fg = theme.try_color("text.primary").unwrap();
        let bg = theme.try_color("bg.base").unwrap();
        let ratio =
            cade_core::resources::calculate_contrast_ratio((fg.r, fg.g, fg.b), (bg.r, bg.g, bg.b));
        assert!(ratio >= 4.5, "contrast ratio should be WCAG AA compliant");
    }
}
