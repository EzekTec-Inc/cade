/// Theme discovery and loading.
///
/// Themes are JSON files in `~/.cade/themes/*.json` or `.cade/themes/*.json`.
/// The `name` key inside the JSON is the theme identifier used in settings.
use std::collections::HashMap;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

// region:    --- Types

/// A single color value in a theme.
///
/// Supported formats (matching pi's theme schema):
/// - `"#rrggbb"` — 24-bit hex
/// - An integer 0–255 — xterm 256-color index
/// - A string referencing a variable defined in `vars`
/// - `""` — terminal default color
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(untagged)]
pub enum ThemeColor {
    Hex(String), // "#rrggbb" or variable name or ""
    Index(u8),   // 0-255 palette index
}

impl Default for ThemeColor {
    fn default() -> Self {
        Self::Hex(String::new())
    }
}

/// Full set of color tokens for a theme (51 tokens matching pi's schema).
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct ThemeTokens {
    // -- Core UI
    pub accent: ThemeColor,
    pub border: ThemeColor,
    #[serde(rename = "borderAccent")]
    pub border_accent: ThemeColor,
    #[serde(rename = "borderMuted")]
    pub border_muted: ThemeColor,
    pub success: ThemeColor,
    pub error: ThemeColor,
    pub warning: ThemeColor,
    pub muted: ThemeColor,
    pub dim: ThemeColor,
    pub text: ThemeColor,
    #[serde(rename = "thinkingText")]
    pub thinking_text: ThemeColor,

    // -- Backgrounds & content
    #[serde(rename = "selectedBg")]
    pub selected_bg: ThemeColor,
    #[serde(rename = "userMessageBg")]
    pub user_message_bg: ThemeColor,
    #[serde(rename = "userMessageText")]
    pub user_message_text: ThemeColor,
    #[serde(rename = "customMessageBg")]
    pub custom_message_bg: ThemeColor,
    #[serde(rename = "customMessageText")]
    pub custom_message_text: ThemeColor,
    #[serde(rename = "customMessageLabel")]
    pub custom_message_label: ThemeColor,
    #[serde(rename = "toolPendingBg")]
    pub tool_pending_bg: ThemeColor,
    #[serde(rename = "toolSuccessBg")]
    pub tool_success_bg: ThemeColor,
    #[serde(rename = "toolErrorBg")]
    pub tool_error_bg: ThemeColor,
    #[serde(rename = "toolTitle")]
    pub tool_title: ThemeColor,
    #[serde(rename = "toolOutput")]
    pub tool_output: ThemeColor,

    // -- Markdown
    #[serde(rename = "mdHeading")]
    pub md_heading: ThemeColor,
    #[serde(rename = "mdLink")]
    pub md_link: ThemeColor,
    #[serde(rename = "mdLinkUrl")]
    pub md_link_url: ThemeColor,
    #[serde(rename = "mdCode")]
    pub md_code: ThemeColor,
    #[serde(rename = "mdCodeBlock")]
    pub md_code_block: ThemeColor,
    #[serde(rename = "mdCodeBlockBorder")]
    pub md_code_block_border: ThemeColor,
    #[serde(rename = "mdQuote")]
    pub md_quote: ThemeColor,
    #[serde(rename = "mdQuoteBorder")]
    pub md_quote_border: ThemeColor,
    #[serde(rename = "mdHr")]
    pub md_hr: ThemeColor,
    #[serde(rename = "mdListBullet")]
    pub md_list_bullet: ThemeColor,

    // -- Diffs
    #[serde(rename = "toolDiffAdded")]
    pub tool_diff_added: ThemeColor,
    #[serde(rename = "toolDiffRemoved")]
    pub tool_diff_removed: ThemeColor,
    #[serde(rename = "toolDiffContext")]
    pub tool_diff_context: ThemeColor,

    // -- Syntax
    #[serde(rename = "syntaxComment")]
    pub syntax_comment: ThemeColor,
    #[serde(rename = "syntaxKeyword")]
    pub syntax_keyword: ThemeColor,
    #[serde(rename = "syntaxFunction")]
    pub syntax_function: ThemeColor,
    #[serde(rename = "syntaxVariable")]
    pub syntax_variable: ThemeColor,
    #[serde(rename = "syntaxString")]
    pub syntax_string: ThemeColor,
    #[serde(rename = "syntaxNumber")]
    pub syntax_number: ThemeColor,
    #[serde(rename = "syntaxType")]
    pub syntax_type: ThemeColor,
    #[serde(rename = "syntaxOperator")]
    pub syntax_operator: ThemeColor,
    #[serde(rename = "syntaxPunctuation")]
    pub syntax_punctuation: ThemeColor,

    // -- Thinking levels
    #[serde(rename = "thinkingOff")]
    pub thinking_off: ThemeColor,
    #[serde(rename = "thinkingMinimal")]
    pub thinking_minimal: ThemeColor,
    #[serde(rename = "thinkingLow")]
    pub thinking_low: ThemeColor,
    #[serde(rename = "thinkingMedium")]
    pub thinking_medium: ThemeColor,
    #[serde(rename = "thinkingHigh")]
    pub thinking_high: ThemeColor,
    #[serde(rename = "thinkingXhigh")]
    pub thinking_xhigh: ThemeColor,

    // -- Bash mode
    #[serde(rename = "bashMode")]
    pub bash_mode: ThemeColor,
}

