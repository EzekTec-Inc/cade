use std::path::Path;
pub type Theme = opaline::Theme;
pub use opaline::{ThemeInfo, list_available_themes};

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
        if !themes.iter().any(|t| t.meta.name == builtin.name)
            && let Some(theme) = opaline::load_by_name(&builtin.name)
        {
            themes.push(theme);
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

/// A resolved theme token with explicit fallback provenance.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ThemeToken {
    /// Token was explicitly defined and matched in the theme.
    Exact {
        token: String,
        r: u8,
        g: u8,
        b: u8,
    },
    /// Primary token was missing; resolved through a documented fallback token.
    Fallback {
        requested: String,
        resolved_token: String,
        r: u8,
        g: u8,
        b: u8,
    },
    /// Neither primary nor any fallback token was defined in the theme.
    Missing {
        requested: String,
    },
}

impl ThemeToken {
    /// Returns the resolved RGB components, if found or resolved via fallback.
    pub fn rgb(&self) -> Option<(u8, u8, u8)> {
        match self {
            Self::Exact { r, g, b, .. } | Self::Fallback { r, g, b, .. } => Some((*r, *g, *b)),
            Self::Missing { .. } => None,
        }
    }

    /// Returns true if this token was an exact match.
    pub fn is_exact(&self) -> bool {
        matches!(self, Self::Exact { .. })
    }

    /// Returns true if this token was resolved via fallback.
    pub fn is_fallback(&self) -> bool {
        matches!(self, Self::Fallback { .. })
    }

    /// Returns true if this token was missing.
    pub fn is_missing(&self) -> bool {
        matches!(self, Self::Missing { .. })
    }
}

/// Explicitly resolve a token from an Opaline Theme with fallbacks.
/// Unlike older heuristics, this does NOT treat intentional neutral gray RGB (128, 128, 128)
/// as missing: `theme.try_color` is used to check genuine token existence.
pub fn resolve_token(theme: &opaline::Theme, primary: &str, fallbacks: &[&str]) -> ThemeToken {
    if let Some(c) = theme.try_color(primary) {
        return ThemeToken::Exact {
            token: primary.to_string(),
            r: c.r,
            g: c.g,
            b: c.b,
        };
    }

    for fb in fallbacks {
        if let Some(c) = theme.try_color(fb) {
            return ThemeToken::Fallback {
                requested: primary.to_string(),
                resolved_token: (*fb).to_string(),
                r: c.r,
                g: c.g,
                b: c.b,
            };
        }
    }

    ThemeToken::Missing {
        requested: primary.to_string(),
    }
}

/// Canonical ThemeResolver coordinating theme search precedence and token resolution.
/// Precedence order:
/// 1. Project-local directory (`.cade/themes`)
/// 2. Global user directory (`~/.cade/themes`)
/// 3. Built-in themes from Opaline
#[derive(Debug, Clone)]
pub struct ThemeResolver {
    project_dir: std::path::PathBuf,
    global_dir: std::path::PathBuf,
}

impl ThemeResolver {
    /// Create a resolver with the given workspace current directory and agent directory.
    pub fn new(cwd: &Path, agent_dir: &Path) -> Self {
        Self {
            project_dir: cwd.join(".cade").join("themes"),
            global_dir: agent_dir.join("themes"),
        }
    }

    /// Create a resolver pointing to specific directories.
    pub fn with_dirs(project_dir: std::path::PathBuf, global_dir: std::path::PathBuf) -> Self {
        Self {
            project_dir,
            global_dir,
        }
    }

    /// Resolve a theme by name or filename according to precedence rules.
    pub fn resolve(&self, name: &str) -> Option<opaline::Theme> {
        if let Some(t) = find_theme_in_dir(&self.project_dir, name) {
            return Some(t);
        }
        if let Some(t) = find_theme_in_dir(&self.global_dir, name) {
            return Some(t);
        }
        get_theme(name)
    }

    /// List all discoverable themes in precedence order, deduplicated by theme name.
    pub fn list_all(&self) -> Vec<opaline::Theme> {
        let mut themes = Vec::new();
        let mut seen = std::collections::HashSet::new();

        load_themes_from_dir(&self.project_dir, &mut themes, &mut seen);
        load_themes_from_dir(&self.global_dir, &mut themes, &mut seen);

        for builtin in list_available_themes() {
            if !seen.contains(&builtin.name)
                && let Some(theme) = opaline::load_by_name(&builtin.name)
            {
                seen.insert(builtin.name);
                themes.push(theme);
            }
        }

        themes
    }

