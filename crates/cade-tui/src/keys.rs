//! Keymap and Key Chord System for CADE TUI (OpenCode TUI Parity).
//!
//! Provides normalized chord parsing, configurable keybindings via `tui.toml`,
//! and leader key (`<leader>`) chord routing.

// region:    --- Imports

use cade_core::settings::tui::{KeybindSpec, TuiSettings};
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use std::collections::HashMap;

// endregion: --- Imports

// region:    --- Types

/// Canonical identifiers for all actions bindable to key chords.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum TuiActionId {
    // -- Session
    SessionNew,
    SessionList,
    SessionCompact,
    SessionTimeline,
    SessionRename,

    // -- Palette & Help
    CommandPalette,
    HelpOverlay,
    SidebarToggle,

    // -- Copy & History
    CopyMessage,
    Undo,
    Redo,

    // -- Selection
    QuoteSelection,
    DropSelection,
    ToggleConceal,

    // -- Pickers & Perms
    ModelPicker,
    ThemePicker,
    TogglePermissions,

    // -- Scroll Navigation
    ScrollPageUp,
    ScrollPageDown,
    ScrollHalfPageUp,
    ScrollHalfPageDown,
    ScrollHome,
    ScrollEnd,

    // -- Prompt
    PromptSubmit,
    PromptNewline,
    PromptDeleteToEnd,
    PromptDeleteToStart,
    PromptPlainTextPaste,
    PromptExternalEditor,
}

impl TuiActionId {
    /// Return the string identifier used in `tui.toml` [keybinds] table.
    pub fn config_key(&self) -> &'static str {
        match self {
            Self::SessionNew => "session_new",
            Self::SessionList => "session_list",
            Self::SessionCompact => "session_compact",
            Self::SessionTimeline => "session_timeline",
            Self::SessionRename => "session_rename",

            Self::CommandPalette => "command_palette",
            Self::HelpOverlay => "help_overlay",
            Self::SidebarToggle => "sidebar_toggle",

            Self::CopyMessage => "copy_message",
            Self::Undo => "undo",
            Self::Redo => "redo",

            Self::QuoteSelection => "quote_selection",
            Self::DropSelection => "drop_selection",
            Self::ToggleConceal => "toggle_conceal",

            Self::ModelPicker => "model_picker",
            Self::ThemePicker => "theme_picker",
            Self::TogglePermissions => "toggle_permissions",

            Self::ScrollPageUp => "scroll_page_up",
            Self::ScrollPageDown => "scroll_page_down",
            Self::ScrollHalfPageUp => "scroll_half_page_up",
            Self::ScrollHalfPageDown => "scroll_half_page_down",
            Self::ScrollHome => "scroll_home",
            Self::ScrollEnd => "scroll_end",

            Self::PromptSubmit => "prompt_submit",
            Self::PromptNewline => "prompt_newline",
            Self::PromptDeleteToEnd => "prompt_delete_to_end",
            Self::PromptDeleteToStart => "prompt_delete_to_start",
            Self::PromptPlainTextPaste => "prompt_plain_text_paste",
            Self::PromptExternalEditor => "prompt_external_editor",
        }
    }

    /// Match an action id from its config name.
    pub fn from_config_key(s: &str) -> Option<Self> {
        match s.trim().to_lowercase().as_str() {
            "session_new" => Some(Self::SessionNew),
            "session_list" => Some(Self::SessionList),
            "session_compact" => Some(Self::SessionCompact),
            "session_timeline" => Some(Self::SessionTimeline),
            "session_rename" => Some(Self::SessionRename),

            "command_palette" => Some(Self::CommandPalette),
            "help_overlay" => Some(Self::HelpOverlay),
            "sidebar_toggle" => Some(Self::SidebarToggle),

            "copy_message" => Some(Self::CopyMessage),
            "undo" => Some(Self::Undo),
            "redo" => Some(Self::Redo),

            "quote_selection" => Some(Self::QuoteSelection),
            "drop_selection" => Some(Self::DropSelection),
            "toggle_conceal" => Some(Self::ToggleConceal),

            "model_picker" => Some(Self::ModelPicker),
            "theme_picker" => Some(Self::ThemePicker),
            "toggle_permissions" => Some(Self::TogglePermissions),

            "scroll_page_up" => Some(Self::ScrollPageUp),
            "scroll_page_down" => Some(Self::ScrollPageDown),
            "scroll_half_page_up" => Some(Self::ScrollHalfPageUp),
            "scroll_half_page_down" => Some(Self::ScrollHalfPageDown),
            "scroll_home" => Some(Self::ScrollHome),
            "scroll_end" => Some(Self::ScrollEnd),

            "prompt_submit" => Some(Self::PromptSubmit),
            "prompt_newline" => Some(Self::PromptNewline),
            "prompt_delete_to_end" => Some(Self::PromptDeleteToEnd),
            "prompt_delete_to_start" => Some(Self::PromptDeleteToStart),
            "prompt_plain_text_paste" => Some(Self::PromptPlainTextPaste),
            "prompt_external_editor" => Some(Self::PromptExternalEditor),

            _ => None,
        }
    }
}

