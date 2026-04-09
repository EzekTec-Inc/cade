use crate::app::*;
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
        PermissionMode::Default => colors.border_muted,
        PermissionMode::AcceptEdits => colors.thinking_minimal,
        PermissionMode::Plan => colors.success,
        PermissionMode::BypassPermissions => colors.error,
    }
}

pub(crate) fn mode_footer_left<'a>(mode: PermissionMode, colors: &ThemeColors) -> (&'a str, &'a str, RC) {
    match mode {
        PermissionMode::Default => ("Press / for commands", "", colors.border_muted),
        PermissionMode::AcceptEdits => ("accept edits", "⏵⏵", colors.thinking_minimal),
        PermissionMode::Plan => ("plan mode", "⏸", colors.success),
        PermissionMode::BypassPermissions => ("bypass (allow all)", "⚡", colors.error),
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


