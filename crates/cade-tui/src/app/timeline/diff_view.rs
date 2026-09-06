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
                        Span::styled("- ", Style::default().fg(colors.c_error()).add_modifier(Modifier::BOLD)),
                        Span::styled(val.to_string(), Style::default().fg(colors.c_error())),
                    ]));
                }
                ChangeTag::Insert => {
                    lines.push(Line::from(vec![
                        Span::styled("+ ", Style::default().fg(colors.c_success()).add_modifier(Modifier::BOLD)),
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
            Span::styled(left_header, Style::default().fg(colors.c_text_muted()).add_modifier(Modifier::BOLD)),
            Span::styled(" │ ", colors.border_muted()),
            Span::styled(right_header, Style::default().fg(colors.c_text_muted()).add_modifier(Modifier::BOLD)),
        ]));

        lines.push(Line::from(Span::styled("─".repeat(width as usize), colors.border_muted())));

        for pair in pairs {
            let (left_str, left_style) = match pair.left {
                Some(s) => {
                    let truncated = if s.len() > col_w {
                        format!("{}…", &s[..col_w.saturating_sub(1)])
                    } else {
                        s
                    };
                    let pad = format!("{:width$}", truncated, width = col_w);
                    if pair.change == ChangeTag::Delete || (pair.change == ChangeTag::Insert && pair.right.is_some()) {
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
