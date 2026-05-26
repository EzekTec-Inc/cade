//! Pluggable autocomplete providers for the CADE TUI input field.
//!
//! Two providers ship built-in:
//!
//! - [`FileAutocompleteProvider`] — Tab path completion and `@` fuzzy file picker.
//! - [`SlashCommandProvider`] — `/` prefix command listing for the select-list overlay.
//!
//! Both are consumed by `TuiApp::handle_key_input` via the [`AutocompleteProvider`]
//! trait, decoupling the filesystem walk and command catalogue from the UI rendering.

use std::path::{Path, PathBuf};
use crate::colors::ThemeColorsExt;

// -- Trait

/// A completion result produced by an [`AutocompleteProvider`].
#[derive(Debug, Clone)]
pub struct Completion {
    /// The text to insert at the cursor position.
    pub text: String,
    /// Optional one-line description shown beside the entry.
    pub description: Option<String>,
}

/// Trait for providing autocomplete suggestions.
pub trait AutocompleteProvider {
    /// Return completions for the current word/context.
    ///
    /// * `input`  — full input buffer text
    /// * `cursor` — byte-offset cursor position within `input`
    fn completions(&self, input: &str, cursor: usize) -> Vec<Completion>;
}

// -- FileAutocompleteProvider

/// Provides Tab path completion and `@` fuzzy file browsing.
#[derive(Debug, Clone)]
pub struct FileAutocompleteProvider {
    root: PathBuf,
}

impl FileAutocompleteProvider {
    pub fn new(root: PathBuf) -> Self {
        Self { root }
    }

    /// Update the root directory (e.g. when the user changes cwd).
    pub fn set_root(&mut self, root: PathBuf) {
        self.root = root;
    }

    // -- Tab path completion

    /// Attempt Tab path completion on the token at `cursor`.
    /// Returns `Some((new_input, new_cursor))` on success, or `None` if
    /// the token at the cursor doesn't look like a path.
    pub fn complete_path(&self, input: &str, cursor: usize) -> Option<(String, usize)> {
        let cursor = cursor.min(input.len());
        let before = &input[..cursor];

        let word_start = before
            .rfind(|c: char| c.is_whitespace())
            .map(|i| i + 1)
            .unwrap_or(0);
        let partial = &before[word_start..];

        if !partial.starts_with('/')
            && !partial.starts_with("./")
            && !partial.starts_with("~/")
            && !partial.contains('/')
        {
            return None;
        }

        let home = dirs::home_dir();
        let expanded: PathBuf = if let Some(stripped) = partial.strip_prefix("~/") {
            let h = home.as_deref()?;
            h.join(stripped)
        } else {
            PathBuf::from(partial)
        };

        let (parent, file_prefix, dir_suffix) = if partial.ends_with('/') {
            (expanded.clone(), "", true)
        } else {
            let p = expanded.parent().unwrap_or(Path::new(".")).to_path_buf();
            let f = expanded.file_name().and_then(|n| n.to_str()).unwrap_or("");
            (p, f, false)
        };

        let mut matches: Vec<(String, bool)> = std::fs::read_dir(&parent)
            .ok()?
            .filter_map(|e| e.ok())
            .filter_map(|e| {
                let name = e.file_name().to_string_lossy().to_string();
                if name.starts_with(file_prefix) {
                    let is_dir = e.path().is_dir();
                    Some((name, is_dir))
                } else {
                    None
                }
            })
            .collect();

        if matches.is_empty() {
            return None;
        }
        matches.sort_by(|a, b| a.0.cmp(&b.0));

        let names: Vec<String> = matches.iter().map(|(n, _)| n.clone()).collect();
        let prefix_str = common_prefix(&names);
        let suffix = if matches.len() == 1 && matches[0].1 {
            "/"
        } else {
            ""
        };
        let completed_name = format!("{prefix_str}{suffix}");

        let parent_display: String = {
            let parent_str = parent.to_string_lossy();
            if let Some(h) = &home {
                if parent.starts_with(h) {
                    let rel = parent
                        .strip_prefix(h)
                        .ok()
                        .and_then(|p| p.to_str())
                        .unwrap_or("");
                    if rel.is_empty() {
                        "~/".to_string()
                    } else {
                        format!("~/{rel}/")
                    }
                } else if dir_suffix {
                    format!("{}/", parent_str)
                } else {
                    format!("{}/", parent_str)
                }
            } else if dir_suffix {
                format!("{}/", parent_str)
            } else {
                format!("{}/", parent_str)
            }
        };

        let new_token = if dir_suffix || partial.ends_with('/') {
            format!("{}{}", parent_display, completed_name)
        } else {
            format!("{}{}", parent_display, completed_name)
        };

        let new_cursor = word_start + new_token.len();
        let new_input = format!("{}{}{}", &input[..word_start], new_token, &input[cursor..]);
        Some((new_input, new_cursor))
    }

