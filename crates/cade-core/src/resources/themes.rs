use std::path::Path;
pub type Theme = opaline::Theme;
pub use opaline::{list_available_themes, ThemeInfo};

/// Discover all custom themes from standard locations.
/// Opaline can load TOML themes.
pub fn discover_themes(cwd: &Path, agent_dir: &Path) -> Vec<opaline::Theme> {
    let mut themes: Vec<opaline::Theme> = Vec::new();
    let mut seen: std::collections::HashSet<String> = std::collections::HashSet::new();

    // Project-local
    let project_dir = cwd.join(".cade").join("themes");
    load_themes_from_dir(&project_dir, &mut themes, &mut seen);

    // Global
    let global_dir = agent_dir.join("themes");
    load_themes_from_dir(&global_dir, &mut themes, &mut seen);

    themes
}

/// Discover all themes (built-ins merged with on-disk) in display order.
pub fn discover_themes_with_builtins(cwd: &Path, agent_dir: &Path) -> Vec<opaline::Theme> {
    let mut themes = discover_themes(cwd, agent_dir);

    for builtin in opaline::list_available_themes() {
        if !themes.iter().any(|t| t.meta.name == builtin.name) {
            if let Some(theme) = opaline::load_by_name(&builtin.name) {
                themes.push(theme);
            }
        }
    }

    themes
}

fn load_themes_from_dir(
    dir: &Path,
    themes: &mut Vec<opaline::Theme>,
    seen: &mut std::collections::HashSet<String>,
) {
    if let Ok(entries) = std::fs::read_dir(dir) {
        let mut paths: Vec<_> = entries
            .filter_map(Result::ok)
            .map(|e| e.path())
            .filter(|p| p.extension().and_then(|e| e.to_str()) == Some("toml"))
            .collect();
        paths.sort(); // Predictable loading order

        for path in paths {
            if let Ok(theme) = load_theme(&path) {
                if !seen.contains(&theme.meta.name) {
                    seen.insert(theme.meta.name.clone());
                    themes.push(theme);
                }
            } else {
                tracing::warn!("Failed to load theme from {:?}", path);
            }
        }
    }
}

pub fn load_theme(path: &Path) -> crate::Result<opaline::Theme> {
    let content = std::fs::read_to_string(path)
        .map_err(|e| crate::Error::custom(format!("read theme {}: {e}", path.display())))?;
    let theme = opaline::load_from_str(&content, Some(path))
        .map_err(|e| crate::Error::custom(format!("parse theme {}: {}", path.display(), e)))?;
    Ok(theme)
}

pub fn get_theme(name: &str) -> Option<opaline::Theme> {
    opaline::load_by_name(name)
}
