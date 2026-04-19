//! Command palette — Ctrl+P fuzzy command launcher.
//!
//! Provides a VS-Code-style command palette overlay that lets users fuzzy-search
//! and execute any slash command without memorising the exact name.

use std::borrow::Cow;

// -- Command entry

/// A single command in the palette registry.
#[derive(Debug, Clone)]
pub struct PaletteCommand {
    /// Display label (e.g. "/agents").
    pub label: Cow<'static, str>,
    /// Short description shown to the right.
    pub description: Cow<'static, str>,
    /// Section/category for grouping (e.g. "Session", "Model & Mode").
    pub section: Cow<'static, str>,
}

// -- State

/// Active command palette state. `None` when the palette is closed.
#[derive(Debug, Clone)]
pub struct CommandPaletteState {
    /// User-typed search query.
    pub query: String,
    /// All available commands (populated once on open).
    pub commands: Vec<PaletteCommand>,
    /// Indices into `commands` that match the current query, ordered by score.
    pub filtered: Vec<usize>,
    /// Currently highlighted entry in `filtered`.
    pub cursor: usize,
}

impl CommandPaletteState {
    /// Create a new palette with all commands and an empty query.
    pub fn new() -> Self {
        let commands = build_command_registry();
        let filtered: Vec<usize> = (0..commands.len()).collect();
        Self {
            query: String::new(),
            commands,
            filtered,
            cursor: 0,
        }
    }

