
use crate::app::*;
use crate::colors::ThemeColorsExt;

/// Render a dim backdrop over the full frame area. Call before rendering an
/// overlay to provide visual separation from the content underneath.
pub(crate) fn render_backdrop(frame: &mut ratatui::Frame, area: ratatui::layout::Rect, colors: &ThemeColors) {
    use ratatui::widgets::Paragraph;
    use ratatui::style::Style;
    let dim_style = Style::default().bg(colors.c_bg_base());
    let blank = " ".repeat(area.width as usize);
    let lines: Vec<ratatui::text::Line<'static>> = (0..area.height)
        .map(|_| ratatui::text::Line::from(ratatui::text::Span::styled(blank.clone(), dim_style)))
        .collect();
    frame.render_widget(Paragraph::new(lines), area);
}

/// Abbreviate a filesystem path for the footer: last 2 components, with ~/
/// prefix when the path is under the user's home directory.
pub(crate) fn abbreviate_cwd(path: &std::path::Path) -> String {
    let home = dirs::home_dir();
    let (prefix, rel_path) = if let Some(h) = &home {
        if let Ok(rel) = path.strip_prefix(h) {
            ("~/".to_string(), rel.to_path_buf())
        } else {
            (String::new(), path.to_path_buf())
        }
    } else {
        (String::new(), path.to_path_buf())
    };

    let parts: Vec<std::ffi::OsString> = rel_path
        .components()
        .map(|c| c.as_os_str().to_owned())
        .collect();

    if parts.is_empty() {
        return if prefix.is_empty() {
            "/".to_string()
        } else {
            "~".to_string()
        };
    }

    let display: String = if parts.len() <= 2 {
        parts
            .iter()
            .map(|p| p.to_string_lossy())
            .collect::<Vec<_>>()
            .join("/")
    } else {
        let last2: String = parts[parts.len() - 2..]
            .iter()
            .map(|p| p.to_string_lossy())
            .collect::<Vec<_>>()
            .join("/");
        format!("…/{last2}")
    };

    format!("{prefix}{display}")
}

pub(crate) fn mode_sep_color(mode: PermissionMode, colors: &ThemeColors) -> RC {
    match mode {
        PermissionMode::Default => colors.c_border_base(),
        PermissionMode::AcceptEdits => colors.c_thinking_minimal(),
        PermissionMode::Plan => colors.c_success(),
        PermissionMode::BypassPermissions => colors.c_error(),
    }
}

pub(crate) fn mode_footer_left<'a>(mode: PermissionMode, colors: &ThemeColors) -> (&'a str, &'a str, RC) {
    match mode {
        PermissionMode::Default => ("Press / for commands", "", colors.c_border_base()),
        PermissionMode::AcceptEdits => ("accept edits", "⏵⏵", colors.c_thinking_minimal()),
        PermissionMode::Plan => ("plan mode", "⏸", colors.c_success()),
        PermissionMode::BypassPermissions => ("bypass (allow all)", "⚡", colors.c_error()),
    }
}

pub fn cycle_mode(mode: PermissionMode) -> PermissionMode {
    match mode {
        PermissionMode::Plan => PermissionMode::Default,
        _ => PermissionMode::Plan,
    }
}

pub fn cycle_mode_back(mode: PermissionMode) -> PermissionMode {
    match mode {
        PermissionMode::Plan => PermissionMode::Default,
        _ => PermissionMode::Plan,
    }
}

pub(crate) fn display_tool_name(name: &str) -> String {
    // Strip MCP server prefix: "developer__shell" → "shell"
    let stripped = if let Some(pos) = name.rfind("__") {
        &name[pos + 2..]
    } else {
        name
    };
    stripped.to_string()
}

pub fn truncate_str(s: &str, max: usize) -> String {
    let chars: Vec<char> = s.chars().collect();
    if chars.len() <= max {
        s.to_string()
    } else {
        format!(
            "{}…",
            chars[..max.saturating_sub(1)].iter().collect::<String>()
        )
    }
}

/// Format a token count compactly: 1234 → "1.2k", 12345 → "12k", 1234567 → "1.2M".
pub fn format_token_count(n: u64) -> String {
    if n < 1_000 {
        n.to_string()
    } else if n < 10_000 {
        format!("{:.1}k", n as f64 / 1_000.0)
    } else if n < 1_000_000 {
        format!("{}k", n / 1_000)
    } else {
        format!("{:.1}M", n as f64 / 1_000_000.0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_format_token_count() {
        assert_eq!(format_token_count(0), "0");
        assert_eq!(format_token_count(500), "500");
        assert_eq!(format_token_count(999), "999");
        assert_eq!(format_token_count(1_000), "1.0k");
        assert_eq!(format_token_count(1_234), "1.2k");
        assert_eq!(format_token_count(9_999), "10.0k");
        assert_eq!(format_token_count(10_000), "10k");
        assert_eq!(format_token_count(50_000), "50k");
        assert_eq!(format_token_count(999_999), "999k");
        assert_eq!(format_token_count(1_000_000), "1.0M");
        assert_eq!(format_token_count(1_500_000), "1.5M");
    }
}


