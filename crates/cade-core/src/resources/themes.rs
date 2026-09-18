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

    /// Validate a theme by name or file path, returning a detailed validation report.
    pub fn validate(&self, name_or_path: &str) -> ThemeValidationReport {
        let path = Path::new(name_or_path);
        if path.is_file() {
            if let Ok(content) = std::fs::read_to_string(path) {
                return validate_theme_str(&content);
            } else {
                return ThemeValidationReport {
                    name: name_or_path.to_string(),
                    is_valid: false,
                    errors: vec![format!("Failed to read file: {name_or_path}")],
                    warnings: Vec::new(),
                    defined_tokens: 0,
                    missing_recommended_tokens: Vec::new(),
                    contrast_warnings: Vec::new(),
                };
            }
        }

        if let Some(theme) = self.resolve(name_or_path) {
            validate_theme(&theme)
        } else {
            ThemeValidationReport {
                name: name_or_path.to_string(),
                is_valid: false,
                errors: vec![format!("Theme '{name_or_path}' not found in project, global, or built-in registry")],
                warnings: Vec::new(),
                defined_tokens: 0,
                missing_recommended_tokens: Vec::new(),
                contrast_warnings: Vec::new(),
            }
        }
    }
}

/// Canonical recommended token roles for complete CADE theming.
pub const CANONICAL_RECOMMENDED_ROLES: &[(&str, &str)] = &[
    ("bg.base", "Base terminal background"),
    ("bg.panel", "Panel / card background"),
    ("bg.elevated", "Elevated / success container background"),
    ("bg.highlight", "Highlight / selection background"),
    ("bg.selection", "Text selection background"),
    ("text.primary", "Primary foreground body text"),
    ("text.muted", "Muted secondary text"),
    ("text.dim", "Dim metadata text"),
    ("accent.primary", "Primary brand accent"),
    ("accent.secondary", "Secondary accent"),
    ("accent.tertiary", "Tertiary accent"),
    ("accent.deep", "Deep accent"),
    ("border.unfocused", "Subtle unfocused border"),
    ("border.focused", "Active / focused border"),
    ("success", "Success status indicator"),
    ("warning", "Warning status indicator"),
    ("error", "Error status indicator"),
    ("code.keyword", "Syntax keyword"),
    ("code.string", "Syntax string literal"),
    ("code.comment", "Syntax comment"),
    ("code.function", "Syntax function/method"),
    ("code.number", "Syntax numeric literal"),
    ("code.type", "Syntax type / struct"),
];

/// Detailed report on theme validity, coverage, and accessibility.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct ThemeValidationReport {
    pub name: String,
    pub is_valid: bool,
    pub errors: Vec<String>,
    pub warnings: Vec<String>,
    pub defined_tokens: usize,
    pub missing_recommended_tokens: Vec<String>,
    pub contrast_warnings: Vec<String>,
}

/// Validate an Opaline theme string directly.
pub fn validate_theme_str(content: &str) -> ThemeValidationReport {
    match opaline::load_from_str(content, None) {
        Ok(theme) => validate_theme(&theme),
        Err(e) => ThemeValidationReport {
            name: "unparsed".to_string(),
            is_valid: false,
            errors: vec![format!("TOML parse error: {e}")],
            warnings: Vec::new(),
            defined_tokens: 0,
            missing_recommended_tokens: Vec::new(),
            contrast_warnings: Vec::new(),
        },
    }
}

/// Validate a loaded Opaline theme.
pub fn validate_theme(theme: &opaline::Theme) -> ThemeValidationReport {
    let mut warnings = Vec::new();
    let mut missing_recommended_tokens = Vec::new();
    let mut contrast_warnings = Vec::new();

    let mut defined = 0;
    for &(role, desc) in CANONICAL_RECOMMENDED_ROLES {
        if theme.has_token(role) {
            defined += 1;
        } else {
            missing_recommended_tokens.push(role.to_string());
            warnings.push(format!("Missing recommended token '{role}' ({desc})"));
        }
    }

    // Contrast analysis: text.primary vs bg.base
    if let (Some(fg), Some(bg)) = (theme.try_color("text.primary"), theme.try_color("bg.base")) {
        let ratio = calculate_contrast_ratio((fg.r, fg.g, fg.b), (bg.r, bg.g, bg.b));
        if ratio < 4.5 {
            let msg = format!(
                "Low contrast between 'text.primary' and 'bg.base': {ratio:.2}:1 (WCAG AA requires >= 4.5:1)"
            );
            contrast_warnings.push(msg.clone());
            warnings.push(msg);
        }
    }

    ThemeValidationReport {
        name: theme.meta.name.clone(),
        is_valid: true,
        errors: Vec::new(),
        warnings,
        defined_tokens: defined,
        missing_recommended_tokens,
        contrast_warnings,
    }
}

/// Compute relative luminance for an sRGB color per WCAG 2.1 specifications.
fn relative_luminance(r: u8, g: u8, b: u8) -> f64 {
    fn channel_l(c: u8) -> f64 {
        let v = c as f64 / 255.0;
        if v <= 0.03928 {
            v / 12.92
        } else {
            ((v + 0.055) / 1.055).powf(2.4)
        }
    }
    0.2126 * channel_l(r) + 0.7152 * channel_l(g) + 0.0722 * channel_l(b)
}