/// Normalized single key combination or leader sub-chord.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct KeyChord {
    pub modifiers: KeyModifiers,
    pub code: KeyCode,
    pub is_leader: bool,
}

impl KeyChord {
    /// Create a direct (non-leader) key chord.
    pub fn direct(code: KeyCode, modifiers: KeyModifiers) -> Self {
        Self {
            modifiers,
            code,
            is_leader: false,
        }
    }

    /// Create a leader sub-chord (e.g. `<leader>p`).
    pub fn leader(code: KeyCode, modifiers: KeyModifiers) -> Self {
        Self {
            modifiers,
            code,
            is_leader: true,
        }
    }

    /// Parse a chord string like `"ctrl+x"`, `"<leader>p"`, `"ctrl+shift+c"`.
    pub fn parse(s: &str) -> Option<Self> {
        let mut trimmed = s.trim();
        if trimmed.is_empty() || trimmed.eq_ignore_ascii_case("none") {
            return None;
        }

        let mut is_leader = false;
        let mut modifiers = KeyModifiers::empty();

        let s_lower = trimmed.to_lowercase();
        if s_lower.starts_with("<leader>") {
            is_leader = true;
            trimmed = trimmed["<leader>".len()..].trim();
            if trimmed.starts_with('+') || trimmed.starts_with('-') {
                trimmed = trimmed[1..].trim();
            }
        } else if s_lower.starts_with("leader")
            && s_lower.len() > 6
            && (s_lower.as_bytes()[6] == b'+'
                || s_lower.as_bytes()[6] == b'-'
                || s_lower.as_bytes()[6] == b' ')
        {
            is_leader = true;
            trimmed = trimmed[6..].trim();
            if trimmed.starts_with('+') || trimmed.starts_with('-') {
                trimmed = trimmed[1..].trim();
            }
        }

        let s_lower = trimmed.to_lowercase();
        let tokens: Vec<&str> = if s_lower.contains('+') {
            trimmed.split('+').map(str::trim).collect()
        } else if s_lower.contains('-') && !s_lower.starts_with('-') {
            trimmed.split('-').map(str::trim).collect()
        } else if s_lower.contains(' ') {
            trimmed.split_whitespace().map(str::trim).collect()
        } else {
            vec![trimmed]
        };

        let mut key_part = "";
        for token in tokens {
            let lower = token.to_lowercase();
            match lower.as_str() {
                "<leader>" | "leader" => is_leader = true,
                "ctrl" | "control" => modifiers.insert(KeyModifiers::CONTROL),
                "alt" | "opt" | "option" | "meta" => modifiers.insert(KeyModifiers::ALT),
                "shift" => modifiers.insert(KeyModifiers::SHIFT),
                _ => key_part = token,
            }
        }

        if key_part.is_empty() {
            return None;
        }

        let code = if key_part.chars().count() == 1 {
            let ch = key_part.chars().next().unwrap();
            if ch.is_uppercase() {
                modifiers.insert(KeyModifiers::SHIFT);
                KeyCode::Char(ch)
            } else if modifiers.contains(KeyModifiers::SHIFT) {
                KeyCode::Char(ch.to_ascii_uppercase())
            } else {
                KeyCode::Char(ch.to_ascii_lowercase())
            }
        } else {
            match key_part.to_lowercase().as_str() {
                "enter" | "return" => KeyCode::Enter,
                "esc" | "escape" => KeyCode::Esc,
                "tab" => KeyCode::Tab,
                "backspace" => KeyCode::Backspace,
                "delete" | "del" => KeyCode::Delete,
                "up" => KeyCode::Up,
                "down" => KeyCode::Down,
                "left" => KeyCode::Left,
                "right" => KeyCode::Right,
                "pageup" | "pgup" => KeyCode::PageUp,
                "pagedown" | "pgdn" => KeyCode::PageDown,
                "home" => KeyCode::Home,
                "end" => KeyCode::End,
                "space" => KeyCode::Char(' '),
                _ => return None,
            }
        };

        Some(Self {
            modifiers,
            code,
            is_leader,
        })
    }