    // -- @ fuzzy file listing

    /// Collect project files matching `query` for the `@` picker.
    /// Walks up to 3 directories deep, skips hidden / `target` / `node_modules`,
    /// returns at most 50 sorted results.
    pub fn collect_files(&self, query: &str) -> Vec<String> {
        let mut out: Vec<String> = Vec::new();
        collect_files_inner(&self.root, &self.root, 0, 3, query, &mut out);
        out.sort();
        out.truncate(50);
        out
    }
}

impl AutocompleteProvider for FileAutocompleteProvider {
    fn completions(&self, input: &str, cursor: usize) -> Vec<Completion> {
        // Extract the token at cursor and list matching files.
        let cursor = cursor.min(input.len());
        let before = &input[..cursor];
        let word_start = before
            .rfind(|c: char| c.is_whitespace())
            .map(|i| i + 1)
            .unwrap_or(0);
        let partial = &before[word_start..];

        if partial.is_empty() {
            return vec![];
        }

        let files = self.collect_files(partial);
        files
            .into_iter()
            .map(|f| Completion {
                text: f,
                description: None,
            })
            .collect()
    }
}

// -- SlashCommandProvider

/// A registered slash command.
#[derive(Debug, Clone)]
pub struct SlashCommandDef {
    /// Command name (without the `/` prefix).
    pub name: String,
    /// Short description shown in the autocomplete list.
    pub description: String,
}

/// Provides `/` prefix slash-command completions.
pub struct SlashCommandProvider {
    commands: Vec<SlashCommandDef>,
}

// -- AgentModelAutocompleteProvider

/// Provides `@` prefix completion for agents and `#` prefix completion for models.
pub struct AgentModelAutocompleteProvider {
    pub agents: Vec<String>,
    pub models: Vec<String>,
}

impl AgentModelAutocompleteProvider {
    pub fn new(agents: Vec<String>, models: Vec<String>) -> Self {
        Self { agents, models }
    }

    pub fn set_agents(&mut self, agents: Vec<String>) {
        self.agents = agents;
    }

    pub fn set_models(&mut self, models: Vec<String>) {
        self.models = models;
    }

    /// Attempt agent/model completion on the token at `cursor`.
    /// Returns `Some((new_input, new_cursor))` on success.
    pub fn complete_token(&self, input: &str, cursor: usize) -> Option<(String, usize)> {
        let cursor = cursor.min(input.len());
        let before = &input[..cursor];

        let word_start = before
            .rfind(|c: char| c.is_whitespace())
            .map(|i| i + 1)
            .unwrap_or(0);
        let partial = &before[word_start..];

        if partial.starts_with('@') && partial.len() > 0 {
            let prefix = &partial[1..].to_lowercase();
            let mut matches: Vec<String> = self
                .agents
                .iter()
                .filter(|a| a.to_lowercase().starts_with(prefix))
                .cloned()
                .collect();

            if matches.is_empty() {
                return None;
            }
            matches.sort();

            let common = common_prefix(&matches);
            let new_token = format!("@{}", common);
            let new_cursor = word_start + new_token.len();
            let new_input = format!("{}{}{}", &input[..word_start], new_token, &input[cursor..]);
            return Some((new_input, new_cursor));
        }

        if partial.starts_with('#') && partial.len() > 0 {
            let prefix = &partial[1..].to_lowercase();
            let mut matches: Vec<String> = self
                .models
                .iter()
                .filter(|m| m.to_lowercase().starts_with(prefix))
                .cloned()
                .collect();

            if matches.is_empty() {
                return None;
            }
            matches.sort();

            let common = common_prefix(&matches);
            let new_token = format!("#{}", common);
            let new_cursor = word_start + new_token.len();
            let new_input = format!("{}{}{}", &input[..word_start], new_token, &input[cursor..]);
            return Some((new_input, new_cursor));
        }

        None
    }
}

