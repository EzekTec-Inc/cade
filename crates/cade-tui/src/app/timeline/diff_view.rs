//! Side-by-Side and Stacked Diff View Engine (`DiffViewEngine`).
//!
//! Provides alignment, syntax highlighting, and layout adaptation for code diffs.

// region:    --- Imports

use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use similar::{ChangeTag, TextDiff};

use crate::colors::{ThemeColors, ThemeColorsExt};

// endregion: --- Imports

// region:    --- Types

/// Layout strategy for rendering file diffs.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum DiffLayout {
    /// Unified vertical stacked diff (+ / - lines in sequence).
    #[default]
    Stacked,
    /// Side-by-side 2-column comparison.
    SideBySide,
}

/// Aligned pair of lines for side-by-side comparison.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DiffLinePair {
    pub left: Option<String>,
    pub right: Option<String>,
    pub change: ChangeTag,
}

/// A single diff line inside a diff card hunk.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DiffHunkLine {
    pub old_lineno: Option<usize>,
    pub new_lineno: Option<usize>,
    pub change: ChangeTag,
    pub content: String,
}

/// Aggregated diff card data for an edit or patch operation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DiffCardData {
    pub file_path: String,
    pub tool_name: String,
    pub additions: usize,
    pub deletions: usize,
    pub lines: Vec<DiffHunkLine>,
}

// endregion: --- Types

// region:    --- DiffViewEngine

/// Deep module rendering side-by-side and stacked code diffs.
pub struct DiffViewEngine;

impl DiffViewEngine {
    /// Compute aligned diff line pairs using Myers diff algorithm via `similar`.
    pub fn compute_line_pairs(old_text: &str, new_text: &str) -> Vec<DiffLinePair> {
        let diff = TextDiff::from_lines(old_text, new_text);
        let mut pairs = Vec::new();

        // For simple side-by-side, walk grouped changes
        for change in diff.iter_all_changes() {
            match change.tag() {
                ChangeTag::Equal => {
                    let val = change.value().trim_end_matches(['\r', '\n']).to_string();
                    pairs.push(DiffLinePair {
                        left: Some(val.clone()),
                        right: Some(val),
                        change: ChangeTag::Equal,
                    });
                }
                ChangeTag::Delete => {
                    let val = change.value().trim_end_matches(['\r', '\n']).to_string();
                    pairs.push(DiffLinePair {
                        left: Some(val),
                        right: None,
                        change: ChangeTag::Delete,
                    });
                }
                ChangeTag::Insert => {
                    let val = change.value().trim_end_matches(['\r', '\n']).to_string();
                    // If previous was Delete with no right side, pair with it
                    if let Some(last) = pairs.last_mut()
                        && last.change == ChangeTag::Delete
                        && last.right.is_none()
                    {
                        last.right = Some(val);
                        continue;
                    }
                    pairs.push(DiffLinePair {
                        left: None,
                        right: Some(val),
                        change: ChangeTag::Insert,
                    });
                }
            }
        }

        pairs
    }

    /// Compute structured diff card data from `old_text` and `new_text`.
    pub fn compute_diff_card_data(
        tool_name: impl Into<String>,
        file_path: impl Into<String>,
        old_text: &str,
        new_text: &str,
    ) -> DiffCardData {
        let diff = TextDiff::from_lines(old_text, new_text);
        let mut lines = Vec::new();
        let mut additions = 0;
        let mut deletions = 0;

        let mut old_idx = 1;
        let mut new_idx = 1;

        for change in diff.iter_all_changes() {
            let val = change.value().trim_end_matches(['\r', '\n']).to_string();
            match change.tag() {
                ChangeTag::Equal => {
                    lines.push(DiffHunkLine {
                        old_lineno: Some(old_idx),
                        new_lineno: Some(new_idx),
                        change: ChangeTag::Equal,
                        content: val,
                    });
                    old_idx += 1;
                    new_idx += 1;
                }
                ChangeTag::Delete => {
                    deletions += 1;
                    lines.push(DiffHunkLine {
                        old_lineno: Some(old_idx),
                        new_lineno: None,
                        change: ChangeTag::Delete,
                        content: val,
                    });
                    old_idx += 1;
                }
                ChangeTag::Insert => {
                    additions += 1;
                    lines.push(DiffHunkLine {
                        old_lineno: None,
                        new_lineno: Some(new_idx),
                        change: ChangeTag::Insert,
                        content: val,
                    });
                    new_idx += 1;
                }
            }
        }

        DiffCardData {
            file_path: file_path.into(),
            tool_name: tool_name.into(),
            additions,
            deletions,
            lines,
        }
    }

