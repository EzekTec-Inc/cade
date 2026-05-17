pub mod clipboard;
pub mod command_palette;
pub mod input;
pub mod layout;
pub mod password;
pub mod questions;
pub mod render;
pub mod state;
pub mod timeline;
pub(crate) use timeline::*;

pub fn strip_orchestrator_prompts(text: &str) -> std::borrow::Cow<'_, str> {
    use regex::Regex;
    use std::sync::OnceLock;
    static RE: OnceLock<Regex> = OnceLock::new();
    let re = RE.get_or_init(|| {
        Regex::new(r"(?is)[\w\d]*>thought\s*CRITICAL INSTRUCTION 1:.*?CRITICAL INSTRUCTION 2:.*?(?:task at hand\.)(?:[^\n]*?task at hand\.)?\s*").unwrap()
    });
    re.replace_all(text, "")
}

use parking_lot::Mutex;
use std::io::Write;
use std::sync::Arc;
use std::time::Instant;

use crate::Result;

use crossterm::event::{
    DisableBracketedPaste, DisableFocusChange, EnableBracketedPaste, EnableFocusChange,
    KeyboardEnhancementFlags, PopKeyboardEnhancementFlags, PushKeyboardEnhancementFlags,
};
use ratatui::{
    DefaultTerminal, Frame,
    layout::Rect,
    style::{Color as RC, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Padding, Paragraph, Wrap},
};

use crate::autocomplete::FileAutocompleteProvider;
use crate::colors::ThemeColors;
use crate::editor::{Editor, ImageEntry};
// Re-export for child modules that `use super::*`
pub(crate) use crate::editor::InputMode;
use cade_core::permissions::PermissionMode;

use layout::helpers::{abbreviate_cwd, display_tool_name};
pub use layout::helpers::{cycle_mode, cycle_mode_back, truncate_str};
use render::{count_wrapped_rows, render_frame};

// -- Constants

/// Fixed non-input rows at the bottom: status + top_sep + bot_sep + footer.
const FIXED_ROWS: u16 = 4;
/// Maximum rows the input area may grow to.
const MAX_INPUT_ROWS: u16 = 6;
/// Vertical padding (rows) inside the scrollable content area.
const CONTENT_PAD_TOP: u16 = 1;
const CONTENT_PAD_BOT: u16 = 1;
/// Braille spinner frames for thinking animation.
const BRAILLE: &[&str] = &["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏"];
const DOTS: &[&str] = &["⠁", "⠂", "⠄", "⠐", "⠠", "⠐", "⠄", "⠂"];

/// Responsive layout breakpoint for showing the right sidebar.
const SIDEBAR_BREAKPOINT: u16 = 110;
/// Target width for the informational sidebar on wide terminals.
const SIDEBAR_WIDTH: u16 = 40;

// -- Skills overlay

// -- RenderLine

/// One logical unit of committed content in the conversation view.
#[derive(Clone, Debug)]
pub enum RenderLine {
    /// Full-width dim separator (between user turns).
    Separator,
    /// User message with `> ` prefix and preceding separator.
    UserMessage(String),
    /// Complete (committed) assistant response block.
    AssistantText(String),
    /// Tool call header: `● Name(args…)`.
    ToolCall { name: String, preview: String },
    /// Tool result: `  ⎿  summary`.
    ToolResult { is_error: bool, content: String },
    /// Collapsed reasoning block. Expandable via Ctrl+O.
    Reasoning { words: usize, content: String },
    /// System / info message (dim gray).
    SystemMsg(String),
    /// Success message (green, ✓ prefix).
    SuccessMsg(String),
    /// Section header (cyan bold — e.g. "  MCP Servers").
    InfoHeader(String),
    /// Dim hint / secondary text (dark gray italic).
    DimMsg(String),
    /// Key-value pair aligned with padding between them.
    Pair { label: String, value: String },
    /// Error message (red).
    ErrorMsg(String),
    /// Structured table data.
    Table {
        headers: Vec<String>,
        rows: Vec<Vec<String>>,
    },
    /// Blank spacer line.
    Blank,
    /// Summary of the heuristic evaluator subagent.
    HeuristicSummary {
        intent: String,
        safety: String,
        directives: String,
    },
    /// Interactive question completed result.
    QuestionResult { header: String, answer: String },
    /// Live-streaming bash output.  Lines accumulate in real-time; only the
    /// last `max_visible` lines are shown when collapsed.  `ctrl+o` shows all.
    LiveOutput {
        lines: Vec<String>,
        max_visible: usize,
        done: bool,
    },
    /// Context-window usage bar chart (single timeline entry).
    ///
    /// Rendered as:
    ///   header line: model · total% (used/window tokens)
    ///   bar line:    proportional █▓▒░ segments per category
    ///   legend lines: one row per category with token count and %
    ///
    /// Categories (index → glyph → label):
    ///   0 system   █  System prompt
    ///   1 tools    ▓  Native tools
    ///   2 mcp      ▒  MCP tools
    ///   3 memory   ░  Memory
    ///   4 skills   ▪  Skills
    ///   5 messages ■  Messages
    ///   6 free     ·  Free
    ///   7 buffer   ⎹  Buffer (autocompact reserve)
    ContextBar {
        /// Short model name (e.g. "claude-sonnet-4-5")
        model: String,
        /// Total context window size in tokens.
        window: u64,
        /// Overall used percentage 0–100.
        pct: u8,
        /// Per-category token counts in category order (indices 0–7).
        category_tokens: Vec<u64>,
    },
}

// -- PickerState (A-01)

/// Result produced by the file picker overlay.
///
/// The host drains this via [`OverlayComponent::take_result`] and
/// acts on the variant (insert text, remove chars, update query, etc.).
#[derive(Debug, Clone)]
pub enum FilePickerAction {
    /// User selected a file — replace @query range and insert selected path.
    Select {
        /// Position of `@` in the editor buffer.
        at_pos: usize,
        /// Length of the query that follows `@`.
        query_len: usize,
        /// The selected file path to insert.
        selected: String,
    },
    /// Backspace on the query — host should remove a char from the editor.
    BackspaceChar {
        /// Position of `@` in the editor buffer.
        at_pos: usize,
        /// Current query length *before* the pop (host removes char at at_pos + 1 + query_len - 1).
        query_len_before: usize,
    },
    /// Backspace with empty query — host should delete the `@` and dismiss.
    DeleteAt { at_pos: usize },
    /// User typed a character — host should insert it into the editor.
    InsertChar {
        /// Byte offset where the char should go.
        position: usize,
        ch: char,
    },
}

/// State for the `@` file fuzzy picker overlay.
#[derive(Debug, Clone)]
pub struct PickerState {
    /// Byte position of the `@` in `app.input` that activated the picker.
    pub at_pos: usize,
    /// The query typed after `@` (grows as user types).
    pub query: String,
    /// Matching file paths (relative to CWD), filtered by `query`.
    pub matches: Vec<String>,
    /// Index of the highlighted entry.
    pub cursor: usize,
    /// Pending action to be drained by the host.
    pending_action: Option<FilePickerAction>,
    /// Autocomplete provider for file matching.
    /// Cloned from TuiApp on overlay creation.
    file_ac: crate::autocomplete::FileAutocompleteProvider,
}

impl PickerState {
    /// Create a new picker at the given `@` position.
    pub fn new(
        at_pos: usize,
        query: String,
        file_ac: &crate::autocomplete::FileAutocompleteProvider,
    ) -> Self {
        let matches = file_ac.collect_files(&query);
        Self {
            at_pos,
            query,
            matches,
            cursor: 0,
            pending_action: None,
            file_ac: file_ac.clone(),
        }
    }
}