impl AutocompleteProvider for AgentModelAutocompleteProvider {
    fn completions(&self, input: &str, cursor: usize) -> Vec<Completion> {
        let cursor = cursor.min(input.len());
        let before = &input[..cursor];
        let word_start = before
            .rfind(|c: char| c.is_whitespace())
            .map(|i| i + 1)
            .unwrap_or(0);
        let partial = &before[word_start..];

        if partial.starts_with('@') {
            let prefix = &partial[1..].to_lowercase();
            return self
                .agents
                .iter()
                .filter(|a| a.to_lowercase().starts_with(prefix))
                .map(|a| Completion {
                    text: format!("@{}", a),
                    description: Some("Agent".to_string()),
                })
                .collect();
        }

        if partial.starts_with('#') {
            let prefix = &partial[1..].to_lowercase();
            return self
                .models
                .iter()
                .filter(|m| m.to_lowercase().starts_with(prefix))
                .map(|m| Completion {
                    text: format!("#{}", m),
                    description: Some("Model".to_string()),
                })
                .collect();
        }

        vec![]
    }
}

impl SlashCommandProvider {
    pub fn new(commands: Vec<SlashCommandDef>) -> Self {
        Self { commands }
    }

    /// Update the command list (e.g. when skills change).
    pub fn set_commands(&mut self, commands: Vec<SlashCommandDef>) {
        self.commands = commands;
    }

    /// Filter commands matching a prefix (case-insensitive).
    pub fn matching(&self, prefix: &str) -> Vec<&SlashCommandDef> {
        let lower = prefix.to_lowercase();
        self.commands
            .iter()
            .filter(|c| c.name.to_lowercase().starts_with(&lower))
            .collect()
    }
}

impl AutocompleteProvider for SlashCommandProvider {
    fn completions(&self, input: &str, _cursor: usize) -> Vec<Completion> {
        let trimmed = input.trim();
        if !trimmed.starts_with('/') {
            return vec![];
        }
        let prefix = &trimmed[1..]; // strip the /
        self.matching(prefix)
            .into_iter()
            .map(|cmd| Completion {
                text: format!("/{}", cmd.name),
                description: Some(cmd.description.clone()),
            })
            .collect()
    }
}

// -- Internal helpers

fn collect_files_inner(
    root: &Path,
    dir: &Path,
    depth: u32,
    max_depth: u32,
    query: &str,
    out: &mut Vec<String>,
) {
    if depth > max_depth {
        return;
    }
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.filter_map(|e| e.ok()) {
        let name = entry.file_name().to_string_lossy().to_string();
        if name.starts_with('.') {
            continue;
        }
        if matches!(name.as_str(), "target" | "node_modules" | ".git") {
            continue;
        }
        let path = entry.path();
        if path.is_dir() {
            collect_files_inner(root, &path, depth + 1, max_depth, query, out);
        } else if query.is_empty() || name.to_lowercase().contains(&query.to_lowercase()) {
            let rel = path
                .strip_prefix(root)
                .ok()
                .and_then(|p| p.to_str())
                .map(|s| s.to_string())
                .unwrap_or(name);
            out.push(rel);
        }
    }
}