    /// Inspect a slice of token specifications `(primary, fallbacks)` against a theme.
    pub fn inspect_tokens(
        &self,
        theme: &opaline::Theme,
        specs: &[(&str, &[&str])],
    ) -> Vec<ThemeToken> {
        specs
            .iter()
            .map(|(primary, fallbacks)| resolve_token(theme, primary, fallbacks))
            .collect()
    }
}

fn find_theme_in_dir(dir: &Path, name: &str) -> Option<opaline::Theme> {
    if !dir.exists() {
        return None;
    }
    // Direct path check: <name>.toml
    let direct = dir.join(format!("{name}.toml"));
    if direct.is_file() {
        if let Ok(theme) = load_theme(&direct) {
            return Some(theme);
        }
    }

    // Scan directory for matching meta.name
    if let Ok(entries) = std::fs::read_dir(dir) {
        let mut paths: Vec<_> = entries
            .filter_map(Result::ok)
            .map(|e| e.path())
            .filter(|p| p.extension().and_then(|e| e.to_str()) == Some("toml"))
            .collect();
        paths.sort();

        for path in paths {
            if let Ok(theme) = load_theme(&path) {
                if theme.meta.name == name {
                    return Some(theme);
                }
            }
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_intentional_neutral_gray_is_preserved() {
        let toml = r##"
        [meta]
        name = "neutral-gray-test"
        variant = "dark"
        [palette]
        gray = "#808080"
        [tokens]
        "bg.base" = "gray"
        "cade.border" = "#ffffff"
        "##;
        let theme = opaline::load_from_str(toml, None).expect("valid toml theme");

        // Primary is explicitly gray (128, 128, 128)
        let token = resolve_token(&theme, "bg.base", &["cade.border"]);
        assert_eq!(
            token,
            ThemeToken::Exact {
                token: "bg.base".to_string(),
                r: 128,
                g: 128,
                b: 128,
            }
        );
        assert_eq!(token.rgb(), Some((128, 128, 128)));
    }

    #[test]
    fn test_fallback_provenance_when_primary_missing() {
        let toml = r##"
        [meta]
        name = "fallback-test"
        variant = "dark"
        [palette]
        accent = "#3399ff"
        [tokens]
        "cade.fallback_token" = "accent"
        "##;
        let theme = opaline::load_from_str(toml, None).expect("valid toml theme");

        let token = resolve_token(&theme, "primary.missing", &["cade.fallback_token"]);
        assert_eq!(
            token,
            ThemeToken::Fallback {
                requested: "primary.missing".to_string(),
                resolved_token: "cade.fallback_token".to_string(),
                r: 51,
                g: 153,
                b: 255,
            }
        );
        assert!(token.is_fallback());
    }

    #[test]
    fn test_missing_token_when_neither_primary_nor_fallbacks_exist() {
        let toml = r##"
        [meta]
        name = "missing-test"
        variant = "dark"
        [palette]
        accent = "#3399ff"
        [tokens]
        "some.other" = "accent"
        "##;
        let theme = opaline::load_from_str(toml, None).expect("valid toml theme");

        let token = resolve_token(&theme, "unconfigured.role", &["fallback.missing"]);
        assert_eq!(
            token,
            ThemeToken::Missing {
                requested: "unconfigured.role".to_string(),
            }
        );
        assert!(token.is_missing());
        assert_eq!(token.rgb(), None);
    }

    #[test]
    fn test_resolver_precedence() {
        let temp_dir = tempfile::tempdir().expect("create temp dir");
        let project_dir = temp_dir.path().join(".cade").join("themes");
        let global_dir = temp_dir.path().join("agent").join("themes");
        std::fs::create_dir_all(&project_dir).unwrap();
        std::fs::create_dir_all(&global_dir).unwrap();

        // Write global theme
        let global_theme_toml = r##"
        [meta]
        name = "shared-theme"
        variant = "dark"
        [palette]
        c = "#111111"
        [tokens]
        "bg.base" = "c"
        "##;
        std::fs::write(global_dir.join("shared-theme.toml"), global_theme_toml).unwrap();

        // Write project theme with same name but different color
        let project_theme_toml = r##"
        [meta]
        name = "shared-theme"
        variant = "dark"
        [palette]
        c = "#222222"
        [tokens]
        "bg.base" = "c"
        "##;
        std::fs::write(project_dir.join("shared-theme.toml"), project_theme_toml).unwrap();

        let resolver = ThemeResolver::with_dirs(project_dir, global_dir);
        let resolved = resolver.resolve("shared-theme").expect("resolved theme");
        assert_eq!(resolved.meta.name, "shared-theme");
        let c = resolved.try_color("bg.base").unwrap();
        // Should resolve from project dir (#222222 -> r: 34, g: 34, b: 34)
        assert_eq!((c.r, c.g, c.b), (34, 34, 34));
    }
}