    /// Update the filtered list based on the current query.
    pub fn update_filter(&mut self) {
        if self.query.is_empty() {
            self.filtered = (0..self.commands.len()).collect();
        } else {
            let q = self.query.to_lowercase();
            let mut scored: Vec<(usize, i32)> = self
                .commands
                .iter()
                .enumerate()
                .filter_map(|(i, cmd)| {
                    fuzzy_score(&q, &cmd.label, &cmd.description, &cmd.section)
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
    pub fn selected_command(&self) -> Option<&str> {
        self.filtered
            .get(self.cursor)
            .and_then(|&idx| self.commands.get(idx))
            .map(|cmd| cmd.label.as_ref())
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

// -- Fuzzy scoring

/// Compute a fuzzy match score for a command against the query.
/// Returns `None` if the command doesn't match at all.
///
/// Scoring priorities:
/// - Exact prefix match on label: +100
/// - Substring match on label: +50
/// - Word-boundary match on label: +30
/// - Substring match on description: +20
/// - Substring match on section: +10
/// - Consecutive character match bonus: +5 per consecutive char
pub fn fuzzy_score(query: &str, label: &str, description: &str, section: &str) -> Option<i32> {
    if query.is_empty() {
        return Some(0);
    }

    let label_lower = label.to_lowercase();
    let desc_lower = description.to_lowercase();
    let section_lower = section.to_lowercase();

    let mut score: i32 = 0;
    let mut matched = false;

    // Exact prefix match on label (highest priority)
    if label_lower.starts_with(query) {
        score += 100;
        matched = true;
    }
    // Prefix match after '/' (e.g. query "ag" matches "/agents")
    else if label_lower.starts_with('/') && label_lower[1..].starts_with(query) {
        score += 95;
        matched = true;
    }
    // Substring match on label
    else if label_lower.contains(query) {
        score += 50;
        matched = true;
    }
    // Subsequence match on label
    else if is_subsequence(query, &label_lower) {
        score += 30;
        // Bonus for word-boundary alignment
        score += word_boundary_bonus(query, &label_lower);
        matched = true;
    }

    // Substring match on description (additive)
    if desc_lower.contains(query) {
        score += 20;
        matched = true;
    }

    // Substring match on section
    if section_lower.contains(query) {
        score += 10;
        matched = true;
    }

    if matched {
        // Tie-breaker: shorter labels score higher (more specific)
        score += (50i32).saturating_sub(label.len() as i32);
        Some(score)
    } else {
        None
    }
}

/// Check if `needle` is a subsequence of `haystack`.
fn is_subsequence(needle: &str, haystack: &str) -> bool {
    let mut it = haystack.chars();
    for nc in needle.chars() {
        loop {
            match it.next() {
                Some(hc) if hc == nc => break,
                Some(_) => continue,
                None => return false,
            }
        }
    }
    true
}

/// Bonus points for characters that match at word boundaries (after '-', '_', ' ').
fn word_boundary_bonus(query: &str, label: &str) -> i32 {
    let boundaries: Vec<usize> = std::iter::once(0)
        .chain(
            label
                .char_indices()
                .filter(|(_, c)| *c == '-' || *c == '_' || *c == ' ' || *c == '/')
                .map(|(i, _)| i + 1),
        )
        .collect();

    let mut bonus = 0i32;
    let mut qi = 0;
    let query_chars: Vec<char> = query.chars().collect();

    for &bi in &boundaries {
        if qi >= query_chars.len() {
            break;
        }
        if let Some(lc) = label.chars().nth(bi)
            && lc == query_chars[qi] {
                bonus += 5;
                qi += 1;
            }
    }
    bonus
}

// -- Command registry

/// Build the full list of palette commands.
/// This mirrors the SECTIONS catalogue from `menu.rs` but as a flat, owned list.
pub fn build_command_registry() -> Vec<PaletteCommand> {
    let mut cmds = Vec::with_capacity(80);

    // -- Session
    let section = "Session";
    for (cmd, desc) in [
        ("/info", "Agent, model, mode, cwd"),
        ("/agent", "Show current agent name and ID"),
        ("/agents", "List + switch agents"),
        ("/new-agent", "Create a brand-new agent"),
        ("/rename", "Rename current agent"),
        ("/delete", "Delete an agent by name/id"),
        ("/pin", "Pin current agent to settings"),
        ("/new", "Start a fresh conversation"),
        ("/resume", "Browse past conversations"),
        ("/checkpoint", "Save a checkpoint"),
        ("/tree", "Browse and restore checkpoints"),
        ("/fork", "Create a new conversation from a checkpoint"),
        ("/artifacts", "List stored artifacts"),
    ] {
        cmds.push(PaletteCommand {
            label: Cow::Borrowed(cmd),
            description: Cow::Borrowed(desc),
            section: Cow::Borrowed(section),
        });
    }

    // -- Model & Mode
    let section = "Model & Mode";
    for (cmd, desc) in [
        ("/theme", "Change colorscheme"),
        ("/model", "Interactive model picker"),
        ("/compaction-model", "Set cheaper model for summarization"),
        ("/reasoning", "Set reasoning effort"),
        ("/toolset", "Switch toolset (default/codex/gemini)"),
        ("/mode", "Show or set permission mode"),
        ("/plan", "Switch to read-only plan mode"),
        ("/todo", "Display agent scratchpad"),
        ("/todos", "Toggle live plan panel"),
        ("/default", "Return to default permission mode"),
        ("/yolo", "Bypass permissions (auto-approve all)"),
        ("/approve-always", "Add an allow rule"),
        ("/deny-always", "Add a deny rule"),
        ("/permissions", "Show permission mode + rules"),
    ] {
        cmds.push(PaletteCommand {
            label: Cow::Borrowed(cmd),
            description: Cow::Borrowed(desc),
            section: Cow::Borrowed(section),
        });
    }

    // -- Memory
    let section = "Memory";
    for (cmd, desc) in [
        ("/memory", "List all memory blocks"),
        ("/memory view", "Show full memory block"),
        ("/memory set", "Set a memory block value"),
        ("/memory edit", "Interactive edit of a memory block"),
        ("/memory delete", "Delete a memory block"),
        ("/memory history", "Last 5 revisions of a memory block"),
        ("/init", "Analyse project + populate memory"),
        ("/remember", "Ask agent to update memory"),
    ] {
        cmds.push(PaletteCommand {
            label: Cow::Borrowed(cmd),
            description: Cow::Borrowed(desc),
            section: Cow::Borrowed(section),
        });
    }

    // -- Tools & Providers
    let section = "Tools & Providers";
    for (cmd, desc) in [
        ("/backend", "Show or switch backend"),
        ("/link", "Register + attach all tools"),
        ("/unlink", "Detach all tools"),
        ("/mcp", "Show MCP server status + tools"),
        ("/connect", "Connect a new AI provider"),
        ("/disconnect", "Remove a provider"),
        ("/providers", "List configured providers"),
    ] {
        cmds.push(PaletteCommand {
            label: Cow::Borrowed(cmd),
            description: Cow::Borrowed(desc),
            section: Cow::Borrowed(section),
        });
    }

    // -- Skills
    let section = "Skills";
    for (cmd, desc) in [
        ("/skills", "List available skills"),
        ("/skills show", "Show skill detail"),
        ("/skills reload", "Reload skills from disk"),
        ("/subagents", "List available subagent definitions"),
    ] {
        cmds.push(PaletteCommand {
            label: Cow::Borrowed(cmd),
            description: Cow::Borrowed(desc),
            section: Cow::Borrowed(section),
        });
    }

    // -- Diagnostics
    let section = "Diagnostics";
    for (cmd, desc) in [
        ("/search", "Search message history"),
        ("/context", "Show context window usage"),
        ("/usage", "Token usage this session"),
        ("/cost", "Estimate API costs"),
        ("/stats", "Full session stats"),
        ("/stats model", "Per-model detail"),
        ("/stream", "Toggle streaming mode"),
        ("/hooks", "Show configured hooks"),
        ("/feedback", "Report issues / give feedback"),
    ] {
        cmds.push(PaletteCommand {
            label: Cow::Borrowed(cmd),
            description: Cow::Borrowed(desc),
            section: Cow::Borrowed(section),
        });
    }

    // -- Misc
    let section = "Misc";
    for (cmd, desc) in [
        ("/copy", "Toggle copy mode"),
        ("/export", "Export agent to JSON"),
        ("/clear", "Clear screen + context window"),
        ("/logout", "Clear stored API key and exit"),
        ("/help", "Show command menu"),
        ("/menu", "Show full command menu"),
    ] {
        cmds.push(PaletteCommand {
            label: Cow::Borrowed(cmd),
            description: Cow::Borrowed(desc),
            section: Cow::Borrowed(section),
        });
    }

    cmds
}

// -- Tests

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_new_palette_has_all_commands() {
        let palette = CommandPaletteState::new();
        assert!(palette.commands.len() > 50, "should have 50+ commands, got {}", palette.commands.len());
        assert_eq!(palette.filtered.len(), palette.commands.len());
        assert_eq!(palette.cursor, 0);
        assert!(palette.query.is_empty());
    }

    #[test]
    fn test_exact_prefix_scores_highest() {
        let score_agents = fuzzy_score("agents", "/agents", "List agents", "Session").unwrap();
        let score_memory = fuzzy_score("agents", "/memory", "List agents in memory", "Session").unwrap();
        assert!(score_agents > score_memory, "exact prefix {} should beat desc match {}", score_agents, score_memory);
    }

    #[test]
    fn test_slash_prefix_match() {
        let score = fuzzy_score("/ag", "/agents", "List agents", "Session");
        assert!(score.is_some());
        assert!(score.unwrap() > 80, "slash-prefix should score high");
    }

    #[test]
    fn test_no_match_returns_none() {
        let score = fuzzy_score("zzzzz", "/agents", "List agents", "Session");
        assert!(score.is_none());
    }

    #[test]
    fn test_empty_query_matches_all() {
        let score = fuzzy_score("", "/agents", "List agents", "Session");
        assert_eq!(score, Some(0));
    }

    #[test]
    fn test_filter_narrows_results() {
        let mut palette = CommandPaletteState::new();
        let all_count = palette.filtered.len();

        palette.push_char('m');
        palette.push_char('o');
        palette.push_char('d');
        palette.push_char('e');
        palette.push_char('l');

        assert!(palette.filtered.len() < all_count, "filtering should narrow results");
        // /model should be in the results
        let has_model = palette.filtered.iter().any(|&i| palette.commands[i].label == "/model");
        assert!(has_model, "/model should match 'model'");
    }

    #[test]
    fn test_cursor_bounds() {
        let mut palette = CommandPaletteState::new();
        palette.cursor_up();
        assert_eq!(palette.cursor, 0, "cursor_up at 0 should stay 0");

        let max = palette.filtered.len().saturating_sub(1);
        palette.cursor = max;
        palette.cursor_down();
        assert_eq!(palette.cursor, max, "cursor_down at end should stay at end");
    }

    #[test]
    fn test_selected_command() {
        let palette = CommandPaletteState::new();
        let selected = palette.selected_command();
        assert!(selected.is_some());
        // First command in registry
        assert_eq!(selected.unwrap(), "/info");
    }

    #[test]
    fn test_pop_char_restores_filter() {
        let mut palette = CommandPaletteState::new();
        let full = palette.filtered.len();

        palette.push_char('x');
        palette.push_char('z');
        let narrow = palette.filtered.len();
        assert!(narrow < full);

        palette.pop_char();
        palette.pop_char();
        assert_eq!(palette.filtered.len(), full, "popping all chars should restore full list");
    }

    #[test]
    fn test_subsequence_match() {
        // "mem" should match "/memory" via subsequence
        assert!(is_subsequence("mem", "/memory"));
        assert!(is_subsequence("ag", "/agents"));
        assert!(!is_subsequence("zz", "/agents"));
    }

    #[test]
    fn test_description_match_works() {
        // Query "token" should match /usage which has "Token usage this session"
        let score = fuzzy_score("token", "/usage", "Token usage this session", "Diagnostics");
        assert!(score.is_some(), "description substring should match");
    }
}