/// Longest common prefix of a non-empty slice of strings.
pub fn common_prefix(words: &[String]) -> String {
    if words.is_empty() {
        return String::new();
    }
    let first = &words[0];
    let len = words
        .iter()
        .skip(1)
        .map(|w| {
            first
                .chars()
                .zip(w.chars())
                .take_while(|(a, b)| a == b)
                .count()
        })
        .min()
        .unwrap_or(first.chars().count());
    first.chars().take(len).collect()
}

// -- ToolAutocompleteProvider

/// Provides completion for connected MCP servers and canonical tool names.
#[derive(Debug, Clone, Default)]
pub struct ToolAutocompleteProvider {
    pub mcp_servers: Vec<String>,
    pub tools: Vec<String>,
}

impl ToolAutocompleteProvider {
    pub fn new(mcp_servers: Vec<String>, tools: Vec<String>) -> Self {
        Self { mcp_servers, tools }
    }

    pub fn set_mcp_servers(&mut self, servers: Vec<String>) {
        self.mcp_servers = servers;
    }

    pub fn set_tools(&mut self, tools: Vec<String>) {
        self.tools = tools;
    }
}

impl AutocompleteProvider for ToolAutocompleteProvider {
    fn completions(&self, input: &str, cursor: usize) -> Vec<Completion> {
        let cursor = cursor.min(input.len());
        let before = &input[..cursor];
        let word_start = before
            .rfind(|c: char| c.is_whitespace())
            .map(|i| i + 1)
            .unwrap_or(0);
        let partial = &before[word_start..];

        if !partial.starts_with(':') {
            return vec![];
        }

        let prefix = &partial[1..].to_lowercase();
        let mut results = Vec::new();

        // 1. Match MCP servers
        for s in &self.mcp_servers {
            if s.to_lowercase().starts_with(prefix) {
                results.push(Completion {
                    text: format!(":{}:", s),
                    description: Some("MCP Server".to_string()),
                });
            }
        }

        // 2. Match tool names
        for t in &self.tools {
            if t.to_lowercase().starts_with(prefix) {
                results.push(Completion {
                    text: format!(":{}", t),
                    description: Some("Tool".to_string()),
                });
            }
        }

        results
    }
}

// -- NextStepAutocompleteProvider

/// Provides helpful completion suggestions based on the last completed task or active plan.
#[derive(Debug, Clone, Default)]
pub struct NextStepAutocompleteProvider {
    pub next_steps: Vec<String>,
}

impl NextStepAutocompleteProvider {
    pub fn new(next_steps: Vec<String>) -> Self {
        Self { next_steps }
    }

    pub fn set_next_steps(&mut self, next_steps: Vec<String>) {
        self.next_steps = next_steps;
    }
}

impl AutocompleteProvider for NextStepAutocompleteProvider {
    fn completions(&self, input: &str, cursor: usize) -> Vec<Completion> {
        let cursor = cursor.min(input.len());
        let before = &input[..cursor];
        let word_start = before
            .rfind(|c: char| c.is_whitespace())
            .map(|i| i + 1)
            .unwrap_or(0);
        let partial = &before[word_start..];

        if !partial.starts_with('?') {
            return vec![];
        }

        let prefix = &partial[1..].to_lowercase();
        let mut results = Vec::new();

        for ns in &self.next_steps {
            if ns.to_lowercase().contains(prefix) {
                results.push(Completion {
                    text: ns.clone(),
                    description: Some("Next Step Hint".to_string()),
                });
            }
        }

        results
    }
}

// -- AutocompleteOverlay

pub struct AutocompleteAction {
    pub text: String,
    pub word_start: usize,
    pub cursor_pos: usize,
}

pub struct AutocompleteOverlay {
    pub id: &'static str,
    pub suggestions: Vec<Completion>,
    pub selected_idx: usize,
    pub dismissed: bool,
    pub result: Option<String>,
    pub word_start: usize,
    pub cursor_pos: usize,
}

