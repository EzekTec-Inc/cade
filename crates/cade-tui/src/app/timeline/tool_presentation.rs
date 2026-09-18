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

pub(crate) fn render_tool_activity_pill(
    presentation: &ToolPresentation,
    colors: &ThemeColors,
    nerd: bool,
) -> Vec<Span<'static>> {
    let icon = crate::icons::tool_icon(&presentation.icon_name, nerd);
    let pill_style = Style::default()
        .fg(colors.c_bg_base())
        .bg(colors.c_primary())
        .add_modifier(Modifier::BOLD);

    vec![
        Span::styled(" ", pill_style),
        Span::styled(format!("{icon} {}", presentation.label), pill_style),
        Span::styled(" ", pill_style),
    ]
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
        assert!(text.contains("▶ Search codebase"), "got {text:?}");
    }
}