    /// Parse a unified diff text (e.g. from git diff / patch) into structured `DiffCardData`.
    pub fn parse_unified_diff(
        tool_name: impl Into<String>,
        file_path: impl Into<String>,
        diff_text: &str,
    ) -> Option<DiffCardData> {
        let mut lines = Vec::new();
        let mut additions = 0;
        let mut deletions = 0;
        let mut old_cur = 1;
        let mut new_cur = 1;
        let mut found_hunk = false;

        for line in diff_text.lines() {
            if line.starts_with("@@") {
                found_hunk = true;
                if let Some((old_start, new_start)) = parse_hunk_header(line) {
                    old_cur = old_start;
                    new_cur = new_start;
                }
                continue;
            }
            if !found_hunk
                && (line.starts_with("--- ")
                    || line.starts_with("+++ ")
                    || line.starts_with("diff ")
                    || line.starts_with("index "))
            {
                continue;
            }
            if let Some(rest) = line.strip_prefix('+') {
                additions += 1;
                lines.push(DiffHunkLine {
                    old_lineno: None,
                    new_lineno: Some(new_cur),
                    change: ChangeTag::Insert,
                    content: rest.to_string(),
                });
                new_cur += 1;
            } else if let Some(rest) = line.strip_prefix('-') {
                deletions += 1;
                lines.push(DiffHunkLine {
                    old_lineno: Some(old_cur),
                    new_lineno: None,
                    change: ChangeTag::Delete,
                    content: rest.to_string(),
                });
                old_cur += 1;
            } else if let Some(rest) = line.strip_prefix(' ') {
                lines.push(DiffHunkLine {
                    old_lineno: Some(old_cur),
                    new_lineno: Some(new_cur),
                    change: ChangeTag::Equal,
                    content: rest.to_string(),
                });
                old_cur += 1;
                new_cur += 1;
            } else if found_hunk {
                lines.push(DiffHunkLine {
                    old_lineno: Some(old_cur),
                    new_lineno: Some(new_cur),
                    change: ChangeTag::Equal,
                    content: line.to_string(),
                });
                old_cur += 1;
                new_cur += 1;
            }
        }

        if additions == 0 && deletions == 0 && lines.is_empty() {
            return None;
        }

        Some(DiffCardData {
            file_path: file_path.into(),
            tool_name: tool_name.into(),
            additions,
            deletions,
            lines,
        })
    }

    /// Render an interactive, styled Code Diff Card.
    pub fn render_diff_card(
        card: &DiffCardData,
        width: u16,
        is_expanded: bool,
        colors: &ThemeColors,
    ) -> Vec<Line<'static>> {
        let card_w = width.max(20) as usize;
        let mut out = Vec::new();

        // 1. Top Header Bar: ╭─ [tool] file_path ────────────── (+N -M) ─╮
        let tool_badge = format!("[{}]", card.tool_name);
        let path_text = &card.file_path;
        let deltas = format!("(+{} -{})", card.additions, card.deletions);

        let left_part_w = 4 + tool_badge.len() + 1 + path_text.len() + 1; // `╭─ [tool] path `
        let right_part_w = deltas.len() + 4; // ` (+N -M) ─╮`
        let fill_dashes = card_w.saturating_sub(left_part_w + right_part_w).max(2);