/// Compute contrast ratio between two sRGB colors per WCAG 2.1 specifications.
pub fn calculate_contrast_ratio(c1: (u8, u8, u8), c2: (u8, u8, u8)) -> f64 {
    let l1 = relative_luminance(c1.0, c1.1, c1.2);
    let l2 = relative_luminance(c2.0, c2.1, c2.2);
    let (lighter, darker) = if l1 > l2 { (l1, l2) } else { (l2, l1) };
    (lighter + 0.05) / (darker + 0.05)
}

/// Canonical starter/reference theme exhibiting full semantic token coverage.
pub const REFERENCE_THEME_TOML: &str = r##"# CADE Canonical Theme Definition
# Format: Opaline TOML Specification

[meta]
name = "reference"
variant = "dark"
description = "Canonical reference theme exhibiting full semantic token coverage for CADE"
author = "CADE Architecture Team"

[palette]
bg_dark         = "#1e1e2e"
bg_panel        = "#252538"
bg_elevated     = "#2f2f45"
bg_highlight    = "#3b3b55"
bg_selection    = "#45475a"

fg_primary      = "#cdd6f4"
fg_muted        = "#a6adc8"
fg_dim          = "#6c7086"

accent_primary  = "#89b4fa"
accent_secondary= "#f5c2e7"
accent_tertiary = "#94e2d5"
accent_deep     = "#b4befe"

status_success  = "#a6e3a1"
status_warning  = "#f9e2af"
status_error    = "#f38ba8"

border_base     = "#313244"
border_active   = "#89b4fa"

syn_keyword     = "#cba6f7"
syn_string      = "#a6e3a1"
syn_comment     = "#6c7086"
syn_function    = "#89b4fa"
syn_number      = "#fab387"
syn_type        = "#f9e2af"

[tokens]
"bg.base"          = "bg_dark"
"bg.panel"         = "bg_panel"
"bg.elevated"      = "bg_elevated"
"bg.highlight"     = "bg_highlight"
"bg.selection"     = "bg_selection"

"text.primary"     = "fg_primary"
"text.muted"       = "fg_muted"
"text.dim"         = "fg_dim"

"accent.primary"   = "accent_primary"
"accent.secondary" = "accent_secondary"
"accent.tertiary"  = "accent_tertiary"
"accent.deep"      = "accent_deep"

"success"          = "status_success"
"warning"          = "status_warning"
"error"            = "status_error"

"border.unfocused" = "border_base"
"border.focused"   = "border_active"

"code.keyword"     = "syn_keyword"
"code.string"      = "syn_string"
"code.comment"     = "syn_comment"
"code.function"    = "syn_function"
"code.number"      = "syn_number"
"code.type"        = "syn_type"

# Documented Compatibility Aliases
"cade.success"             = "status_success"
"cade.warning"             = "status_warning"
"cade.error"               = "status_error"
"cade.border"              = "border_base"
"cade.border_accent"       = "border_active"
"cade.selected_bg"         = "bg_highlight"
"cade.user_message_bg"     = "bg_panel"
"cade.tool_success_bg"     = "bg_elevated"
"cade.syntax_comment"      = "syn_comment"
"cade.syntax_keyword"      = "syn_keyword"
"cade.syntax_function"     = "syn_function"
"cade.syntax_string"       = "syn_string"
"cade.syntax_number"       = "syn_number"
"cade.syntax_type"         = "syn_type"
"##;

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

    #[test]
    fn test_reference_theme_is_valid_and_complete() {
        let report = validate_theme_str(REFERENCE_THEME_TOML);
        assert!(report.is_valid, "reference theme should be valid");
        assert!(report.errors.is_empty(), "reference theme should have 0 errors");
        assert_eq!(
            report.defined_tokens,
            CANONICAL_RECOMMENDED_ROLES.len(),
            "reference theme should define all canonical recommended tokens"
        );
        assert!(
            report.missing_recommended_tokens.is_empty(),
            "no recommended tokens should be missing in reference theme"
        );
        assert!(
            report.contrast_warnings.is_empty(),
            "reference theme should meet WCAG contrast guidelines"
        );
    }

    #[test]
    fn test_validation_reports_missing_tokens_and_contrast() {
        let toml_missing_and_low_contrast = r##"
        [meta]
        name = "low-contrast"
        variant = "dark"
        [palette]
        dark = "#222222"
        dim = "#282828"
        [tokens]
        "bg.base" = "dark"
        "text.primary" = "dim"
        "##;
        let report = validate_theme_str(toml_missing_and_low_contrast);
        assert!(report.is_valid);
        assert!(!report.warnings.is_empty());
        assert!(!report.contrast_warnings.is_empty());
        assert!(report.contrast_warnings[0].contains("Low contrast"));
        assert!(report.missing_recommended_tokens.contains(&"accent.primary".to_string()));
    }
}
