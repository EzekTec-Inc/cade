//! Keyboard shortcuts for the cade-gui dashboard.
//!
//! All shortcut definitions live here so they are testable and documented
//! in a single place.  The render loop in `app.rs` calls
//! [`ShortcutAction::poll`] once per frame to check for fired shortcuts.

/// Actions that can be triggered by a keyboard shortcut.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ShortcutAction {
    /// Send the current message (Enter or Ctrl+Enter).
    Send,
    /// Insert a newline in the input (Shift+Enter).
    InsertNewline,
    /// Dismiss the error toast (Escape).
    DismissError,
    /// Focus the chat input box (Ctrl+L or `/`).
    FocusInput,
    /// Open the slash-command palette (Ctrl+K).
    OpenPalette,
    /// Close the palette without executing anything (Escape while open).
    ClosePalette,
    /// Move palette highlight up (ArrowUp while open).
    PalettePrev,
    /// Move palette highlight down (ArrowDown while open).
    PaletteNext,
    /// Execute the currently-highlighted palette entry (Enter while open).
    PaletteExecute,
}

/// A shortcut definition: modifier flags + key.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Shortcut {
    pub ctrl: bool,
    pub shift: bool,
    pub alt: bool,
    pub key: egui::Key,
}

impl Shortcut {
    pub const fn new(key: egui::Key) -> Self {
        Self {
            ctrl: false,
            shift: false,
            alt: false,
            key,
        }
    }

    pub const fn ctrl(mut self) -> Self {
        self.ctrl = true;
        self
    }

    pub const fn shift(mut self) -> Self {
        self.shift = true;
        self
    }
}

/// All registered shortcuts, checked in priority order.
///
/// The first match wins, so more-specific combos (Shift+Enter) must come
/// before less-specific ones (Enter).
///
/// Palette-specific actions (`ClosePalette`, `PalettePrev`, `PaletteNext`,
/// `PaletteExecute`) are **not** in this table — they share keys with
/// non-palette actions (Esc, ArrowUp, ArrowDown, Enter) and must be
/// disambiguated by `app.rs` based on `session.is_palette_open()`.
pub const SHORTCUTS: &[(Shortcut, ShortcutAction)] = &[
    // Shift+Enter → newline (must be before bare Enter)
    (
        Shortcut::new(egui::Key::Enter).shift(),
        ShortcutAction::InsertNewline,
    ),
    // Enter → send (when input is focused and non-empty)
    (Shortcut::new(egui::Key::Enter), ShortcutAction::Send),
    // Ctrl+Enter → send (explicit)
    (
        Shortcut::new(egui::Key::Enter).ctrl(),
        ShortcutAction::Send,
    ),
    // Escape → dismiss error toast (also closes palette, handled in app.rs)
    (Shortcut::new(egui::Key::Escape), ShortcutAction::DismissError),
    // Ctrl+L → focus input
    (
        Shortcut::new(egui::Key::L).ctrl(),
        ShortcutAction::FocusInput,
    ),
    // Ctrl+K → open slash-command palette
    (
        Shortcut::new(egui::Key::K).ctrl(),
        ShortcutAction::OpenPalette,
    ),
];

/// Check which shortcut, if any, was pressed this frame.
///
/// Takes an `egui::InputState` snapshot (obtained via `ui.input(|i| ...)`
/// in the render loop) and returns the first matching action.
pub fn poll_shortcut(input: &egui::InputState) -> Option<ShortcutAction> {
    for (shortcut, action) in SHORTCUTS {
        if input.key_pressed(shortcut.key)
            && input.modifiers.ctrl == shortcut.ctrl
            && input.modifiers.shift == shortcut.shift
            && input.modifiers.alt == shortcut.alt
        {
            return Some(*action);
        }
    }
    None
}

/// Human-readable label for a shortcut, for tooltips and footer hints.
pub fn shortcut_hint(action: ShortcutAction) -> &'static str {
    match action {
        ShortcutAction::Send => "Enter",
        ShortcutAction::InsertNewline => "Shift+Enter",
        ShortcutAction::DismissError => "Esc",
        ShortcutAction::FocusInput => "Ctrl+L",
        ShortcutAction::OpenPalette => "Ctrl+K",
        ShortcutAction::ClosePalette => "Esc",
        ShortcutAction::PalettePrev => "↑",
        ShortcutAction::PaletteNext => "↓",
        ShortcutAction::PaletteExecute => "Enter",
    }
}