        out.push(Line::from(vec![
            Span::styled("╭─ ", colors.border_accent()),
            Span::styled(
                tool_badge,
                Style::default()
                    .fg(colors.c_primary())
                    .add_modifier(Modifier::BOLD),
            ),
            Span::raw(" "),
            Span::styled(
                path_text.to_string(),
                Style::default()
                    .fg(colors.c_text_primary())
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled(
                format!(" {} ", "─".repeat(fill_dashes)),
                colors.border_muted(),
            ),
            Span::styled(
                format!("+{}", card.additions),
                Style::default()
                    .fg(colors.c_success())
                    .add_modifier(Modifier::BOLD),
            ),
            Span::raw(" "),
            Span::styled(
                format!("-{}", card.deletions),
                Style::default()
                    .fg(colors.c_error())
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled(" ─╮", colors.border_accent()),
        ]));

        // 2. Diff Lines (Threshold: 8 lines when collapsed)
        let max_visible = if is_expanded { usize::MAX } else { 8 };
        let total_lines = card.lines.len();
        let show_count = total_lines.min(max_visible);

        let content_max_w = card_w.saturating_sub(16).max(10);

        for line in card.lines.iter().take(show_count) {
            let old_str = line
                .old_lineno
                .map(|n| format!("{:>4}", n))
                .unwrap_or_else(|| "    ".to_string());
            let new_str = line
                .new_lineno
                .map(|n| format!("{:>4}", n))
                .unwrap_or_else(|| "    ".to_string());

            let (sign, line_color, sign_style) = match line.change {
                ChangeTag::Equal => (" ", colors.text_dim(), colors.text_dim()),
                ChangeTag::Delete => (
                    "-",
                    Style::default().fg(colors.c_error()),
                    Style::default()
                        .fg(colors.c_error())
                        .add_modifier(Modifier::BOLD),
                ),
                ChangeTag::Insert => (
                    "+",
                    Style::default().fg(colors.c_success()),
                    Style::default()
                        .fg(colors.c_success())
                        .add_modifier(Modifier::BOLD),
                ),
            };

            let truncated_content = if line.content.len() > content_max_w {
                format!("{}…", &line.content[..content_max_w.saturating_sub(1)])
            } else {
                line.content.clone()
            };

            out.push(Line::from(vec![
                Span::styled("│ ", colors.border_muted()),
                Span::styled(old_str, colors.text_dim()),
                Span::raw(" "),
                Span::styled(new_str, colors.text_dim()),
                Span::styled(" │ ", colors.border_muted()),
                Span::styled(format!("{sign} "), sign_style),
                Span::styled(truncated_content, line_color),
            ]));
        }

        // 3. Truncation Pill Badge
        if total_lines > show_count {
            let hidden = total_lines - show_count;
            let badge = format!("+ {} lines hidden · Click or Enter to expand", hidden);
            out.push(Line::from(vec![
                Span::styled("│          │   ", colors.border_muted()),
                Span::styled(
                    format!("[{badge}]"),
                    Style::default()
                        .fg(colors.c_primary())
                        .add_modifier(Modifier::BOLD),
                ),
            ]));
        }

        // 4. Bottom Border: ╰───────────────────────────────────────────────────╯
        let bot_dashes = card_w.saturating_sub(2);
        out.push(Line::from(Span::styled(
            format!("╰{}╯", "─".repeat(bot_dashes)),
            colors.border_accent(),
        )));

        out
    }

    /// Render formatted and colorized Ratatui `Line`s for the diff.
    pub fn render_diff(
        old_text: &str,
        new_text: &str,
        layout: DiffLayout,
        width: u16,
        colors: &ThemeColors,
    ) -> Vec<Line<'static>> {
        // Narrow terminals automatically fall back to Stacked
        let effective_layout = if width < 80 {
            DiffLayout::Stacked
        } else {
            layout
        };

        match effective_layout {
            DiffLayout::Stacked => Self::render_stacked(old_text, new_text, colors),
            DiffLayout::SideBySide => Self::render_side_by_side(old_text, new_text, width, colors),
        }
    }

    fn render_stacked(old_text: &str, new_text: &str, colors: &ThemeColors) -> Vec<Line<'static>> {
        let diff = TextDiff::from_lines(old_text, new_text);
        let mut lines = Vec::new();

        for change in diff.iter_all_changes() {
            let val = change.value().trim_end_matches(['\r', '\n']);
            match change.tag() {
                ChangeTag::Equal => {
                    lines.push(Line::from(vec![
                        Span::styled("  ", colors.text_dim()),
                        Span::styled(val.to_string(), colors.text_dim()),
                    ]));
                }
                ChangeTag::Delete => {
                    lines.push(Line::from(vec![
                        Span::styled(
                            "- ",
                            Style::default()
                                .fg(colors.c_error())
                                .add_modifier(Modifier::BOLD),
                        ),
                        Span::styled(val.to_string(), Style::default().fg(colors.c_error())),
                    ]));
                }
                ChangeTag::Insert => {
                    lines.push(Line::from(vec![
                        Span::styled(
                            "+ ",
                            Style::default()
                                .fg(colors.c_success())
                                .add_modifier(Modifier::BOLD),
                        ),
                        Span::styled(val.to_string(), Style::default().fg(colors.c_success())),
                    ]));
                }
            }
        }