impl crate::overlay_component::OverlayComponent for PickerState {
    fn id(&self) -> &'static str {
        "file_picker"
    }

    fn render_overlay(
        &mut self,
        frame: &mut ratatui::Frame,
        area: ratatui::layout::Rect,
        colors: &crate::colors::ThemeColors,
    ) {
        crate::app::layout::pickers::render_picker(frame, self, area, colors);
    }

    fn handle_input(
        &mut self,
        key: crossterm::event::KeyEvent,
    ) -> crate::overlay_component::OverlayInputResult {
        use crate::overlay_component::OverlayInputResult;
        use crossterm::event::{KeyCode, KeyModifiers};

        match (key.code, key.modifiers) {
            (KeyCode::Esc, _) => OverlayInputResult::Dismiss,
            (KeyCode::Up, _) => {
                self.cursor = self.cursor.saturating_sub(1);
                OverlayInputResult::Consumed
            }
            (KeyCode::Down, _) => {
                if !self.matches.is_empty() && self.cursor + 1 < self.matches.len() {
                    self.cursor += 1;
                }
                OverlayInputResult::Consumed
            }
            (KeyCode::Enter, KeyModifiers::NONE) => {
                if let Some(selected) = self.matches.get(self.cursor).cloned() {
                    self.pending_action = Some(FilePickerAction::Select {
                        at_pos: self.at_pos,
                        query_len: self.query.len(),
                        selected,
                    });
                }
                OverlayInputResult::Dismiss
            }
            (KeyCode::Backspace, _) => {
                if self.query.is_empty() {
                    self.pending_action = Some(FilePickerAction::DeleteAt {
                        at_pos: self.at_pos,
                    });
                    OverlayInputResult::Dismiss
                } else {
                    let old_len = self.query.len();
                    self.pending_action = Some(FilePickerAction::BackspaceChar {
                        at_pos: self.at_pos,
                        query_len_before: old_len,
                    });
                    self.query.pop();
                    self.cursor = 0;
                    self.matches = self.file_ac.collect_files(&self.query);
                    OverlayInputResult::Consumed
                }
            }
            (KeyCode::Char(c), m) if m == KeyModifiers::NONE || m == KeyModifiers::SHIFT => {
                let insert_pos = self.at_pos + 1 + self.query.len();
                self.pending_action = Some(FilePickerAction::InsertChar {
                    position: insert_pos,
                    ch: c,
                });
                self.query.push(c);
                self.cursor = 0;
                self.matches = self.file_ac.collect_files(&self.query);
                OverlayInputResult::Consumed
            }
            _ => OverlayInputResult::Consumed,
        }
    }

    fn take_result(&mut self) -> Option<Box<dyn std::any::Any>> {
        self.pending_action
            .take()
            .map(|a| Box::new(a) as Box<dyn std::any::Any>)
    }
}

// -- ThemePickerState

/// Result produced by the theme picker overlay.
///
/// The host calls [`OverlayComponent::take_result`] after every input
/// dispatch and downcasts to this enum to apply side effects.
#[derive(Debug, Clone)]
pub enum ThemePickerAction {
    /// Cursor moved — apply this theme as a live preview.
    Preview(crate::colors::ThemeColors),
    /// User confirmed selection — submit this as a REPL command.
    Submit(String),
    /// User cancelled — revert to this theme and show a toast.
    Revert(crate::colors::ThemeColors),
}

/// State for the `/theme` floating picker overlay.
#[derive(Debug, Clone)]
pub struct ThemePickerState {
    pub query: String,
    pub themes: Vec<cade_core::resources::themes::Theme>,
    pub filtered_indices: Vec<usize>,
    pub cursor: usize,
    /// If cancelled, restore to this
    pub original_theme: crate::colors::ThemeColors,
    /// Pending action to be drained by the host via `take_result()`.
    pending_action: Option<ThemePickerAction>,
}

impl ThemePickerState {
    /// Resolve the theme colors for the currently highlighted entry.
    fn current_preview_colors(&self) -> Option<crate::colors::ThemeColors> {
        if self.filtered_indices.is_empty() {
            return None;
        }
        let idx = self.filtered_indices[self.cursor];
        Some(self.themes[idx].clone())
    }

    /// Re-filter the theme list based on the current query and reset cursor.
    fn update_filter(&mut self) {
        self.cursor = 0;
        let query_lower = self.query.to_lowercase();
        self.filtered_indices = self
            .themes
            .iter()
            .enumerate()
            .filter(|(_, t)| {
                query_lower.is_empty()
                    || t.meta.name.to_lowercase().contains(&query_lower)
                    || t.meta
                        .description
                        .as_deref()
                        .unwrap_or("")
                        .to_lowercase()
                        .contains(&query_lower)
                    || match t.meta.variant {
                        opaline::ThemeVariant::Dark => "dark",
                        opaline::ThemeVariant::Light => "light",
                    }
                    .contains(&query_lower)
            })
            .map(|(i, _)| i)
            .collect();
        // Emit preview for new cursor position after filter.
        if let Some(colors) = self.current_preview_colors() {
            self.pending_action = Some(ThemePickerAction::Preview(colors));
        }
    }

    /// Move cursor and emit a preview action.
    fn move_cursor(&mut self, delta: isize) {
        if self.filtered_indices.is_empty() {
            return;
        }
        let new = (self.cursor as isize + delta)
            .max(0)
            .min(self.filtered_indices.len() as isize - 1) as usize;
        if new != self.cursor {
            self.cursor = new;
            if let Some(colors) = self.current_preview_colors() {
                self.pending_action = Some(ThemePickerAction::Preview(colors));
            }
        }
    }
}

impl crate::overlay_component::OverlayComponent for ThemePickerState {
    fn id(&self) -> &'static str {
        "theme_picker"
    }

    fn render_overlay(
        &mut self,
        frame: &mut ratatui::Frame,
        area: ratatui::layout::Rect,
        colors: &crate::colors::ThemeColors,
    ) {
        crate::app::layout::pickers::render_theme_picker(frame, self, area, colors);
    }

    fn handle_input(
        &mut self,
        key: crossterm::event::KeyEvent,
    ) -> crate::overlay_component::OverlayInputResult {
        use crate::overlay_component::OverlayInputResult;
        use crossterm::event::{KeyCode, KeyModifiers};

        match (key.code, key.modifiers) {
            // Cancel → revert
            (KeyCode::Esc, _) | (KeyCode::Char('c'), KeyModifiers::CONTROL) => {
                self.pending_action = Some(ThemePickerAction::Revert(self.original_theme.clone()));
                OverlayInputResult::Dismiss
            }
            // Navigate up
            (KeyCode::Up, _) | (KeyCode::BackTab, _) => {
                self.move_cursor(-1);
                OverlayInputResult::Consumed
            }
            // Navigate down
            (KeyCode::Down, _) | (KeyCode::Tab, _) => {
                self.move_cursor(1);
                OverlayInputResult::Consumed
            }
            // Confirm selection
            (KeyCode::Enter, _) => {
                if !self.filtered_indices.is_empty() {
                    let idx = self.filtered_indices[self.cursor];
                    let name = self.themes[idx].meta.name.clone();
                    self.pending_action =
                        Some(ThemePickerAction::Submit(format!("/theme {}", name)));
                    OverlayInputResult::Dismiss
                } else {
                    // Empty results — stay open
                    OverlayInputResult::Consumed
                }
            }
            // Filter: backspace
            (KeyCode::Backspace, _) => {
                self.query.pop();
                self.update_filter();
                OverlayInputResult::Consumed
            }
            // Filter: type char
            (KeyCode::Char(c), m) if m == KeyModifiers::NONE || m == KeyModifiers::SHIFT => {
                self.query.push(c);
                self.update_filter();
                OverlayInputResult::Consumed
            }
            _ => OverlayInputResult::Consumed,
        }
    }

    fn take_result(&mut self) -> Option<Box<dyn std::any::Any>> {
        self.pending_action
            .take()
            .map(|a| Box::new(a) as Box<dyn std::any::Any>)
    }
}

