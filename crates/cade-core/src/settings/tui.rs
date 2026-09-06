//! Decoupled TUI / CLI Appearance & Ergonomics Configuration (`tui.toml`).
//!
//! Separates UI visual preferences, theme selections, diff layouts, leader chords,
//! and notifications from agent runtime policies.

// region:    --- Imports

use serde::{Deserialize, Serialize};
use std::path::Path;

// endregion: --- Imports

// region:    --- Types

/// Top-level settings loaded from `tui.toml`.
#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq)]
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
    #[serde(default = "default_true")]
    pub enable_mouse: bool,
}

impl Default for ScrollSettings {
    fn default() -> Self {
        Self {
            speed_multiplier: 1.0,
            enable_mouse: true,
        }
    }
}

fn default_true() -> bool {
    true
}

fn default_speed() -> f32 {
    1.0
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
        self.notifications.enable_bell = other.notifications.enable_bell;
        self.notifications.enable_osc = other.notifications.enable_osc;
        self.scrolling.speed_multiplier = other.scrolling.speed_multiplier;
        self.scrolling.enable_mouse = other.scrolling.enable_mouse;
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

        let merged = base.merge(override_layer);
        assert_eq!(merged.theme.default_theme.as_deref(), Some("tokyo-night"));
        assert_eq!(merged.diff.default_layout.as_deref(), Some("side-by-side"));
        assert!(merged.notifications.enable_bell);
    }
}

// endregion: --- Tests