        lines
    }

    fn render_side_by_side(
        old_text: &str,
        new_text: &str,
        width: u16,
        colors: &ThemeColors,
    ) -> Vec<Line<'static>> {
        let pairs = Self::compute_line_pairs(old_text, new_text);
        let col_w = (width.saturating_sub(5) / 2) as usize;
        let mut lines = Vec::new();

        // Header row
        let left_header = format!("{:width$}", " ORIGINAL", width = col_w);
        let right_header = format!("{:width$}", " MODIFIED", width = col_w);
        lines.push(Line::from(vec![
            Span::styled(
                left_header,
                Style::default()
                    .fg(colors.c_text_muted())
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled(" │ ", colors.border_muted()),
            Span::styled(
                right_header,
                Style::default()
                    .fg(colors.c_text_muted())
                    .add_modifier(Modifier::BOLD),
            ),
        ]));

        lines.push(Line::from(Span::styled(
            "─".repeat(width as usize),
            colors.border_muted(),
        )));

        for pair in pairs {
            let (left_str, left_style) = match pair.left {
                Some(s) => {
                    let truncated = if s.len() > col_w {
                        format!("{}…", &s[..col_w.saturating_sub(1)])
                    } else {
                        s
                    };
                    let pad = format!("{:width$}", truncated, width = col_w);
                    if pair.change == ChangeTag::Delete
                        || (pair.change == ChangeTag::Insert && pair.right.is_some())
                    {
                        (pad, Style::default().fg(colors.c_error()))
                    } else {
                        (pad, colors.text_dim())
                    }
                }
                None => (format!("{:width$}", "", width = col_w), Style::default()),
            };

            let (right_str, right_style) = match pair.right {
                Some(s) => {
                    let truncated = if s.len() > col_w {
                        format!("{}…", &s[..col_w.saturating_sub(1)])
                    } else {
                        s
                    };
                    let pad = format!("{:width$}", truncated, width = col_w);
                    if pair.change == ChangeTag::Insert || pair.change == ChangeTag::Delete {
                        (pad, Style::default().fg(colors.c_success()))
                    } else {
                        (pad, colors.text_dim())
                    }
                }
                None => (format!("{:width$}", "", width = col_w), Style::default()),
            };

            lines.push(Line::from(vec![
                Span::styled(left_str, left_style),
                Span::styled(" │ ", colors.border_muted()),
                Span::styled(right_str, right_style),
            ]));
        }

        lines
    }
}

fn parse_hunk_header(line: &str) -> Option<(usize, usize)> {
    let trimmed = line.trim_start_matches('@').trim();
    let parts: Vec<&str> = trimmed.split_whitespace().collect();
    if parts.len() >= 2 {
        let old_part = parts[0].trim_start_matches('-');
        let new_part = parts[1].trim_start_matches('+');
        let old_start = old_part.split(',').next()?.parse::<usize>().ok()?;
        let new_start = new_part.split(',').next()?.parse::<usize>().ok()?;
        return Some((old_start, new_start));
    }
    None
}

// endregion: --- DiffViewEngine

// region:    --- Tests

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_diff_line_pairs_computation() {
        let old = "fn main() {\n    println!(\"old\");\n}";
        let new = "fn main() {\n    println!(\"new\");\n}";

        let pairs = DiffViewEngine::compute_line_pairs(old, new);
        assert_eq!(pairs.len(), 3);
        assert_eq!(pairs[0].change, ChangeTag::Equal);
        assert_eq!(pairs[1].left, Some("    println!(\"old\");".to_string()));
        assert_eq!(pairs[1].right, Some("    println!(\"new\");".to_string()));
    }

    #[test]
    fn test_diff_card_data_computation() {
        let old = "fn main() {\n    let a = 1;\n}";
        let new = "fn main() {\n    let a = 2;\n    let b = 3;\n}";

        let card = DiffViewEngine::compute_diff_card_data("edit", "src/main.rs", old, new);
        assert_eq!(card.file_path, "src/main.rs");
        assert_eq!(card.tool_name, "edit");
        assert_eq!(card.additions, 2);
        assert_eq!(card.deletions, 1);
        assert_eq!(card.lines.len(), 5);
    }

    #[test]
    fn test_diff_card_rendering_threshold_8_lines() {
        let old = (0..20)
            .map(|i| format!("line {i}"))
            .collect::<Vec<_>>()
            .join("\n");
        let new = (0..20)
            .map(|i| format!("line {i} modified"))
            .collect::<Vec<_>>()
            .join("\n");
        let colors = ThemeColors::default();

        let card = DiffViewEngine::compute_diff_card_data("edit", "src/test.rs", &old, &new);
        let rendered_collapsed = DiffViewEngine::render_diff_card(&card, 80, false, &colors);

        // Collapsed view should include the 8-line threshold badge
        assert!(
            rendered_collapsed
                .iter()
                .any(|l| l.spans.iter().any(|s| s.content.contains("lines hidden")))
        );

        let rendered_expanded = DiffViewEngine::render_diff_card(&card, 80, true, &colors);
        // Expanded view should not contain hidden badge
        assert!(
            !rendered_expanded
                .iter()
                .any(|l| l.spans.iter().any(|s| s.content.contains("lines hidden")))
        );
    }

    #[test]
    fn test_narrow_terminal_fallback_to_stacked() {
        let old = "line 1\nline 2";
        let new = "line 1\nline 2 modified";
        let colors = ThemeColors::default();

        let lines = DiffViewEngine::render_diff(old, new, DiffLayout::SideBySide, 60, &colors);
        assert!(!lines.is_empty());
        // Narrow terminal should not contain column separator header
        assert!(!lines[0].spans.iter().any(|s| s.content == " │ "));
    }
}

// endregion: --- Tests