// -- SummaryState

/// State for the `/summarize` floating modal.
#[derive(Debug, Clone)]
pub struct SummaryState {
    pub text: String,
    pub scroll_y: u16,
}

impl crate::overlay_component::OverlayComponent for SummaryState {
    fn id(&self) -> &'static str {
        "summary"
    }

    fn render_overlay(
        &mut self,
        frame: &mut ratatui::Frame,
        area: ratatui::layout::Rect,
        colors: &crate::colors::ThemeColors,
    ) {
        crate::app::layout::summary::render_summary(frame, self, area, colors);
    }

    fn handle_input(
        &mut self,
        key: crossterm::event::KeyEvent,
    ) -> crate::overlay_component::OverlayInputResult {
        use crate::overlay_component::OverlayInputResult;
        use crossterm::event::{KeyCode, KeyModifiers};

        match (key.code, key.modifiers) {
            (KeyCode::Esc, _)
            | (KeyCode::Enter, _)
            | (KeyCode::Char('c'), KeyModifiers::CONTROL) => OverlayInputResult::Dismiss,
            (KeyCode::Up, _) | (KeyCode::Char('k'), _) => {
                self.scroll_y = self.scroll_y.saturating_sub(1);
                OverlayInputResult::Consumed
            }
            (KeyCode::Down, _) | (KeyCode::Char('j'), _) => {
                self.scroll_y = self.scroll_y.saturating_add(1);
                OverlayInputResult::Consumed
            }
            (KeyCode::PageUp, _) => {
                self.scroll_y = self.scroll_y.saturating_sub(20);
                OverlayInputResult::Consumed
            }
            (KeyCode::PageDown, _) => {
                self.scroll_y = self.scroll_y.saturating_add(20);
                OverlayInputResult::Consumed
            }
            _ => OverlayInputResult::Consumed,
        }
    }
}

// -- ThinkingState

/// Active thinking animation state.
pub struct ThinkingState {
    /// Shared text updated by the assessing timer and on_event.
    pub text: Arc<Mutex<String>>,
    /// When this turn started (for elapsed time display).
    pub started: Instant,
}

#[derive(Debug, Clone, Copy)]
pub enum ToastLevel {
    Info,
    Success,
    Warning,
    Error,
}

#[derive(Debug, Clone)]
pub struct Toast {
    pub message: String,
    pub level: ToastLevel,
    pub created_at: Instant,
    pub ttl: std::time::Duration,
}

impl Toast {
    /// Returns true if the toast has lived past its TTL.
    pub fn is_expired(&self) -> bool {
        self.created_at.elapsed() >= self.ttl
    }
}

/// Pure tick logic for surfacing background-subagent completion toasts.
///
/// Called from the input loop's 50ms idle tick.  Mutates `toast` in place
/// only when `pending` has changed since the last announcement, so the
/// toast doesn't re-trigger on every tick.  Setting `pending = 0`
/// (REPL drained the queue) resets the announcement counter so a future
/// completion will re-toast.
///
/// Returns `true` if `toast` was written (caller should set `draw_dirty`).
///
/// Pure & non-async: takes `&mut Option<Toast>` and `&mut usize` so it
/// is testable without constructing a `TuiApp` (which requires a TTY).
pub fn tick_bg_pending_toast(
    pending: usize,
    last_announced: &mut usize,
    toast: &mut Option<Toast>,
) -> bool {
    if pending == *last_announced {
        return false;
    }
    *last_announced = pending;
    if pending == 0 {
        return false;
    }
    let msg = match pending {
        1 => "✓ Subagent finished — press Enter to receive".to_string(),
        n => format!("✓ {n} subagents finished — press Enter to receive"),
    };
    *toast = Some(Toast {
        message: msg,
        level: ToastLevel::Success,
        created_at: Instant::now(),
        ttl: std::time::Duration::from_secs(8),
    });
    true
}

// -- ActiveQuestionState
#[derive(Debug, Clone)]
pub struct ActiveQuestionDrawState {
    pub question: crate::question::Question,
    pub cursor_pos: usize,
    pub custom_text: String,
    pub checked: Vec<bool>,
    pub n_real: usize,
    pub has_other: bool,
    pub has_submit: bool,
    pub total_items: usize,
    pub other_idx: usize,
    pub submit_idx: usize,
}

use crate::overlay_component::{OverlayComponent, OverlayInputResult};
use std::any::Any;

pub struct ActiveQuestionState {
    pub draw_state: ActiveQuestionDrawState,
    pub tx: Option<tokio::sync::oneshot::Sender<Option<crate::question::QuestionAnswer>>>,
    pub result: Option<Option<crate::question::QuestionAnswer>>,
}

impl OverlayComponent for ActiveQuestionState {
    fn id(&self) -> &'static str {
        "active_question"
    }

    fn render_overlay(&mut self, _frame: &mut Frame, _area: Rect, _colors: &ThemeColors) {}

    fn render_inline(&self, frame: &mut Frame, area: Rect, colors: &ThemeColors) {
        let sep_area = Rect::new(area.x, area.y, area.width, 1);
        let body_area = Rect::new(
            area.x,
            area.y + 1,
            area.width,
            area.height.saturating_sub(1),
        );
        crate::app::layout::question::render_question_inline(
            frame,
            &self.draw_state,
            sep_area,
            body_area,
            colors,
        );
    }

    fn handle_input(&mut self, key: crossterm::event::KeyEvent) -> OverlayInputResult {
        use crossterm::event::{KeyCode, KeyModifiers};
        let st = &mut self.draw_state;
        let mut ans_opt: Option<Option<crate::question::QuestionAnswer>> = None;

        match (key.code, key.modifiers) {
            (KeyCode::Esc, _) | (KeyCode::Char('c'), KeyModifiers::CONTROL) => {
                ans_opt = Some(None);
            }
            (KeyCode::Up, _) => {
                if st.cursor_pos > 0 {
                    st.cursor_pos -= 1;
                }
            }
            (KeyCode::Down, _) => {
                if st.cursor_pos + 1 < st.total_items {
                    st.cursor_pos += 1;
                }
            }
            (KeyCode::Tab, _) => {
                st.cursor_pos = (st.cursor_pos + 1) % st.total_items;
            }
            (KeyCode::BackTab, _) => {
                st.cursor_pos = if st.cursor_pos == 0 {
                    st.total_items.saturating_sub(1)
                } else {
                    st.cursor_pos - 1
                };
            }
            (KeyCode::Char(c), KeyModifiers::NONE) if c.is_ascii_digit() && c != '0' => {
                let idx = (c as usize) - ('0' as usize) - 1;
                if idx < st.total_items {
                    if st.question.multi_select {
                        if idx < st.n_real {
                            st.checked[idx] = !st.checked[idx];
                            st.cursor_pos = idx;
                        }
                    } else if idx != st.other_idx {
                        let label = st.question.options[idx].label.clone();
                        ans_opt = Some(Some(crate::question::QuestionAnswer::Single(label)));
                    } else {
                        st.cursor_pos = idx;
                    }
                }
            }
            (KeyCode::Backspace, _) if st.cursor_pos == st.other_idx => {
                st.custom_text.pop();
            }
            (KeyCode::Enter, _) => {
                if st.question.multi_select {
                    if st.cursor_pos == st.submit_idx {
                        let selected: Vec<String> = st
                            .checked
                            .iter()
                            .enumerate()
                            .filter(|(_, c)| **c)
                            .map(|(i, _)| st.question.options[i].label.clone())
                            .collect();
                        if !selected.is_empty() {
                            ans_opt = Some(Some(crate::question::QuestionAnswer::Multi(selected)));
                        }
                    } else if st.cursor_pos == st.other_idx {
                        if !st.custom_text.is_empty() {
                            ans_opt = Some(Some(crate::question::QuestionAnswer::Multi(vec![
                                st.custom_text.clone(),
                            ])));
                        }
                    } else if st.cursor_pos < st.n_real {
                        st.checked[st.cursor_pos] = !st.checked[st.cursor_pos];
                    }
                } else if st.cursor_pos == st.other_idx {
                    if !st.custom_text.is_empty() {
                        ans_opt = Some(Some(crate::question::QuestionAnswer::Single(
                            st.custom_text.clone(),
                        )));
                    }
                } else {
                    let label = st.question.options[st.cursor_pos].label.clone();
                    ans_opt = Some(Some(crate::question::QuestionAnswer::Single(label)));
                }
            }
            (KeyCode::Char('u'), KeyModifiers::CONTROL) if st.cursor_pos == st.other_idx => {
                st.custom_text.clear();
            }
            (KeyCode::Char(c), m) if m == KeyModifiers::NONE || m == KeyModifiers::SHIFT => {
                if st.cursor_pos == st.other_idx {
                    st.custom_text.push(c);
                } else {
                    return OverlayInputResult::NotHandled;
                }
            }
            _ => return OverlayInputResult::NotHandled,
        }

        if let Some(ans) = ans_opt {
            if let Some(tx) = self.tx.take() {
                let _ = tx.send(ans.clone());
            }
            self.result = Some(ans);
            OverlayInputResult::Dismiss
        } else {
            OverlayInputResult::Consumed
        }
    }

    fn take_result(&mut self) -> Option<Box<dyn Any>> {
        self.result.take().map(|r| Box::new(r) as Box<dyn Any>)
    }

    fn inline_height(&self, max_height: u16) -> u16 {
        crate::app::layout::question::question_height(&self.draw_state, max_height)
    }
}