    /// Check if this chord matches a runtime KeyEvent.
    pub fn matches_event(&self, event: &KeyEvent, leader_pending: bool) -> bool {
        if self.is_leader != leader_pending {
            return false;
        }

        // Compare key code case-insensitively for Char
        let code_matches = match (self.code, event.code) {
            (KeyCode::Char(a), KeyCode::Char(b)) => {
                a.to_ascii_lowercase() == b.to_ascii_lowercase()
            }
            (a, b) => a == b,
        };

        if !code_matches {
            return false;
        }

        if self.is_leader {
            let chord_has_shift = self.modifiers.contains(KeyModifiers::SHIFT);
            let event_has_shift = event.modifiers.contains(KeyModifiers::SHIFT)
                || match event.code {
                    KeyCode::Char(c) => c.is_uppercase(),
                    _ => false,
                };
            if chord_has_shift != event_has_shift {
                return false;
            }
            (self.modifiers - KeyModifiers::SHIFT) == (event.modifiers - KeyModifiers::SHIFT)
        } else {
            // Direct chords: exact modifier match
            self.modifiers == event.modifiers
                || (self.modifiers.contains(KeyModifiers::SHIFT)
                    && match event.code {
                        KeyCode::Char(c) => {
                            c.is_uppercase()
                                && (self.modifiers - KeyModifiers::SHIFT)
                                    == (event.modifiers - KeyModifiers::SHIFT)
                        }
                        _ => false,
                    })
        }
    }
}

// endregion: --- Types

// region:    --- Keymap

/// Bidirectional map between chords and actions, populated with OpenCode defaults
/// and customizable via `tui.toml`.
#[derive(Debug, Clone)]
pub struct Keymap {
    chords_to_action: HashMap<KeyChord, TuiActionId>,
    leader_chord: KeyChord,
    leader_timeout_ms: u64,
}

impl Default for Keymap {
    fn default() -> Self {
        Self::with_settings(&TuiSettings::default())
    }
}

impl Keymap {
    /// Construct a keymap using settings from `tui.toml`.
    pub fn with_settings(settings: &TuiSettings) -> Self {
        let leader_str = settings.resolved_leader();
        let leader_chord = KeyChord::parse(leader_str).unwrap_or(KeyChord {
            modifiers: KeyModifiers::CONTROL,
            code: KeyCode::Char('x'),
            is_leader: false,
        });

        let mut km = Self {
            chords_to_action: HashMap::new(),
            leader_chord,
            leader_timeout_ms: settings.resolved_leader_timeout_ms(),
        };

        km.load_defaults();
        km.apply_settings(settings);
        km
    }

    /// Load default OpenCode keybindings.
    fn load_defaults(&mut self) {
        // Session
        self.bind("<leader>n", TuiActionId::SessionNew);
        self.bind("<leader>l", TuiActionId::SessionList);
        self.bind("<leader>c", TuiActionId::SessionCompact);
        self.bind("<leader>g", TuiActionId::SessionTimeline);
        self.bind("ctrl+r", TuiActionId::SessionRename);

        // Palette & Help
        self.bind("ctrl+p", TuiActionId::CommandPalette);
        self.bind("<leader>h", TuiActionId::HelpOverlay);
        self.bind("<leader>?", TuiActionId::HelpOverlay);
        self.bind("<leader>b", TuiActionId::SidebarToggle);

        // Copy & History
        self.bind("<leader>y", TuiActionId::CopyMessage);
        self.bind("<leader>u", TuiActionId::Undo);
        self.bind("<leader>r", TuiActionId::Redo);

        // Selection & Quoting
        self.bind("<leader>p", TuiActionId::QuoteSelection);
        self.bind("ctrl+shift+c", TuiActionId::QuoteSelection);
        self.bind("esc", TuiActionId::DropSelection);

        // Pickers & Perms
        self.bind("<leader>m", TuiActionId::ModelPicker);
        self.bind("<leader>t", TuiActionId::ThemePicker);
        self.bind("<leader>s", TuiActionId::SessionList);
        self.bind("<leader>P", TuiActionId::TogglePermissions);

        // Scroll
        self.bind("pageup", TuiActionId::ScrollPageUp);
        self.bind("pagedown", TuiActionId::ScrollPageDown);
        self.bind("ctrl+alt+u", TuiActionId::ScrollHalfPageUp);
        self.bind("ctrl+alt+d", TuiActionId::ScrollHalfPageDown);
        self.bind("home", TuiActionId::ScrollHome);
        self.bind("end", TuiActionId::ScrollEnd);

        // Prompt
        self.bind("ctrl+alt+v", TuiActionId::PromptPlainTextPaste);
    }