/// A loaded theme ready for use.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Theme {
    pub name: String,
    /// Optional variable aliases (referenced by name in `colors`).
    #[serde(default)]
    pub vars: HashMap<String, ThemeColor>,
    pub colors: ThemeTokens,
    /// Source file path (not serialized).
    #[serde(skip)]
    pub source: PathBuf,
}

// endregion: --- Types

// region:    --- Discovery

/// Discover all custom themes from standard locations.
/// Project-local themes shadow global ones with the same name.
pub fn discover_themes(cwd: &Path, agent_dir: &Path) -> Vec<Theme> {
    let mut themes: Vec<Theme> = Vec::new();
    let mut seen: std::collections::HashSet<String> = std::collections::HashSet::new();

    // Project-local (higher priority)
    let project_dir = cwd.join(".cade").join("themes");
    load_themes_from_dir(&project_dir, &mut themes, &mut seen);

    // Global
    let global_dir = agent_dir.join("themes");
    load_themes_from_dir(&global_dir, &mut themes, &mut seen);

    themes
}

/// Load a single theme from a JSON or tmTheme file.
pub fn load_theme(path: &Path) -> crate::Result<Theme> {
    if path.extension().and_then(|e| e.to_str()) == Some("tmTheme") {
        return load_tmtheme(path);
    }
    
    let content = std::fs::read_to_string(path)
        .map_err(|e| crate::Error::custom(format!("read theme {}: {e}", path.display())))?;
    let mut theme: Theme = serde_json::from_str(&content)
        .map_err(|e| crate::Error::custom(format!("parse theme {}: {e}", path.display())))?;
    theme.source = path.to_path_buf();
    Ok(theme)
}

fn load_tmtheme(path: &Path) -> crate::Result<Theme> {
    let plist_value = plist::Value::from_file(path)
        .map_err(|e| crate::Error::custom(format!("parse tmTheme {}: {e}", path.display())))?;
        
    let dict = plist_value.into_dictionary()
        .ok_or_else(|| crate::Error::custom("root is not a dictionary"))?;
        
    let name = dict.get("name")
        .and_then(|v| v.as_string())
        .unwrap_or("unnamed").to_string();
        
    let mut theme = Theme {
        name,
        vars: HashMap::new(),
        colors: ThemeTokens::default(),
        source: path.to_path_buf(),
    };
    
    let settings_array = dict.get("settings")
        .and_then(|v| v.as_array())
        .ok_or_else(|| crate::Error::custom("missing settings array"))?;
        
    let mut global_settings = None;
    let mut scopes = HashMap::new();
    
    for item in settings_array {
        if let Some(item_dict) = item.as_dictionary() {
            let item_settings = item_dict.get("settings").and_then(|v| v.as_dictionary());
            if let Some(settings) = item_settings {
                if let Some(scope) = item_dict.get("scope").and_then(|v| v.as_string()) {
                    scopes.insert(scope.to_string(), settings.clone());
                } else if global_settings.is_none() {
                    global_settings = Some(settings.clone());
                }
            }
        }
    }
    
    let to_color = |v: Option<&plist::Value>| -> Option<ThemeColor> {
        v.and_then(|val| val.as_string()).map(|s| ThemeColor::Hex(s.to_string()))
    };
    
    let mut fallback_bg = ThemeColor::default();
    let mut fallback_fg = ThemeColor::default();
    
    if let Some(global) = global_settings {
        if let Some(bg) = to_color(global.get("background")) {
            theme.colors.user_message_bg = bg.clone();
            theme.colors.tool_pending_bg = bg.clone();
            theme.colors.bash_mode = bg.clone();
            fallback_bg = bg.clone();
        }
        if let Some(fg) = to_color(global.get("foreground")) {
            theme.colors.text = fg.clone();
            fallback_fg = fg.clone();
        }
        if let Some(sel) = to_color(global.get("selection")) {
            theme.colors.selected_bg = sel.clone();
            theme.colors.tool_diff_context = sel.clone();
        }
        if let Some(line_hl) = to_color(global.get("lineHighlight")) {
            theme.colors.border = line_hl.clone();
            theme.colors.dim = line_hl.clone();
            theme.colors.muted = line_hl.clone();
        } else {
            theme.colors.border = fallback_bg.clone();
            theme.colors.dim = fallback_bg.clone();
            theme.colors.muted = fallback_bg.clone();
        }
    }
    
    let get_scope_fg = |scope_str: &str| -> Option<ThemeColor> {
        for (scope, settings) in &scopes {
            if scope.split(',').any(|s| s.trim().starts_with(scope_str)) {
                return to_color(settings.get("foreground"));
            }
        }
        None
    };
    
    if let Some(accent) = get_scope_fg("keyword.control") {
        theme.colors.accent = accent.clone();
        theme.colors.tool_title = accent;
    } else {
        theme.colors.accent = fallback_fg.clone();
    }
    
    if let Some(success) = get_scope_fg("string") {
        theme.colors.success = success.clone();
        theme.colors.tool_success_bg = success;
    }
    
    if let Some(warning) = get_scope_fg("entity.name.function") {
        theme.colors.warning = warning;
    }
    
    if let Some(error) = get_scope_fg("invalid") {
        theme.colors.error = error.clone();
        theme.colors.tool_error_bg = error;
    }
    
    if let Some(link) = get_scope_fg("markup.underline.link") {
        theme.colors.md_link = link;
    }
    
    Ok(theme)
}