#[derive(Debug, Clone)]
pub struct PlanStep {
    pub id: usize,
    pub description: String,
    pub is_done: bool,
}

#[derive(Debug, Clone)]
pub struct PlanState {
    pub steps: Vec<PlanStep>,
    pub is_visible: bool,
    /// Scroll offset for the plan panel (0-based row index of the first visible step).
    pub scroll_offset: usize,
}

impl PlanState {
    /// Auto-scroll so the first incomplete step is visible.
    /// `visible_rows` is the number of steps that fit in the panel (excluding border).
    /// If all steps are done, scrolls to show the last steps.
    pub fn auto_scroll(&mut self, visible_rows: usize) {
        if visible_rows == 0 || self.steps.len() <= visible_rows {
            self.scroll_offset = 0;
            return;
        }

        let max_offset = self.steps.len().saturating_sub(visible_rows);

        // Find first incomplete step index
        if let Some(idx) = self.steps.iter().position(|s| !s.is_done) {
            // Scroll so that the incomplete step is near the middle of the visible area
            // but at minimum is visible
            let target = idx.saturating_sub(visible_rows / 3);
            self.scroll_offset = target.min(max_offset);
        } else {
            // All done → scroll to bottom
            self.scroll_offset = max_offset;
        }
    }
}

use regex::Regex;
use std::sync::OnceLock;

fn done_regex() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| Regex::new(r"(?i)\[DONE:(\d+)\]").expect("valid regex"))
}

/// Snap a byte offset to the nearest valid UTF-8 character boundary (rounding down).
fn snap_to_char_boundary(s: &str, byte_offset: usize) -> usize {
    let mut pos = byte_offset.min(s.len());
    while pos > 0 && !s.is_char_boundary(pos) {
        pos -= 1;
    }
    pos
}

// -- TuiApp

pub struct TuiApp {
    /// The single ratatui terminal (alternate screen, raw mode).
    pub terminal: DefaultTerminal,

    // -- Content state
    pub lines: Vec<RenderLine>,
    /// Lines scrolled up from the bottom.  0 = show latest content.
    pub scroll: usize,
    /// Target scroll position for smooth-scroll animation.
    /// When different from `scroll`, the tick loop interpolates toward it.
    scroll_target: usize,
    /// When true, snap to bottom on new content; disabled by manual scroll.
    pub follow: bool,
    pub expand_all: bool,
    /// Per-item expansion overrides keyed by stable timeline identity.
    expanded_items: std::collections::HashSet<TimelineKey>,
    pub active_plan: Option<PlanState>,

    // -- Streaming state
    streaming_text: String,
    streaming_active: bool,
    /// Typewriter reveal: number of bytes of `streaming_text` currently visible.
    /// Advances progressively each tick for smooth character-by-character output.
    streaming_reveal_len: usize,
    reasoning_text: String,
    reasoning_active: bool,

    // -- Input state
    pub editor: Box<dyn crate::editor_component::EditorComponent>,
    /// Image paste side-channel (not on the trait — image handling is
    /// a CADE-specific concern, not a generic editor concern).
    pub image_counter: usize,
    pub pending_paste_images: Vec<ImageEntry>,
    /// Last known terminal width — kept in sync during draw() so that
    /// Up/Down cursor navigation uses the real column width.
    term_width: u16,

    // -- Status / thinking
    pub thinking: Option<ThinkingState>,
    pub last_status: Option<String>,

    // -- Footer info
    pub mode: PermissionMode,
    pub agent_name: String,
    pub model: String,
    pub reasoning_effort: Option<String>,
    /// Abbreviated working directory shown in the footer.
    pub cwd: String,
    /// Context window usage (0–99 %) updated after each turn's usage event.
    pub context_pct: Option<u8>,
    /// Cumulative session token usage (input, output) for footer display.
    pub session_tokens: (u64, u64),
    /// Number of completed user→assistant turn pairs.
    pub turn_count: u32,
    /// Rolling history of context-window percentages (one per turn).
    /// Used by the sidebar sparkline widget. Max 50 entries.
    pub token_history: Vec<u8>,

    // -- Mouse capture disable mode (for OS text selection)
    pub mouse_capture_disabled: bool,

    // -- Autocomplete (A-01)
    /// File autocomplete provider (Tab path completion + `@` fuzzy picker).
    pub file_ac: FileAutocompleteProvider,

    // -- Dynamic overlay stack (Phase 3)
    /// Heterogeneous stack of modal overlays.  The host dispatches
    /// input to `overlays.last_mut()` and renders bottom-to-top.
    /// All modal overlays (file picker, theme picker, command palette,
    /// summary viewer) live here via [`OverlayComponent`].
    pub overlays: Vec<Box<dyn crate::overlay_component::OverlayComponent>>,

    // -- Image paste staging
    /// Images drained from the editor on the last submission.
    /// `repl.rs` reads and clears this after calling `read_input()`.
    pub pending_submit_images: Vec<ImageEntry>,

    // -- Extensibility slots (A-02)
    /// Pinned header rendered as a fixed strip above the messages pane.
    /// Populated by the caller (e.g. startup banner). Does not scroll.
    pub header_lines: Vec<RenderLine>,
    /// Optional extra row rendered below the footer (plugin/extension status).
    pub footer_extra: Option<String>,