impl AutocompleteOverlay {
    pub fn new(suggestions: Vec<Completion>, word_start: usize, cursor_pos: usize) -> Self {
        Self {
            id: "autocomplete_suggestions",
            suggestions,
            selected_idx: 0,
            dismissed: false,
            result: None,
            word_start,
            cursor_pos,
        }
    }
}

impl crate::overlay_component::OverlayComponent for AutocompleteOverlay {
    fn id(&self) -> &'static str {
        self.id
    }

    fn render_overlay(&mut self, frame: &mut ratatui::Frame, area: ratatui::layout::Rect, colors: &crate::colors::ThemeColors) {
        use ratatui::widgets::{Block, Borders, List, ListItem, ListState, Clear};
        use ratatui::layout::Rect;
        use ratatui::style::Style;
        use ratatui::text::{Line, Span};

        if self.suggestions.is_empty() {
            return;
        }

        let card_h = (self.suggestions.len() as u16 + 2).min(8).min(area.height.saturating_sub(4));
        let card_w = 65u16.min(area.width.saturating_sub(4));
        
        let card_area = Rect::new(
            area.x + 2,
            area.y + area.height.saturating_sub(card_h).saturating_sub(2),
            card_w,
            card_h,
        );

        frame.render_widget(Clear, card_area);

        let block = Block::default()
            .title(Span::styled(" Suggestions (Enter/Tab to select) ", Style::default().bold().fg(colors.c_primary())))
            .borders(Borders::ALL)
            .border_style(Style::default().fg(colors.c_text_muted()));

        let mut items = Vec::new();
        for (idx, comp) in self.suggestions.iter().enumerate() {
            let text_style = if idx == self.selected_idx {
                Style::default().fg(colors.c_primary()).bold()
            } else {
                Style::default().fg(colors.c_text_primary())
            };

            let mut spans = vec![Span::styled(&comp.text, text_style)];

            if let Some(ref desc) = comp.description {
                spans.push(Span::raw("  ·  "));
                spans.push(Span::styled(desc, Style::default().fg(colors.c_text_muted()).dim()));
            }

            items.push(ListItem::new(Line::from(spans)));
        }

        let mut state = ListState::default();
        state.select(Some(self.selected_idx));

        let list = List::new(items)
            .block(block)
            .highlight_style(Style::default().bg(colors.c_bg_surface1()));

        frame.render_stateful_widget(list, card_area, &mut state);
    }

    fn handle_input(&mut self, key: crossterm::event::KeyEvent) -> crate::overlay_component::OverlayInputResult {
        use crossterm::event::KeyCode;
        use crate::overlay_component::OverlayInputResult;

        match key.code {
            KeyCode::Up => {
                if self.selected_idx > 0 {
                    self.selected_idx -= 1;
                } else {
                    self.selected_idx = self.suggestions.len().saturating_sub(1);
                }
                OverlayInputResult::Consumed
            }
            KeyCode::Down => {
                if self.selected_idx + 1 < self.suggestions.len() {
                    self.selected_idx += 1;
                } else {
                    self.selected_idx = 0;
                }
                OverlayInputResult::Consumed
            }
            KeyCode::Enter | KeyCode::Tab => {
                if let Some(comp) = self.suggestions.get(self.selected_idx) {
                    self.result = Some(comp.text.clone());
                }
                self.dismissed = true;
                OverlayInputResult::Dismiss
            }
            KeyCode::Esc => {
                self.dismissed = true;
                OverlayInputResult::Dismiss
            }
            _ => OverlayInputResult::NotHandled,
        }
    }

    fn is_dismissed(&self) -> bool {
        self.dismissed
    }

    fn take_result(&mut self) -> Option<Box<dyn std::any::Any>> {
        self.result.take().map(|r| {
            Box::new(AutocompleteAction {
                text: r,
                word_start: self.word_start,
                cursor_pos: self.cursor_pos,
            }) as Box<dyn std::any::Any>
        })
    }
}