    /// Apply user overrides from `tui.toml`.
    pub fn apply_settings(&mut self, settings: &TuiSettings) {
        for (action_str, spec) in &settings.keybinds {
            if let Some(action) = TuiActionId::from_config_key(action_str) {
                if spec.is_disabled() {
                    self.unbind_action(action);
                } else {
                    self.unbind_action(action);
                    for chord_str in spec.chords() {
                        self.bind(&chord_str, action);
                    }
                }
            }
        }
    }

    /// Bind a chord string to an action.
    pub fn bind(&mut self, chord_str: &str, action: TuiActionId) {
        if let Some(chord) = KeyChord::parse(chord_str) {
            self.chords_to_action.insert(chord, action);
        }
    }

    /// Unbind all chords pointing to `action`.
    pub fn unbind_action(&mut self, action: TuiActionId) {
        self.chords_to_action.retain(|_, a| *a != action);
    }

    /// The parsed leader chord (default `ctrl+x`).
    pub fn leader_chord(&self) -> &KeyChord {
        &self.leader_chord
    }

    /// Timeout in milliseconds for pending leader chord.
    pub fn leader_timeout_ms(&self) -> u64 {
        self.leader_timeout_ms
    }

    /// Check if an incoming key event matches the leader activation chord.
    pub fn is_leader_event(&self, event: &KeyEvent) -> bool {
        self.leader_chord.matches_event(event, false)
    }

    /// Resolve an incoming key event against active keybindings.
    pub fn resolve(&self, event: &KeyEvent, leader_pending: bool) -> Option<TuiActionId> {
        for (chord, action) in &self.chords_to_action {
            if chord.matches_event(event, leader_pending) {
                return Some(*action);
            }
        }
        None
    }
}

// endregion: --- Keymap

// region:    --- Tests

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_chord_parsing() {
        let c1 = KeyChord::parse("ctrl+x").expect("ctrl+x");
        assert_eq!(c1.modifiers, KeyModifiers::CONTROL);
        assert_eq!(c1.code, KeyCode::Char('x'));
        assert!(!c1.is_leader);

        let c2 = KeyChord::parse("<leader>p").expect("<leader>p");
        assert_eq!(c2.code, KeyCode::Char('p'));
        assert!(c2.is_leader);

        let c3 = KeyChord::parse("ctrl+shift+c").expect("ctrl+shift+c");
        assert!(c3.modifiers.contains(KeyModifiers::CONTROL));
        assert!(c3.modifiers.contains(KeyModifiers::SHIFT));
        assert_eq!(c3.code, KeyCode::Char('C'));
        assert!(!c3.is_leader);

        let c4 = KeyChord::parse("none");
        assert!(c4.is_none());
    }

    #[test]
    fn test_keymap_resolution() {
        let km = Keymap::default();

        // Direct Ctrl+P should resolve to CommandPalette
        let evt_palette = KeyEvent::new(KeyCode::Char('p'), KeyModifiers::CONTROL);
        assert_eq!(
            km.resolve(&evt_palette, false),
            Some(TuiActionId::CommandPalette)
        );

        // When leader is pending, 'p' should resolve to QuoteSelection
        let evt_p = KeyEvent::new(KeyCode::Char('p'), KeyModifiers::NONE);
        assert_eq!(km.resolve(&evt_p, true), Some(TuiActionId::QuoteSelection));

        // When leader is pending, 'y' should resolve to CopyMessage
        let evt_y = KeyEvent::new(KeyCode::Char('y'), KeyModifiers::NONE);
        assert_eq!(km.resolve(&evt_y, true), Some(TuiActionId::CopyMessage));
    }

    #[test]
    fn test_keymap_user_override_and_disable() {
        let mut settings = TuiSettings::default();
        // Disable quote_selection
        settings.keybinds.insert(
            "quote_selection".to_string(),
            KeybindSpec::Single("none".to_string()),
        );
        // Remap command_palette to ctrl+k
        settings.keybinds.insert(
            "command_palette".to_string(),
            KeybindSpec::Single("ctrl+k".to_string()),
        );

        let km = Keymap::with_settings(&settings);

        let evt_p = KeyEvent::new(KeyCode::Char('p'), KeyModifiers::NONE);
        assert_eq!(km.resolve(&evt_p, true), None);

        let evt_old = KeyEvent::new(KeyCode::Char('p'), KeyModifiers::CONTROL);
        assert_eq!(km.resolve(&evt_old, false), None);

        let evt_new = KeyEvent::new(KeyCode::Char('k'), KeyModifiers::CONTROL);
        assert_eq!(
            km.resolve(&evt_new, false),
            Some(TuiActionId::CommandPalette)
        );
    }
}

// endregion: --- Tests