    // -- Dynamic UI extension slots (Phase 4)
    /// Registry of plugin-injected widgets keyed by [`UiSlot`].
    /// The render path queries this for Header, Footer, and Sidebar
    /// components; each gets a dedicated layout region when occupied.
    pub slots: crate::slots::SlotManager,

    /// Lua script engine for UI extensions.
    pub lua_engine: Option<crate::lua_engine::LuaEngine>,

    // -- Scroll indicator
    /// Number of committed lines pushed while the user was scrolled up.
    /// Reset to 0 whenever scroll returns to 0 (bottom).
    pending_lines: usize,
    /// Number of follow-up messages currently queued (typed during a running turn).
    /// Shown as a badge in the status row so the user knows their input was accepted.
    pub queued_count: usize,

    /// Transient toast notification shown in the corner of the UI.
    pub toast: Option<Toast>,
    /// Width of the input area calculated during the last render.
    pub last_input_width: u16,

    /// Subagent trackers for rendering glass cards in the TUI.
    pub subagent_trackers: Vec<crate::subagent_tracker::SubagentTracker>,

    // -- Skills overlay

    // -- Render throttle (R-01)
    /// When true, the viewport has accumulated state changes that haven't
    /// been flushed to the terminal yet.  The tick task checks this flag
    /// every ~100 ms and calls `draw()` if set, ensuring trailing updates
    /// are never lost even when `draw_throttled()` skips a frame.
    pub draw_dirty: bool,
    /// Timestamp of the last successful `draw()`.  `draw_throttled()` skips
    /// the draw if less than `DRAW_MIN_INTERVAL` has elapsed, dramatically
    /// reducing redraws during high-frequency streaming (tokens, live bash).
    last_draw_at: Instant,

    /// Active color theme — replaces hardcoded RC::Rgb values at render time.
    pub colors: ThemeColors,

    /// When true, render Nerd Font glyphs for tool icons and status badges.
    /// When false, fall back to plain ASCII/Unicode symbols.
    pub use_nerd_fonts: bool,

    /// Optional getter that, when set, returns the count of background
    /// subagents that have completed and are waiting in the parent REPL's
    /// pending-results queue.  Polled during the input-loop's 50ms tick so
    /// the TUI can flash a toast ("✓ N subagents finished — press Enter")
    /// while the user is idle at the prompt.
    ///
    /// Stored as a boxed callback because `cade-tui` cannot depend on
    /// `cade-agent` (which owns the result type).  The CLI injects a
    /// closure over its `Arc<Mutex<Vec<BackgroundResult>>>` at startup.
    pub bg_pending_count: Option<Box<dyn Fn() -> usize + Send + Sync>>,

    /// Track how many pending background results the toast tick has
    /// already announced — so we don't spam the toast while the count
    /// stays the same between ticks.  Reset to 0 once the REPL drains.
    pub bg_last_announced: usize,
}

impl TuiApp {
    /// Create the TuiApp and initialise the ratatui terminal
    /// (enters alternate screen + enables raw mode).
    pub fn new(
        mode: PermissionMode,
        agent_name: String,
        model: String,
        reasoning_effort: Option<String>,
    ) -> Self {
        Self::new_with_theme(
            mode,
            agent_name,
            model,
            reasoning_effort,
            ThemeColors::default(),
        )
    }

    /// Create a `TuiApp` with an explicit color theme.
    pub fn new_with_theme(
        mode: PermissionMode,
        agent_name: String,
        model: String,
        reasoning_effort: Option<String>,
        colors: ThemeColors,
    ) -> Self {
        let terminal = ratatui::init();
        // No mouse capture on startup — terminal handles all mouse events
        // natively (click-and-drag selects text, right-click copies).
        // Scroll via keyboard: PgUp/PgDn, arrows, Ctrl+U/D.
        // Use /mouse to opt-in to scroll-wheel capture if preferred.
        let _ = crossterm::execute!(std::io::stdout(), EnableBracketedPaste, EnableFocusChange);
        // Many terminals (including Ghostty and WezTerm in some configs) fail to respond
        // to `supports_keyboard_enhancement()` within the timeout, or the user's setup
        // swallows the query. Unrecognized escape codes are safely ignored by VT100
        // terminals, so we unconditionally push the enhancement flags to ensure
        // Shift+Enter works where supported.
        let _ = crossterm::execute!(
            std::io::stdout(),
            PushKeyboardEnhancementFlags(KeyboardEnhancementFlags::DISAMBIGUATE_ESCAPE_CODES)
        );
        Self {
            terminal,
            lines: Vec::new(),
            scroll: 0,
            scroll_target: 0,
            follow: true,
            expand_all: false,
            expanded_items: std::collections::HashSet::new(),
            active_plan: None,
            streaming_text: String::new(),
            streaming_active: false,
            streaming_reveal_len: 0,
            reasoning_text: String::new(),
            reasoning_active: false,
            editor: Box::new(Editor::new()),
            image_counter: 0,
            pending_paste_images: Vec::new(),
            term_width: 80,
            thinking: None,
            last_status: None,
            mode,
            agent_name,
            model,
            reasoning_effort,
            cwd: abbreviate_cwd(&std::env::current_dir().unwrap_or_default()),
            context_pct: None,
            session_tokens: (0, 0),
            turn_count: 0,
            token_history: Vec::new(),
            mouse_capture_disabled: false,
            file_ac: FileAutocompleteProvider::new(std::env::current_dir().unwrap_or_default()),
            overlays: Vec::new(),
            pending_submit_images: Vec::new(),
            header_lines: Vec::new(),
            footer_extra: None,
            slots: crate::slots::SlotManager::new(),
            lua_engine: crate::lua_engine::LuaEngine::new().ok(),
            pending_lines: 0,
            queued_count: 0,
            toast: None,
            last_input_width: 80,
            subagent_trackers: Vec::new(),
            draw_dirty: false,
            last_draw_at: Instant::now(),
            colors,
            use_nerd_fonts: true,
            bg_pending_count: None,
            bg_last_announced: 0,
        }
    }

    // -- Content mutation

    /// Apply a new theme without re-initializing the terminal.
    pub fn apply_theme(&mut self, colors: ThemeColors) {
        self.colors = colors;
        self.draw_dirty = true;
        // U7: do NOT call self.draw() here — let the tick loop or the
        // caller coalesce redraws.  Previously every Up/Down in the
        // theme picker triggered two full draws per keystroke.
    }

    // -- Live output (streaming bash)

    // -- Config updates

    // -- Thinking animation

    // -- Rendering

    /// Redraw the full screen (unconditional — always redraws).
    pub fn draw(&mut self) -> Result<()> {
        self.draw_dirty = false;
        self.last_draw_at = Instant::now();
        // Auto-dismiss expired toasts
        if let Some(t) = &self.toast
            && t.is_expired()
        {
            self.toast = None;
        }
        self.tick_streaming_reveal();
        self.tick_smooth_scroll();
        self.draw_impl()
    }

    /// Advance smooth-scroll animation: interpolate `scroll` toward `scroll_target`.
    /// Called every draw cycle (~50ms). Moves ~40% of the remaining distance each
    /// tick, producing an ease-out deceleration curve.
    fn tick_smooth_scroll(&mut self) {
        if self.scroll == self.scroll_target {
            return;
        }
        let diff = if self.scroll_target > self.scroll {
            let d = self.scroll_target - self.scroll;
            let step = (d * 2 / 5).max(1); // ~40% of distance, min 1
            self.scroll = self.scroll.saturating_add(step);
            if self.scroll > self.scroll_target {
                self.scroll = self.scroll_target;
            }
            self.scroll_target - self.scroll
        } else {
            let d = self.scroll - self.scroll_target;
            let step = (d * 2 / 5).max(1);
            self.scroll = self.scroll.saturating_sub(step);
            if self.scroll < self.scroll_target {
                self.scroll = self.scroll_target;
            }
            self.scroll.abs_diff(self.scroll_target)
        };
        // Keep animating if not yet at target.
        if diff > 0 {
            self.draw_dirty = true;
        }
        // Snap follow state when we reach bottom.
        if self.scroll == 0 && self.scroll_target == 0 {
            self.follow = true;
            self.pending_lines = 0;
        }
    }

