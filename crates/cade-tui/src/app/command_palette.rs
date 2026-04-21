//! Command palette — Ctrl+P fuzzy command launcher.
//!
//! Provides a VS-Code-style command palette overlay that lets users fuzzy-search
//! and execute any slash command without memorising the exact name.

use cade_core::resources::palette::{fuzzy_score, CMD_DEFS};

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
}
