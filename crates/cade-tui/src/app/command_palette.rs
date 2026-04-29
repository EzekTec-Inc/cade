//! Command palette — Ctrl+P fuzzy command launcher.
//!
//! Provides a VS-Code-style command palette overlay that lets users fuzzy-search
//! and execute any slash command without memorising the exact name.

use std::any::Any;

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

use cade_core::resources::palette::{fuzzy_score, CMD_DEFS};

use crate::colors::ThemeColors;
use crate::overlay_component::{OverlayComponent, OverlayInputResult};

// -- State

/// Active command palette state. `None` when the palette is closed.
#[derive(Debug, Clone)]
pub struct CommandPaletteState {
    /// User-typed search query.
    pub query: String,
    /// Indices into `CMD_DEFS` that match the current query, ordered by score.
    pub filtered: Vec<usize>,
    /// Currently highlighted entry in `filtered`.
    pub cursor: usize,
    /// Result to be returned to the host on dismiss (e.g. `"/help"`).
    result: Option<String>,
}

impl Default for CommandPaletteState {
    fn default() -> Self {
        Self::new()
    }
}

impl CommandPaletteState {
    /// Create a new palette with all commands and an empty query.
    pub fn new() -> Self {
        let filtered: Vec<usize> = (0..CMD_DEFS.len()).collect();
        Self {
            query: String::new(),
            filtered,
            cursor: 0,
            result: None,
        }
    }

    /// Update the filtered list based on the current query.
    pub fn update_filter(&mut self) {
        if self.query.is_empty() {
            self.filtered = (0..CMD_DEFS.len()).collect();
        } else {
            let q = self.query.to_lowercase();
            let mut scored: Vec<(usize, i32)> = CMD_DEFS
                .iter()
                .enumerate()
                .filter_map(|(i, cmd)| {
                    let section = match cmd.category {
                        cade_core::resources::palette::CmdCategory::Navigation => "Navigation",
                        cade_core::resources::palette::CmdCategory::Memory => "Memory",
                        cade_core::resources::palette::CmdCategory::Tools => "Tools",
                        cade_core::resources::palette::CmdCategory::Session => "Session",
                        cade_core::resources::palette::CmdCategory::Display => "Display",
                    };
                    fuzzy_score(&q, cmd.trigger, cmd.description, section)
                        .map(|score| (i, score))
                })
                .collect();
            scored.sort_by(|a, b| b.1.cmp(&a.1));
            self.filtered = scored.into_iter().map(|(i, _)| i).collect();
        }
        // Clamp cursor
        if self.filtered.is_empty() {
            self.cursor = 0;
        } else if self.cursor >= self.filtered.len() {
            self.cursor = self.filtered.len() - 1;
        }
    }

    /// Move cursor up by one.
    pub fn cursor_up(&mut self) {
        self.cursor = self.cursor.saturating_sub(1);
    }

    /// Move cursor down by one.
    pub fn cursor_down(&mut self) {
        if !self.filtered.is_empty() && self.cursor + 1 < self.filtered.len() {
            self.cursor += 1;
        }
    }

    /// Get the currently selected command label, if any.
    pub fn selected_command(&self) -> Option<&'static str> {
        self.filtered
            .get(self.cursor)
            .and_then(|&idx| CMD_DEFS.get(idx))
            .map(|cmd| cmd.trigger)
    }

    /// Push a character into the query and re-filter.
    pub fn push_char(&mut self, c: char) {
        self.query.push(c);
        self.update_filter();
    }

    /// Pop a character from the query and re-filter.
    pub fn pop_char(&mut self) {
        self.query.pop();
        self.update_filter();
    }
}

// -- OverlayComponent impl