    /// Instantly set scroll position (no animation). Used by programmatic
    /// scroll changes (e.g. submit, follow, tool result auto-scroll).
    pub fn scroll_instant(&mut self, pos: usize) {
        self.scroll = pos;
        self.scroll_target = pos;
    }

    /// Advance the typewriter reveal cursor toward the full streaming text length.
    /// Called every draw cycle (~50ms). Reveals ~8 chars per tick (~160 chars/sec)
    /// which feels smooth without lagging behind fast model output.
    fn tick_streaming_reveal(&mut self) {
        if !self.streaming_active {
            return;
        }
        let target = self.streaming_text.len();
        if self.streaming_reveal_len < target {
            // Reveal rate: adaptive — faster when we're far behind, slower when close.
            let behind = target - self.streaming_reveal_len;
            let step = if behind > 200 {
                // Very far behind: catch up quickly (whole chunks at a time)
                behind / 2
            } else if behind > 50 {
                // Moderately behind: reveal ~20 chars per tick
                20
            } else {
                // Close to caught up: smooth typewriter at ~8 chars/tick
                8
            };
            self.streaming_reveal_len = (self.streaming_reveal_len + step).min(target);
            // If still not fully revealed, keep dirty so next tick continues.
            if self.streaming_reveal_len < target {
                self.draw_dirty = true;
            }
        }
    }

    /// R-01: Throttled redraw — skips the draw if less than DRAW_MIN_INTERVAL
    /// has elapsed since the last draw.  Sets `draw_dirty = true` so the tick
    /// task will pick up the pending update on its next cycle.  Used by
    /// high-frequency callers (`push_streaming_chunk`, `append_live_output_line`).
    pub fn draw_throttled(&mut self) -> Result<()> {
        self.draw_dirty = true;
        Ok(())
    }

    pub fn draw_impl(&mut self) -> Result<()> {
        // Snapshot all rendering data (avoids borrow conflicts).
        let lines = self.lines.clone();
        let streaming = if self.streaming_active {
            let full = crate::app::strip_orchestrator_prompts(&self.streaming_text).into_owned();
            // Typewriter effect: only reveal up to streaming_reveal_len bytes.
            let reveal = self.streaming_reveal_len.min(full.len());
            // Snap to a valid char boundary.
            let end = snap_to_char_boundary(&full, reveal);
            if end > 0 {
                Some(full[..end].to_string())
            } else {
                Some(String::new())
            }
        } else {
            None
        };
        let mut scroll = self.scroll;
        if self.follow {
            self.scroll = 0;
            self.scroll_target = 0;
            scroll = 0;
        }
        let mut textarea = {
            let text = self.editor.text();
            let cursor_byte = self.editor.cursor_pos();
            let mut ta = tui_textarea::TextArea::from(text.lines().map(|s| s.to_string()));
            // Restore cursor position from byte offset
            let lines = ta.lines().to_vec();
            let mut remaining = cursor_byte;
            for (row, line) in lines.iter().enumerate() {
                let line_len = line.len() + 1; // +1 for newline
                if remaining < line_len || row == lines.len() - 1 {
                    let col = remaining.min(line.len());
                    ta.move_cursor(tui_textarea::CursorMove::Jump(row as u16, col as u16));
                    break;
                }
                remaining -= line_len;
            }
            ta
        };
        let input_mode = self.editor_input_mode();
        let mode = self.mode;
        let agent_name = self.agent_name.clone();
        let model = self.model.clone();
        let last_status = self.last_status.clone();
        let thinking_text = self.thinking.as_ref().map(|ts| ts.text.lock().clone());
        let thinking_elapsed = self.thinking.as_ref().map(|ts| ts.started.elapsed());
        let expand_all = self.expand_all;
        let expanded_items = self.expanded_items.clone();
        let pending_lines = self.pending_lines;
        let queued_count = self.queued_count;
        let cwd = self.cwd.clone();
        let context_pct = self.context_pct;
        let turn_count = self.turn_count;
        let token_history = self.token_history.clone();
        let header_lines = self.header_lines.clone();
        let footer_extra = self.footer_extra.clone().or_else(|| {
            self.lua_engine.as_ref().and_then(|lua| lua.get_footer_text())
        });
        let reasoning_effort = self.reasoning_effort.clone();
        let mut active_plan_snap = self.active_plan.clone();
        // Auto-scroll plan panel to keep first incomplete step visible
        if let Some(plan) = &mut active_plan_snap {
            let plan_h = (plan.steps.len() as u16 + 2).min(10).max(4);
            let visible_rows = (plan_h.saturating_sub(2)) as usize;
            plan.auto_scroll(visible_rows);
            // Persist computed offset back so it's stable across frames
            if let Some(real_plan) = &mut self.active_plan {
                real_plan.scroll_offset = plan.scroll_offset;
            }
        }
        if self
            .toast
            .as_ref()
            .is_some_and(|t| t.created_at.elapsed() >= t.ttl)
        {
            self.toast = None;
        }
        let toast = self.toast.clone();
        let mouse_capture_disabled = self.mouse_capture_disabled;
        let colors = self.colors.clone();
        let nerd = self.use_nerd_fonts;

        // V-04: capture max_skip returned by render_frame to clamp self.scroll.
        let mut max_skip: u16 = 0;
        let mut input_cursor_pos: Option<(u16, u16)> = None;

        // CSI 2026: begin synchronized output — the terminal emulator buffers
        // all writes until the matching end sequence, then paints the entire
        // frame atomically.  Eliminates single-frame visual artifacts (tearing,
        // V-05 input field jump) on terminals that support it (kitty, WezTerm,
        // foot, ghostty, etc.).  Unsupported terminals silently ignore the
        // sequence — no feature detection needed.
        let _ = write!(std::io::stdout(), "\x1b[?2026h");

        // Temporarily take the overlay stack out of self so we can
        // call render_overlay(&mut self) inside the terminal.draw
        // closure (which already borrows self.terminal mutably).
        let mut overlay_stack = std::mem::take(&mut self.overlays);
        let mut slot_mgr = std::mem::take(&mut self.slots);

        self.terminal.draw(|frame| {
            let (m_skip, cur_pos) = render_frame(
                frame,
                &lines,
                streaming.as_deref(),
                scroll,
                expand_all,
                &mut textarea,
                input_mode,
                mode,
                &agent_name,
                &model,
                &last_status,
                thinking_text.as_deref(),
                thinking_elapsed,
                overlay_stack
                    .last()
                    .map(|o| &**o as &dyn crate::overlay_component::OverlayComponent),
                pending_lines,
                queued_count,
                &cwd,
                context_pct,
                self.session_tokens,
                turn_count,
                &token_history,
                &header_lines,
                footer_extra.as_deref(),
                reasoning_effort.as_deref(),
                active_plan_snap.as_ref(),
                mouse_capture_disabled,
                toast.as_ref(),
                &expanded_items,
                &colors,
                &mut self.last_input_width,
                nerd,
                &self.subagent_trackers,
            );
            max_skip = m_skip;
            input_cursor_pos = cur_pos;

            // -- Dynamic UI extension slots (Phase 4)
            // Slots render into dedicated regions of the frame.
            // Header: top of frame, Footer: bottom, Sidebar: right edge.
            // They paint on top of render_frame's output — occupied slots
            // use Clear to wipe their region first, so there's no bleed-through.
            {
                use crate::slots::UiSlot;
                use ratatui::layout::Rect;
                use ratatui::widgets::Clear;

                let full = frame.area();

                // -- Header slot: top N rows
                if let Some(hdr) = slot_mgr.get_mut(UiSlot::Header) {
                    let h = hdr.preferred_height().min(full.height / 4).max(1);
                    let area = Rect::new(full.x, full.y, full.width, h);
                    frame.render_widget(Clear, area);
                    hdr.render(frame, area, &colors);
                }

                // -- Footer slot: bottom N rows
                if let Some(ftr) = slot_mgr.get_mut(UiSlot::Footer) {
                    let h = ftr.preferred_height().min(full.height / 4).max(1);
                    let y = full.y + full.height.saturating_sub(h);
                    let area = Rect::new(full.x, y, full.width, h);
                    frame.render_widget(Clear, area);
                    ftr.render(frame, area, &colors);
                }

                // -- Sidebar slot: right edge (only when terminal is wide enough)
                if let Some(sb) = slot_mgr.get_mut(UiSlot::Sidebar) {
                    let sb_w = sb.preferred_height().min(full.width / 3).max(1);
                    let x = full.x + full.width.saturating_sub(sb_w);
                    let area = Rect::new(x, full.y, sb_w, full.height);
                    frame.render_widget(Clear, area);
                    sb.render(frame, area, &colors);
                }
            }

            // -- Dynamic overlay stack (Phase 3: renders on top of everything)
            if !overlay_stack.is_empty() {
                let full_area = frame.area();
                for overlay in overlay_stack.iter_mut() {
                    overlay.render_overlay(frame, full_area, &colors);
                }
                // When any overlay is open, hide the main cursor
                // (the overlay is responsible for its own cursor, if any).
                input_cursor_pos = None;
            }
        })?;

        // Restore the overlay stack and slot manager.
        self.overlays = overlay_stack;
        self.slots = slot_mgr;

        if let Some((x, y)) = input_cursor_pos {
            let _ = crossterm::execute!(
                std::io::stdout(),
                crossterm::cursor::MoveTo(x, y),
                crossterm::cursor::Show
            );
        } else {
            let _ = crossterm::execute!(std::io::stdout(), crossterm::cursor::Hide);
        }

        // CSI 2026: end synchronized output — terminal flushes the buffered
        // frame to the screen in one atomic paint.
        let _ = write!(std::io::stdout(), "\x1b[?2026l");
        let _ = std::io::stdout().flush();
        // V-04: clamp self.scroll so stale over-scroll doesn't trap the user.
        if self.scroll > max_skip as usize {
            self.scroll = max_skip as usize;
        }
        if self.scroll_target > max_skip as usize {
            self.scroll_target = max_skip as usize;
        }
        // Keep term_width in sync so Up/Down cursor navigation is accurate.
        if let Ok(sz) = crossterm::terminal::size() {
            self.term_width = sz.0;
        }
        Ok(())
    }
}