// endregion: --- Discovery

// region:    --- Support

fn load_themes_from_dir(
    dir: &Path,
    themes: &mut Vec<Theme>,
    seen: &mut std::collections::HashSet<String>,
) {
    if !dir.exists() {
        return;
    }
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.extension().and_then(|e| e.to_str()) != Some("json") && path.extension().and_then(|e| e.to_str()) != Some("tmTheme") {
            continue;
        }
        match load_theme(&path) {
            Ok(theme) => {
                if !seen.contains(&theme.name) {
                    seen.insert(theme.name.clone());
                    themes.push(theme);
                }
            }
            Err(e) => tracing::warn!("Skipping theme {}: {e}", path.display()),
        }
    }
}

// endregion: --- Support

// region:    --- Tests

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::TempDir;

    fn make_dir() -> TempDir {
        tempfile::tempdir().expect("tempdir")
    }

    #[test]
    fn test_discover_themes_empty() {
        // -- Setup & Fixtures
        let cwd = make_dir();
        let agent_dir = make_dir();

        // -- Exec
        let themes = discover_themes(cwd.path(), agent_dir.path());

        // -- Check
        assert!(themes.is_empty());
    }

    #[test]
    fn test_load_theme_valid() {
        // -- Setup & Fixtures
        let dir = make_dir();
        let path = dir.path().join("mytheme.json");
        // A valid theme must have all required token fields.
        // We use serde_json::to_string of a Theme with default colors.
        let theme = Theme {
            name: "mytheme".to_string(),
            vars: Default::default(),
            colors: ThemeTokens::default(),
            source: PathBuf::new(),
        };
        let json_str = serde_json::to_string(&theme).unwrap();
        fs::write(&path, json_str).unwrap();

        // -- Exec
        let loaded = load_theme(&path).unwrap();

        // -- Check
        assert_eq!(loaded.name, "mytheme");
        assert_eq!(loaded.source, path);
    }

    #[test]
    fn test_theme_color_hex_serde() {
        // -- Setup & Fixtures
        let color = ThemeColor::Hex("#ff0000".to_string());

        // -- Exec
        let json = serde_json::to_string(&color).unwrap();
        let back: ThemeColor = serde_json::from_str(&json).unwrap();

        // -- Check
        assert_eq!(back, color);
    }

    #[test]
    fn test_theme_color_index_serde() {
        // -- Setup & Fixtures
        let color = ThemeColor::Index(42);

        // -- Exec
        let json = serde_json::to_string(&color).unwrap();
        let back: ThemeColor = serde_json::from_str(&json).unwrap();

        // -- Check
        assert_eq!(back, color);
    }

    #[test]
    fn test_load_tmtheme_valid() {
        let dir = make_dir();
        let path = dir.path().join("mytheme.tmTheme");
        
        let xml = r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple Computer//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
    <key>name</key>
    <string>mytheme</string>
    <key>settings</key>
    <array>
        <dict>
            <key>settings</key>
            <dict>
                <key>background</key>
                <string>#1E1E1E</string>
                <key>foreground</key>
                <string>#D4D4D4</string>
                <key>selection</key>
                <string>#264F78</string>
                <key>lineHighlight</key>
                <string>#2A2D2E</string>
            </dict>
        </dict>
        <dict>
            <key>name</key>
            <string>Keyword</string>
            <key>scope</key>
            <string>keyword.control</string>
            <key>settings</key>
            <dict>
                <key>foreground</key>
                <string>#C586C0</string>
            </dict>
        </dict>
    </array>
</dict>
</plist>"#;
        fs::write(&path, xml).unwrap();

        let loaded = load_theme(&path).unwrap();
        assert_eq!(loaded.name, "mytheme");
        assert_eq!(loaded.source, path);
        // The parser should extract #1E1E1E for userMessageBg (from background)
        assert_eq!(loaded.colors.user_message_bg, ThemeColor::Hex("#1E1E1E".to_string()));
        // The parser should extract #D4D4D4 for text (from foreground)
        assert_eq!(loaded.colors.text, ThemeColor::Hex("#D4D4D4".to_string()));
        // The parser should extract #C586C0 for accent (from keyword.control)
        assert_eq!(loaded.colors.accent, ThemeColor::Hex("#C586C0".to_string()));
    }
}

// endregion: --- Tests
