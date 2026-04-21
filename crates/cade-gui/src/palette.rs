//! Slash-command palette for the cade-gui dashboard.
//!
//! This module is **pure Rust** — no browser dependencies.  It contains:
//!   - [`PaletteCmd`] — every command the palette can issue.
//!   - [`CMD_DEFS`] — static table of (trigger, description, category).
//!   - [`parse_palette_input`] — converts raw text into a `PaletteCmd`.
//!   - [`fuzzy_filter`] — ranks entries by how well they match a query.
//!
//! The wasm-side render loop (`app.rs`) opens the palette on `Ctrl+K` or
//! when the user types `/` at the start of a blank input, and dispatches
//! the resolved command.

use cade_core::resources::palette::{CmdDef, CMD_DEFS};
pub use cade_core::resources::palette::PaletteCmd;
pub use cade_core::resources::palette::parse_palette_input;

// ── Fuzzy filter ──────────────────────────────────────────────────────────

/// A filtered + scored entry for palette display.
#[derive(Debug, Clone)]
pub struct FilteredCmd<'a> {
    pub def: &'a CmdDef,
    /// Score: higher = better match. `0` means query was empty (show all).
    pub score: usize,
}

/// Filter and rank `CMD_DEFS` by how well they match `query`.
///
/// Empty query → all entries at score 0 (preserve table order).
/// Otherwise uses a simple substring / initials heuristic:
///   - Full prefix match → 100
///   - Substring match in trigger → 50
///   - Substring match in description → 20
///   - No match → excluded
pub fn fuzzy_filter<'a>(query: &str) -> Vec<FilteredCmd<'a>> {
    let q = query.trim().to_lowercase();
    if q.is_empty() {
        return CMD_DEFS
            .iter()
            .map(|d| FilteredCmd { def: d, score: 0 })
            .collect();
    }

    let mut results: Vec<FilteredCmd<'_>> = CMD_DEFS
        .iter()
        .filter_map(|def| {
            cade_core::resources::palette::fuzzy_score(&q, def.trigger, def.description, "")
                .map(|score| FilteredCmd { def, score: score as usize })
        })
        .collect();

    results.sort_by(|a, b| b.score.cmp(&a.score));
    results
}

// ── Tests ─────────────────────────────────────────────────────────────────
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn filter_preserves_order_on_empty() {
        let res = fuzzy_filter("");
        assert_eq!(res.len(), CMD_DEFS.len());
        assert_eq!(res[0].def.trigger, CMD_DEFS[0].trigger);
    }
}