/// Called from repl.rs after each usage_statistics SSE event.
impl TuiApp {}

impl Drop for TuiApp {
    fn drop(&mut self) {
        let _ = crossterm::execute!(std::io::stdout(), PopKeyboardEnhancementFlags);
        // Only disable mouse capture if it was enabled via /mouse.
        if !self.mouse_capture_disabled {
            let _ = crossterm::execute!(std::io::stdout(), crossterm::event::DisableMouseCapture);
        }
        let _ = crossterm::execute!(std::io::stdout(), DisableBracketedPaste, DisableFocusChange);
        ratatui::restore();
    }
}

// region:    --- Tests

#[cfg(test)]
mod tests {
    #[allow(unused)]
    type Result<T> = core::result::Result<T, Box<dyn std::error::Error>>; // For tests.

    use super::render::count_wrapped_segment;
    use super::*;

    #[test]
    fn test_app_question_result_formatting() {
        // -- Setup & Fixtures
        let line = RenderLine::QuestionResult {
            header: "Decision".to_string(),
            answer: "Yes".to_string(),
        };

        // -- Check
        match line {
            RenderLine::QuestionResult { header, answer } => {
                assert_eq!(header, "Decision");
                assert_eq!(answer, "Yes");
            }
            _ => panic!("Expected QuestionResult"),
        }
    }

    #[test]
    fn test_app_count_wrapped_segment() {
        // -- Exec & Check
        assert_eq!(count_wrapped_segment("a", 10), 1);
        assert_eq!(count_wrapped_segment("1234567890", 10), 1);
        assert_eq!(count_wrapped_segment("12345678901", 10), 2);
        assert_eq!(count_wrapped_segment("123456789012345678901", 10), 3);
        assert_eq!(count_wrapped_segment("a 12345678901", 10), 3);
        assert_eq!(count_wrapped_segment("a 12345678901 ", 10), 3);
    }

    #[test]
    fn test_timeline_item_tool_call_measurement_smoke() {
        let line = RenderLine::ToolCall {
            name: "bash".to_string(),
            preview: "cargo test --workspace".to_string(),
        };
        let item = TimelineItem::from_render_line(&line);
        assert_eq!(item.kind(), TimelineItemKind::ToolCall);
        assert!(item.visual_rows(80, false, &ThemeColors::default(), true) >= 1);
    }

    #[test]
    fn test_timeline_item_maps_assistant_variant() {
        let line = RenderLine::AssistantText("hello".to_string());
        let item = TimelineItem::from_render_line(&line);
        assert!(matches!(item, TimelineItem::Assistant("hello")));
    }

    #[test]
    fn test_timeline_item_maps_system_variant() {
        let line = RenderLine::SystemMsg("info".to_string());
        let item = TimelineItem::from_render_line(&line);
        assert!(matches!(item, TimelineItem::System("info")));
    }

    #[test]
    fn test_timeline_entry_keys_are_stable() {
        let lines = vec![
            RenderLine::UserMessage("hello".to_string()),
            RenderLine::ToolCall {
                name: "bash".to_string(),
                preview: "cargo test".to_string(),
            },
            RenderLine::ToolResult {
                is_error: false,
                content: "ok".to_string(),
            },
        ];
        let entries = build_timeline_entries(&lines);
        assert_eq!(entries.len(), 3);
        assert_eq!(entries[0].key.index, 0);
        assert_eq!(entries[0].key.kind, TimelineItemKind::User);
        assert!(!entries[0].key.streaming);
        assert_eq!(entries[1].key.index, 1);
        assert_eq!(entries[1].key.kind, TimelineItemKind::ToolCall);
        assert_eq!(entries[2].key.kind, TimelineItemKind::ToolResult);

        let stream = TimelineEntry::streaming(entries.len(), "partial");
        assert_eq!(stream.key.index, 3);
        assert_eq!(stream.key.kind, TimelineItemKind::StreamingAssistant);
        assert!(stream.key.streaming);
    }

    #[test]
    fn test_per_item_expansion_state_changes_measurement() {
        let line = RenderLine::Reasoning {
            words: 3,
            content: "one\ntwo\nthree".to_string(),
        };
        let entry = TimelineEntry::from_render_line(0, &line);
        let colors = ThemeColors::default();
        let expanded: std::collections::HashSet<TimelineKey> = std::collections::HashSet::new();
        let collapsed_rows = entry.visual_rows_with_state(80, false, &expanded, &colors, true);

        let mut expanded = std::collections::HashSet::new();
        expanded.insert(entry.key);
        assert!(timeline_key_expanded(false, &expanded, &entry.key));
        let expanded_rows = entry.visual_rows_with_state(80, false, &expanded, &colors, true);
        assert!(expanded_rows > collapsed_rows);
    }

