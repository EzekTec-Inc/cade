//! Decoupled TUI / CLI Appearance & Ergonomics Configuration (`tui.toml`).
//!
//! Separates UI visual preferences, theme selections, diff layouts, leader chords,
//! and notifications from agent runtime policies.

// region:    --- Imports

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::Path;

// endregion: --- Imports

// region:    --- Types

/// Linux clipboard target buffer selection.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum LinuxClipboardSelection {
    Clipboard,
    Primary,
    Both,
}

impl Default for LinuxClipboardSelection {
    fn default() -> Self {
        Self::Both
    }
}

/// Specification for a keybinding in `tui.toml`.
///
/// Supports:
/// - Single chord: `"ctrl+x p"` or `"none"`
/// - Disabled: `false`
/// - Multiple chords: `["ctrl+x p", "ctrl+shift+c"]`
/// - Detailed mapping object: `{ key = "ctrl+x p", event = "...", prevent_default = true }`
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(untagged)]
pub enum KeybindSpec {
    Disabled(bool),
    Single(String),
    Multiple(Vec<String>),
    Detailed {
        key: String,
        #[serde(default)]
        event: Option<String>,
        #[serde(default)]
        prevent_default: Option<bool>,
    },
}

impl KeybindSpec {
    pub fn is_disabled(&self) -> bool {
        match self {
            KeybindSpec::Disabled(b) => !*b,
            KeybindSpec::Single(s) => s.trim().eq_ignore_ascii_case("none"),
            KeybindSpec::Multiple(v) => v.is_empty(),
            KeybindSpec::Detailed { key, .. } => key.trim().eq_ignore_ascii_case("none"),
        }
    }

    pub fn chords(&self) -> Vec<String> {
        match self {
            KeybindSpec::Disabled(_) => vec![],
            KeybindSpec::Single(s) => {
                if s.trim().eq_ignore_ascii_case("none") {
                    vec![]
                } else {
                    s.split(',')
                        .map(|x| x.trim().to_string())
                        .filter(|x| !x.is_empty())
                        .collect()
                }
            }
            KeybindSpec::Multiple(v) => v.clone(),
            KeybindSpec::Detailed { key, .. } => {
                if key.trim().eq_ignore_ascii_case("none") {
                    vec![]
                } else {
                    vec![key.clone()]
                }
            }
        }
    }
}

/// Top-level settings loaded from `tui.toml`.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct TuiSettings {
    #[serde(default)]
    pub theme: ThemeSettings,
    #[serde(default)]
    pub diff: DiffSettings,
    #[serde(default)]
    pub leader_keys: LeaderKeySettings,
    #[serde(default)]
    pub notifications: NotificationSettings,
    #[serde(default)]
    pub scrolling: ScrollSettings,

    #[serde(default)]
    pub linux_clipboard_selection: LinuxClipboardSelection,
    #[serde(default = "default_leader")]
    pub leader: String,
    #[serde(default = "default_leader_timeout_ms")]
    pub leader_timeout_ms: u64,
    #[serde(default)]
    pub keybinds: HashMap<String, KeybindSpec>,
}

impl Default for TuiSettings {
    fn default() -> Self {
        Self {
            theme: ThemeSettings::default(),
            diff: DiffSettings::default(),
            leader_keys: LeaderKeySettings::default(),
            notifications: NotificationSettings::default(),
            scrolling: ScrollSettings::default(),
            linux_clipboard_selection: LinuxClipboardSelection::default(),
            leader: default_leader(),
            leader_timeout_ms: default_leader_timeout_ms(),
            keybinds: HashMap::default(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq, Eq)]
pub struct ThemeSettings {
    #[serde(default)]
    pub default_theme: Option<String>,
    #[serde(default)]
    pub cursor_style: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq, Eq)]
pub struct DiffSettings {
    #[serde(default)]
    pub default_layout: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq, Eq)]