// ── Tests ───────────────────────────────────────────────────────────
#[cfg(test)]
mod tests {
    use super::*;

    // Helpers to construct minimal `egui::InputState` snapshots are
    // not easily possible without a running egui context.  Instead we
    // test the data-layer guarantees that don't require input polling.

    #[test]
    fn shortcut_table_has_all_actions() {
        let actions: Vec<ShortcutAction> = SHORTCUTS.iter().map(|(_, a)| *a).collect();
        assert!(actions.contains(&ShortcutAction::Send));
        assert!(actions.contains(&ShortcutAction::InsertNewline));
        assert!(actions.contains(&ShortcutAction::DismissError));
        assert!(actions.contains(&ShortcutAction::FocusInput));
    }

    #[test]
    fn shift_enter_comes_before_enter() {
        // Shift+Enter must be checked before bare Enter so it wins.
        let positions: Vec<(usize, ShortcutAction)> = SHORTCUTS
            .iter()
            .enumerate()
            .map(|(i, (_, a))| (i, *a))
            .collect();
        let shift_enter_pos = positions
            .iter()
            .find(|(_, a)| *a == ShortcutAction::InsertNewline)
            .unwrap()
            .0;
        let enter_pos = positions
            .iter()
            .find(|(_, a)| *a == ShortcutAction::Send)
            .unwrap()
            .0;
        assert!(
            shift_enter_pos < enter_pos,
            "Shift+Enter must be checked before Enter"
        );
    }

    #[test]
    fn shortcut_hint_returns_nonempty() {
        assert!(!shortcut_hint(ShortcutAction::Send).is_empty());
        assert!(!shortcut_hint(ShortcutAction::InsertNewline).is_empty());
        assert!(!shortcut_hint(ShortcutAction::DismissError).is_empty());
        assert!(!shortcut_hint(ShortcutAction::FocusInput).is_empty());
    }

    #[test]
    fn shortcut_builder_sets_modifiers() {
        let s = Shortcut::new(egui::Key::Enter).ctrl().shift();
        assert!(s.ctrl);
        assert!(s.shift);
        assert!(!s.alt);
        assert_eq!(s.key, egui::Key::Enter);
    }

    #[test]
    fn shortcut_builder_plain_key() {
        let s = Shortcut::new(egui::Key::Escape);
        assert!(!s.ctrl);
        assert!(!s.shift);
        assert!(!s.alt);
        assert_eq!(s.key, egui::Key::Escape);
    }

    #[test]
    fn all_shortcut_actions_have_hints() {
        // Every ShortcutAction variant should produce a non-empty hint.
        let all_actions = [
            ShortcutAction::Send,
            ShortcutAction::InsertNewline,
            ShortcutAction::DismissError,
            ShortcutAction::FocusInput,
            ShortcutAction::OpenPalette,
            ShortcutAction::ClosePalette,
            ShortcutAction::PalettePrev,
            ShortcutAction::PaletteNext,
            ShortcutAction::PaletteExecute,
        ];
        for action in all_actions {
            let hint = shortcut_hint(action);
            assert!(
                !hint.is_empty(),
                "Missing hint for {:?}",
                action
            );
        }
    }

    #[test]
    fn open_palette_is_bound_to_ctrl_k() {
        let hit = SHORTCUTS
            .iter()
            .find(|(_, a)| *a == ShortcutAction::OpenPalette)
            .expect("OpenPalette must be in SHORTCUTS");
        assert_eq!(hit.0.key, egui::Key::K);
        assert!(hit.0.ctrl);
        assert!(!hit.0.shift);
        assert!(!hit.0.alt);
    }

    #[test]
    fn no_duplicate_shortcut_bindings() {
        // Each (key, modifiers) combo should appear at most once
        // (except Send which has Enter and Ctrl+Enter — same action, different combos).
        let combos: Vec<_> = SHORTCUTS
            .iter()
            .map(|(s, _)| (s.key, s.ctrl, s.shift, s.alt))
            .collect();
        for (i, a) in combos.iter().enumerate() {
            for (j, b) in combos.iter().enumerate() {
                if i != j {
                    assert_ne!(
                        a, b,
                        "Duplicate shortcut binding at indices {} and {}",
                        i, j
                    );
                }
            }
        }
    }
}