    #[test]
    fn test_prepare_timeline_entries_row_sum() {
        let lines = vec![
            RenderLine::UserMessage("hello".to_string()),
            RenderLine::AssistantText("world".to_string()),
            RenderLine::SystemMsg("info".to_string()),
        ];
        let entries = build_timeline_entries(&lines);
        let colors = ThemeColors::default();
        let expanded = std::collections::HashSet::new();
        let prepared = prepare_timeline_entries(&entries, 80, false, &expanded, &colors, true);
        assert_eq!(prepared.len(), 3);
        let total: u16 = prepared.iter().map(|p| p.rows).sum();
        assert!(total >= 3, "at least 1 row per item; got {total}");
    }

    #[test]
    fn test_snap_to_char_boundary_ascii() {
        let s = "hello world";
        assert_eq!(snap_to_char_boundary(s, 5), 5);
        assert_eq!(snap_to_char_boundary(s, 0), 0);
        assert_eq!(snap_to_char_boundary(s, 100), s.len());
    }

    #[test]
    fn test_snap_to_char_boundary_multibyte() {
        let s = "héllo"; // 'é' is 2 bytes in UTF-8
        // Byte layout: h(1) é(2) l(1) l(1) o(1) = 6 bytes
        assert_eq!(snap_to_char_boundary(s, 1), 1); // after 'h' — valid boundary
        assert_eq!(snap_to_char_boundary(s, 2), 1); // mid-'é' — snaps back to after 'h'
        assert_eq!(snap_to_char_boundary(s, 3), 3); // after 'é' — valid boundary
    }

    #[test]
    fn test_snap_to_char_boundary_emoji() {
        let s = "a🎉b"; // 🎉 is 4 bytes
        // Byte layout: a(1) 🎉(4) b(1) = 6 bytes
        assert_eq!(snap_to_char_boundary(s, 1), 1); // after 'a'
        assert_eq!(snap_to_char_boundary(s, 2), 1); // inside emoji, snap back to after 'a'
        assert_eq!(snap_to_char_boundary(s, 3), 1); // still inside emoji
        assert_eq!(snap_to_char_boundary(s, 4), 1); // still inside emoji
        assert_eq!(snap_to_char_boundary(s, 5), 5); // after emoji — valid
    }
    #[test]
    fn test_toast_expires_after_ttl() {
        let toast = Toast {
            message: "hello".to_string(),
            level: ToastLevel::Success,
            created_at: Instant::now() - std::time::Duration::from_secs(5),
            ttl: std::time::Duration::from_secs(3),
        };
        assert!(toast.is_expired(), "toast should be expired after TTL");

        let fresh = Toast {
            message: "fresh".to_string(),
            level: ToastLevel::Info,
            created_at: Instant::now(),
            ttl: std::time::Duration::from_secs(3),
        };
        assert!(!fresh.is_expired(), "fresh toast should not be expired");
    }

    // -- tick_bg_pending_toast

    #[test]
    fn tick_bg_no_change_returns_false_and_leaves_toast_alone() {
        let mut last = 2usize;
        let mut toast: Option<Toast> = None;
        let wrote = tick_bg_pending_toast(2, &mut last, &mut toast);
        assert!(!wrote, "no change must not write toast");
        assert!(toast.is_none());
        assert_eq!(last, 2);
    }

    #[test]
    fn tick_bg_singular_toast_for_one_pending() {
        let mut last = 0usize;
        let mut toast: Option<Toast> = None;
        let wrote = tick_bg_pending_toast(1, &mut last, &mut toast);
        assert!(wrote);
        let t = toast.expect("toast set");
        assert!(t.message.contains("Subagent finished"));
        assert!(matches!(t.level, ToastLevel::Success));
        assert_eq!(last, 1);
    }

    #[test]
    fn tick_bg_plural_toast_for_many() {
        let mut last = 0usize;
        let mut toast: Option<Toast> = None;
        let wrote = tick_bg_pending_toast(4, &mut last, &mut toast);
        assert!(wrote);
        assert!(
            toast
                .as_ref()
                .unwrap()
                .message
                .contains("4 subagents finished"),
            "got: {}",
            toast.unwrap().message
        );
        assert_eq!(last, 4);
    }

    #[test]
    fn tick_bg_drain_to_zero_resets_counter_without_toast() {
        let mut last = 3usize;
        let mut toast: Option<Toast> = None;
        let wrote = tick_bg_pending_toast(0, &mut last, &mut toast);
        assert!(!wrote, "draining to zero must not toast");
        assert!(toast.is_none());
        assert_eq!(last, 0, "counter must reset so future completions re-toast");
    }

    #[test]
    fn tick_bg_after_drain_re_announces_new_completion() {
        let mut last = 0usize;
        let mut toast: Option<Toast> = None;
        // Simulates: REPL just drained (last=0), then a new completion arrives.
        let wrote = tick_bg_pending_toast(1, &mut last, &mut toast);
        assert!(wrote);
        assert_eq!(last, 1);
    }

    // -- PlanState scroll offset

    #[test]
    fn plan_state_has_scroll_offset_defaulting_to_zero() {
        let plan = PlanState {
            steps: vec![PlanStep {
                id: 1,
                description: "task".into(),
                is_done: false,
            }],
            is_visible: true,
            scroll_offset: 0,
        };
        assert_eq!(plan.scroll_offset, 0);
    }

    #[test]
    fn plan_state_auto_scroll_targets_first_incomplete() {
        let mut plan = PlanState {
            steps: (1..=15)
                .map(|i| PlanStep {
                    id: i,
                    description: format!("Step {i}"),
                    is_done: i <= 10,
                })
                .collect(),
            is_visible: true,
            scroll_offset: 0,
        };
        plan.auto_scroll(8); // visible_rows = 8
        // First incomplete is step 11 (index 10).
        // Should scroll so step 11 is visible.
        // With 8 visible rows, offset should be at least 10 - 7 = 3
        assert!(
            plan.scroll_offset >= 3,
            "scroll_offset={}",
            plan.scroll_offset
        );
        assert!(plan.scroll_offset <= 10);
    }

    #[test]
    fn plan_state_auto_scroll_stays_zero_when_all_fit() {
        let mut plan = PlanState {
            steps: (1..=5)
                .map(|i| PlanStep {
                    id: i,
                    description: format!("Step {i}"),
                    is_done: false,
                })
                .collect(),
            is_visible: true,
            scroll_offset: 0,
        };
        plan.auto_scroll(8);
        assert_eq!(plan.scroll_offset, 0);
    }

    #[test]
    fn plan_state_auto_scroll_when_all_done() {
        let mut plan = PlanState {
            steps: (1..=15)
                .map(|i| PlanStep {
                    id: i,
                    description: format!("Step {i}"),
                    is_done: true,
                })
                .collect(),
            is_visible: true,
            scroll_offset: 0,
        };
        plan.auto_scroll(8);
        // All done → scroll to bottom so last steps visible
        let max_offset = plan.steps.len().saturating_sub(8);
        assert_eq!(plan.scroll_offset, max_offset);
    }

    #[test]
    #[ignore = "requires tty"]
    fn set_plan_initializes_scroll_offset_zero() {
        let mut app = TuiApp::new(
            cade_core::permissions::PermissionMode::Default,
            "test".into(),
            "test-model".into(),
            None,
        );
        app.set_plan(vec!["a".into(), "b".into(), "c".into()]);
        assert_eq!(app.active_plan.as_ref().unwrap().scroll_offset, 0);
    }
}

// endregion: --- Tests
