use crate::app::layout::helpers::display_tool_name;
use crate::app::*;
use crate::colors::ThemeColorsExt;

/// Visual policy for a timeline tool call.
///
/// The timeline supplies the stable raw name and preview; this module owns
/// human-facing naming, provider-prefix normalization, icon selection, and
/// pill styling.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ToolPresentation {
    pub(crate) icon_name: String,
    pub(crate) label: String,
}

pub(crate) fn resolve_tool_presentation(raw_name: &str) -> ToolPresentation {
    let icon_name = display_tool_name(raw_name);

    let label = match icon_name.as_str() {
        "search_for_pattern" | "grep_search" | "grep" => "Search codebase".to_owned(),
        "semantic_search" => "Semantic search".to_owned(),
        "find_file" | "glob" => "Find files".to_owned(),
        "read_file" | "read_multiple_files" => "Read file".to_owned(),
        "list_directory" => "List directory".to_owned(),
        "create_text_file" | "write_file" | "create_file" => "Create file".to_owned(),
        "replace_content" | "edit_block" | "edit_file" => "Edit file".to_owned(),
        "execute_shell_command" | "run_command" | "shell" | "bash" => "Run command".to_owned(),
        "run_subagent" => "Run subagent".to_owned(),
        "run_parallel_subagents" => "Run parallel agents".to_owned(),
        "index_workspace" => "Index workspace".to_owned(),
        "UpdatePlan" => "Update plan".to_owned(),
        _ => humanize(&icon_name),
    };

    ToolPresentation { icon_name, label }
}

fn color_to_rgb(color: ratatui::style::Color) -> Option<(u8, u8, u8)> {
    match color {
        ratatui::style::Color::Rgb(r, g, b) => Some((r, g, b)),
        ratatui::style::Color::Black => Some((0, 0, 0)),
        ratatui::style::Color::White => Some((255, 255, 255)),
        ratatui::style::Color::Gray => Some((128, 128, 128)),
        ratatui::style::Color::DarkGray => Some((64, 64, 64)),
        _ => None,
    }
}

fn pick_high_contrast_fg(
    bg_color: ratatui::style::Color,
    option1: ratatui::style::Color,
    option2: ratatui::style::Color,
) -> ratatui::style::Color {
    if let (Some(bg), Some(c1), Some(c2)) = (
        color_to_rgb(bg_color),
        color_to_rgb(option1),
        color_to_rgb(option2),
    ) {
        let r1 = cade_core::resources::calculate_contrast_ratio(c1, bg);
        let r2 = cade_core::resources::calculate_contrast_ratio(c2, bg);
        if r1 >= r2 { option1 } else { option2 }
    } else {
        option1
    }
}

pub(crate) fn render_tool_activity_pill(
    presentation: &ToolPresentation,
    colors: &ThemeColors,
    nerd: bool,
) -> Vec<Span<'static>> {
    let icon = crate::icons::tool_icon(&presentation.icon_name, nerd);
    let pill_bg = colors.c_primary();
    let pill_fg = pick_high_contrast_fg(pill_bg, colors.c_bg_base(), colors.c_text_primary());
    let base_bg = colors.c_bg_base();
    let shadow_bg = colors.c_bg_surface0();

    let pill_style = Style::default()
        .fg(pill_fg)
        .bg(pill_bg)
        .add_modifier(Modifier::BOLD);

    if nerd {
        vec![
            // Left rounded edge
            Span::styled("\u{e0b6}", Style::default().fg(pill_bg).bg(base_bg)),
            // Pill content
            Span::styled(format!("{icon} {}", presentation.label), pill_style),
            // Right rounded edge with shadow transition
            Span::styled("\u{e0b4}", Style::default().fg(pill_bg).bg(shadow_bg)),
            // Drop-shadow background segment
            Span::styled("\u{2590}", Style::default().fg(shadow_bg).bg(base_bg)),
        ]
    } else {
        vec![
            // ASCII rounded bookends
            Span::styled("(", pill_style),
            Span::styled(format!("{icon} {}", presentation.label), pill_style),
            Span::styled(")", pill_style),
            Span::styled("\u{2590}", Style::default().fg(shadow_bg).bg(base_bg)),
        ]
    }
}

fn humanize(name: &str) -> String {
    let mut words = name.split('_').filter(|word| !word.is_empty());
    let Some(first) = words.next() else {
        return "Tool".to_owned();
    };

    let mut label = first.to_owned();
    if let Some(first_char) = label.get_mut(0..1) {
        first_char.make_ascii_uppercase();
    }
    for word in words {
        label.push(' ');
        label.push_str(word);
    }
    label
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resolves_known_native_tool() {
        assert_eq!(
            resolve_tool_presentation("search_for_pattern").label,
            "Search codebase"
        );
    }

    #[test]
    fn strips_mcp_prefix_before_resolving_label() {
        let presentation = resolve_tool_presentation("serena__search_for_pattern");
        assert_eq!(presentation.icon_name, "search_for_pattern");
        assert_eq!(presentation.label, "Search codebase");
    }

    #[test]
    fn humanizes_unknown_prefixed_tool() {
        assert_eq!(
            resolve_tool_presentation("custom_mcp__archive_project").label,
            "Archive project"
        );
    }

    #[test]
    fn renders_ascii_safe_pill() {
        let colors = ThemeColors::default();
        let spans = render_tool_activity_pill(
            &resolve_tool_presentation("search_for_pattern"),
            &colors,
            false,
        );
        let text = spans
            .iter()
            .map(|span| span.content.as_ref())
            .collect::<String>();
        assert!(text.contains("(▶ Search codebase)"), "got {text:?}");
        assert!(text.contains("\u{2590}"), "shadow should be present");
    }

    #[test]
    fn renders_nerd_rounded_pill_with_shadow() {
        let colors = ThemeColors::default();
        let spans = render_tool_activity_pill(
            &resolve_tool_presentation("execute_shell_command"),
            &colors,
            true,
        );
        let text = spans
            .iter()
            .map(|span| span.content.as_ref())
            .collect::<String>();
        assert!(text.starts_with("\u{e0b6}"), "left rounded cap expected");
        assert!(text.contains("Run command"), "label expected");
        assert!(text.contains("\u{e0b4}"), "right rounded cap expected");
        assert!(text.ends_with("\u{2590}"), "shadow glyph expected");
    }
}