impl OverlayComponent for CommandPaletteState {
    fn id(&self) -> &'static str {
        "command_palette"
    }

    fn render_overlay(
        &mut self,
        frame: &mut ratatui::Frame,
        area: ratatui::layout::Rect,
        colors: &ThemeColors,
    ) {
        super::layout::command_palette::render_command_palette(frame, self, area, colors);
    }

    fn handle_input(&mut self, key: KeyEvent) -> OverlayInputResult {
        match (key.code, key.modifiers) {
            (KeyCode::Esc, _) | (KeyCode::Char('c'), KeyModifiers::CONTROL) => {
                OverlayInputResult::Dismiss
            }
            (KeyCode::Up, _) | (KeyCode::BackTab, _) => {
                self.cursor_up();
                OverlayInputResult::Consumed
            }
            (KeyCode::Down, _) | (KeyCode::Tab, _) => {
                self.cursor_down();
                OverlayInputResult::Consumed
            }
            (KeyCode::Enter, _) => {
                if let Some(cmd) = self.selected_command() {
                    self.result = Some(format!("/{}", cmd));
                    OverlayInputResult::Dismiss
                } else {
                    OverlayInputResult::Consumed
                }
            }
            (KeyCode::Backspace, _) => {
                if self.query.is_empty() {
                    OverlayInputResult::Dismiss
                } else {
                    self.pop_char();
                    OverlayInputResult::Consumed
                }
            }
            (KeyCode::Char(c), m) if m == KeyModifiers::NONE || m == KeyModifiers::SHIFT => {
                self.push_char(c);
                OverlayInputResult::Consumed
            }
            _ => OverlayInputResult::Consumed,
        }
    }

    fn take_result(&mut self) -> Option<Box<dyn Any>> {
        self.result.take().map(|s| Box::new(s) as Box<dyn Any>)
    }
}

// -- Tests

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_new_palette_has_all_commands() {
        let palette = CommandPaletteState::new();
        assert!(CMD_DEFS.len() > 10, "should have 10+ commands, got {}", CMD_DEFS.len());
        assert_eq!(palette.filtered.len(), CMD_DEFS.len());
        assert_eq!(palette.cursor, 0);
        assert!(palette.query.is_empty());
    }

    #[test]
    fn test_exact_prefix_scores_highest() {
        let score_agents = fuzzy_score("agents", "/agents", "List agents", "Session").unwrap();
        let score_memory = fuzzy_score("agents", "/memory", "List agents in memory", "Session").unwrap();
        assert!(score_agents > score_memory);
    }

    // -- OverlayComponent tests

    #[test]
    fn overlay_esc_dismisses() {
        use crate::overlay_component::{OverlayComponent, OverlayInputResult};
        let mut cp = CommandPaletteState::new();
        let key = KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE);
        assert_eq!(cp.handle_input(key), OverlayInputResult::Dismiss);
        assert!(cp.take_result().is_none()); // no command selected
    }

    #[test]
    fn overlay_enter_selects_command() {
        use crate::overlay_component::{OverlayComponent, OverlayInputResult};
        let mut cp = CommandPaletteState::new();
        // First item should be a valid command
        assert!(!cp.filtered.is_empty());
        let key = KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE);
        assert_eq!(cp.handle_input(key), OverlayInputResult::Dismiss);
        let result = cp.take_result().unwrap();
        let cmd = result.downcast::<String>().unwrap();
        assert!(cmd.starts_with('/'));
    }

    #[test]
    fn overlay_typing_filters() {
        use crate::overlay_component::{OverlayComponent, OverlayInputResult};
        let mut cp = CommandPaletteState::new();
        let initial_count = cp.filtered.len();
        let key = KeyEvent::new(KeyCode::Char('h'), KeyModifiers::NONE);
        assert_eq!(cp.handle_input(key), OverlayInputResult::Consumed);
        assert_eq!(cp.query, "h");
        // Filtering should reduce (or at least not increase) matches
        assert!(cp.filtered.len() <= initial_count);
    }

    #[test]
    fn overlay_backspace_on_empty_dismisses() {
        use crate::overlay_component::{OverlayComponent, OverlayInputResult};
        let mut cp = CommandPaletteState::new();
        let key = KeyEvent::new(KeyCode::Backspace, KeyModifiers::NONE);
        assert_eq!(cp.handle_input(key), OverlayInputResult::Dismiss);
    }

    #[test]
    fn overlay_id_is_command_palette() {
        use crate::overlay_component::OverlayComponent;
        let cp = CommandPaletteState::new();
        assert_eq!(cp.id(), "command_palette");
    }
}