pub struct LeaderKeySettings {
    #[serde(default)]
    pub leader_key: Option<String>,
    #[serde(default)]
    pub timeout_ms: Option<u64>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct NotificationSettings {
    #[serde(default = "default_true")]
    pub enable_bell: bool,
    #[serde(default = "default_true")]
    pub enable_osc: bool,
}

impl Default for NotificationSettings {
    fn default() -> Self {
        Self {
            enable_bell: true,
            enable_osc: true,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ScrollSettings {
    #[serde(default = "default_speed")]
    pub speed_multiplier: f32,
    #[serde(default = "default_scroll_speed")]
    pub scroll_speed: u16,
    #[serde(default = "default_true")]
    pub scroll_acceleration: bool,
    #[serde(default = "default_true")]
    pub enable_mouse: bool,
}

impl Default for ScrollSettings {
    fn default() -> Self {
        Self {
            speed_multiplier: 1.0,
            scroll_speed: default_scroll_speed(),
            scroll_acceleration: true,
            enable_mouse: true,
        }
    }
}

fn default_scroll_speed() -> u16 {
    3
}

fn default_true() -> bool {
    true
}

fn default_speed() -> f32 {
    1.0
}

fn default_leader() -> String {
    "ctrl+x".to_string()
}

fn default_leader_timeout_ms() -> u64 {
    2000
}

// endregion: --- Types

// region:    --- Resolver

impl TuiSettings {
    /// Load `tui.toml` from a specific directory, or return default if missing.
    pub fn load_from_dir(dir: &Path) -> Self {
        let path = dir.join("tui.toml");
        if path.exists()
            && let Ok(content) = std::fs::read_to_string(&path)
            && let Ok(parsed) = toml::from_str::<TuiSettings>(&content)
        {
            return parsed;
        }
        Self::default()
    }

    /// Get the effective leader key string.
    pub fn resolved_leader(&self) -> &str {
        if let Some(ref l) = self.leader_keys.leader_key {
            l.as_str()
        } else {
            &self.leader
        }
    }

    /// Get the effective leader chord timeout in milliseconds.
    pub fn resolved_leader_timeout_ms(&self) -> u64 {
        self.leader_keys
            .timeout_ms
            .unwrap_or(self.leader_timeout_ms)
    }

    /// Hierarchically merge another TuiSettings layer onto `self`.
    pub fn merge(mut self, other: TuiSettings) -> Self {
        if other.theme.default_theme.is_some() {
            self.theme.default_theme = other.theme.default_theme;
        }
        if other.theme.cursor_style.is_some() {
            self.theme.cursor_style = other.theme.cursor_style;
        }
        if other.diff.default_layout.is_some() {
            self.diff.default_layout = other.diff.default_layout;
        }
        if other.leader_keys.leader_key.is_some() {
            self.leader_keys.leader_key = other.leader_keys.leader_key;
        }
        if other.leader_keys.timeout_ms.is_some() {
            self.leader_keys.timeout_ms = other.leader_keys.timeout_ms;
        }
        if other.leader != default_leader() {
            self.leader = other.leader;
        }
        if other.leader_timeout_ms != default_leader_timeout_ms() {
            self.leader_timeout_ms = other.leader_timeout_ms;
        }
        if other.linux_clipboard_selection != LinuxClipboardSelection::default() {
            self.linux_clipboard_selection = other.linux_clipboard_selection;
        }
        for (k, v) in other.keybinds {
            self.keybinds.insert(k, v);
        }
        self.notifications.enable_bell = other.notifications.enable_bell;
        self.notifications.enable_osc = other.notifications.enable_osc;
        self.scrolling.speed_multiplier = other.scrolling.speed_multiplier;
        self.scrolling.enable_mouse = other.scrolling.enable_mouse;
        if other.scrolling.scroll_speed != default_scroll_speed() {
            self.scrolling.scroll_speed = other.scrolling.scroll_speed;
        }
        self.scrolling.scroll_acceleration = other.scrolling.scroll_acceleration;
        self
    }
}

// endregion: --- Resolver

// region:    --- Tests

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_tui_settings_defaults_and_roundtrip() {
        let settings = TuiSettings::default();
        assert!(settings.notifications.enable_bell);
        assert!(settings.notifications.enable_osc);
        assert_eq!(settings.scrolling.speed_multiplier, 1.0);
        assert_eq!(
            settings.linux_clipboard_selection,
            LinuxClipboardSelection::Both
        );
        assert_eq!(settings.resolved_leader(), "ctrl+x");
        assert_eq!(settings.resolved_leader_timeout_ms(), 2000);

        let toml_str = toml::to_string_pretty(&settings).unwrap_or_default();
        assert!(!toml_str.is_empty());

        let parsed: TuiSettings = toml::from_str(&toml_str).unwrap_or_default();
        assert_eq!(parsed, settings);
    }

    #[test]
    fn test_tui_settings_merge() {
        let base = TuiSettings::default();
        let mut override_layer = TuiSettings::default();
        override_layer.theme.default_theme = Some("tokyo-night".to_string());
        override_layer.diff.default_layout = Some("side-by-side".to_string());
        override_layer.linux_clipboard_selection = LinuxClipboardSelection::Primary;
        override_layer.leader = "ctrl+a".to_string();
        override_layer.leader_timeout_ms = 1500;
        override_layer.keybinds.insert(
            "quote_selection".to_string(),
            KeybindSpec::Single("<leader>q".to_string()),
        );

        let merged = base.merge(override_layer);
        assert_eq!(merged.theme.default_theme.as_deref(), Some("tokyo-night"));
        assert_eq!(merged.diff.default_layout.as_deref(), Some("side-by-side"));
        assert_eq!(
            merged.linux_clipboard_selection,
            LinuxClipboardSelection::Primary
        );
        assert_eq!(merged.resolved_leader(), "ctrl+a");
        assert_eq!(merged.resolved_leader_timeout_ms(), 1500);
        assert_eq!(
            merged.keybinds.get("quote_selection"),
            Some(&KeybindSpec::Single("<leader>q".to_string()))
        );
        assert!(merged.notifications.enable_bell);
    }

    #[test]
    fn test_keybind_spec_parsing() {
        let toml_data = r#"
            disabled_bool = false
            single_chord = "ctrl+x p"
            none_chord = "none"
            multi_chord = ["ctrl+p", "ctrl+k"]

            [detailed]
            key = "ctrl+e"
            event = "external_editor"
            prevent_default = true
        "#;

        #[derive(Deserialize)]
        struct KeybindTest {
            disabled_bool: KeybindSpec,
            single_chord: KeybindSpec,
            none_chord: KeybindSpec,
            multi_chord: KeybindSpec,
            detailed: KeybindSpec,
        }

        let parsed: KeybindTest = toml::from_str(toml_data).expect("parse keybinds");
        assert!(parsed.disabled_bool.is_disabled());
        assert_eq!(parsed.disabled_bool.chords(), Vec::<String>::new());

        assert!(!parsed.single_chord.is_disabled());
        assert_eq!(parsed.single_chord.chords(), vec!["ctrl+x p"]);

        assert!(parsed.none_chord.is_disabled());
        assert_eq!(parsed.none_chord.chords(), Vec::<String>::new());

        assert!(!parsed.multi_chord.is_disabled());
        assert_eq!(parsed.multi_chord.chords(), vec!["ctrl+p", "ctrl+k"]);

        assert!(!parsed.detailed.is_disabled());
        assert_eq!(parsed.detailed.chords(), vec!["ctrl+e"]);
    }
}

// endregion: --- Tests
