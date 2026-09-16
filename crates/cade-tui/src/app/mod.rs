pub mod clipboard;
pub mod command_palette;
pub mod copy_overlay;
pub mod frecency;
pub mod help_overlay;
pub mod input;
pub mod layout;
pub mod leader;
pub mod notifier;
pub mod password;
pub mod permission_overlay;
pub mod prompt_stash;
pub mod questions;
pub mod reducer;
pub mod render;
pub mod state;
pub mod subagent_inspector;
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

/// Resolve the session cost cap (in USD) for the sidebar budget gauge.
///
/// Precedence: `CADE_MAX_SESSION_COST_USD` env var > `.cade/settings.json`
/// (`max_session_cost_usd`, project wins over global) > `$120.00` default.
fn resolve_session_cost_cap(cwd: &std::path::Path) -> f64 {
    std::env::var("CADE_MAX_SESSION_COST_USD")
        .ok()
        .and_then(|v| v.parse::<f64>().ok())
        .filter(|v| *v > 0.0)
        .or_else(|| {
            cade_core::settings::SettingsManager::new(cwd)
                .ok()
                .and_then(|s| s.max_session_cost_usd())
        })
        .unwrap_or(120.00)
}

use parking_lot::Mutex;
use std::io::Write;
use std::sync::Arc;
use std::time::Instant;

use crate::Result;

use crossterm::event::{
    DisableBracketedPaste, DisableFocusChange, DisableMouseCapture, EnableBracketedPaste,
    EnableFocusChange, EnableMouseCapture, KeyboardEnhancementFlags, PopKeyboardEnhancementFlags,
    PushKeyboardEnhancementFlags,
};
use ratatui::{
    DefaultTerminal, Frame,
    layout::Rect,
    style::{Color as RC, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Padding, Paragraph, Wrap},
};

use crate::autocomplete::FileAutocompleteProvider;
use crate::colors::{ThemeColors, ThemeColorsExt};
use crate::editor::{Editor, ImageEntry};
// Re-export for child modules that `use super::*`
pub(crate) use crate::editor::InputMode;
use cade_core::permissions::PermissionMode;

use layout::helpers::{abbreviate_cwd, display_tool_name};
pub use layout::helpers::{cycle_mode, cycle_mode_back, truncate_str};
pub use reducer::TuiAction;
use render::{RenderContext, count_wrapped_rows, render_frame};

// -- Constants

/// Fixed non-input rows at the bottom: input borders + footer + hotkey bar.
const FIXED_ROWS: u16 = 4;
/// Maximum rows the input area may grow to.
const MAX_INPUT_ROWS: u16 = 6;
/// Vertical padding (rows) inside the scrollable content area.
const CONTENT_PAD_TOP: u16 = 1;
const CONTENT_PAD_BOT: u16 = 1;

/// Responsive layout breakpoint for showing the right sidebar.
const SIDEBAR_BREAKPOINT: u16 = 110;
/// Target width for the informational sidebar on wide terminals.
const SIDEBAR_WIDTH: u16 = 40;

/// Minimum interval between full redraws while live output is streaming or
/// thinking is being streamed.  Caps the render loop at ~30 FPS so the heavy
/// per-frame work (markdown parse, syntax highlighting, wrap) never runs at
/// the full 60 Hz tick; commit/finalize paths run with `streaming_active`
/// cleared and therefore always draw immediately.
const DRAW_MIN_INTERVAL: std::time::Duration = std::time::Duration::from_millis(33);

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
            (KeyCode::Char(c), m)
                if (m == KeyModifiers::NONE || m == KeyModifiers::SHIFT)
                    && st.cursor_pos == st.other_idx =>
            {
                st.custom_text.push(c);
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

#[allow(dead_code)]
pub(crate) fn snap_to_char_boundary(s: &str, byte_offset: usize) -> usize {
    let mut pos = byte_offset.min(s.len());
    while pos > 0 && !s.is_char_boundary(pos) {
        pos -= 1;
    }
    pos
}

/// Compute the typewriter-revealed prefix of the streaming text.
///
/// `reveal_len` advances in byte steps that may land mid-character for
/// multi-byte output (emoji, CJK, accented Latin), so the offset is always
/// snapped to a valid UTF-8 boundary before slicing.
#[allow(dead_code)]
pub(crate) fn streaming_revealed_prefix(full: &str, reveal_len: usize) -> String {
    let reveal = reveal_len.min(full.len());
    let end = snap_to_char_boundary(full, reveal);
    full[..end].to_string()
}

// -- TuiApp

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ServerBootStatus {
    Loading,
    Ready { tool_count: usize },
    Failed(String),
    Timeout(u64),
}

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
    /// When true, hide the right sidebar regardless of terminal width.
    pub sidebar_hidden: bool,

    // -- Streaming state
    streaming_text: String,
    streaming_active: bool,
    // Reveal-free streaming: no typewriter pacing.  The full accumulated text
    // is rendered on every chunk arrival via the identical markdown path used
    // for committed content, so styling/spacing never drifts while in flight.
    /// Stripped (prompt-free) mirror of `streaming_text`.  Refreshed once per
    /// incoming chunk so draw frames never re-run the strip regex over the
    /// whole accumulated response.
    streaming_display: String,
    reasoning_text: String,
    reasoning_active: bool,
    /// Stripped (prompt-free) mirror of `reasoning_text`, served to the live
    /// thinking block in the viewport while the model is reasoning.
    reasoning_display: String,

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
    /// Cumulative session cost in USD for sidebar budget gauge.
    pub session_cost_usd: f64,
    /// Session cost cap in USD (from .cade/settings.json, env var, or default).
    pub session_cost_cap_usd: f64,
    /// Number of completed user→assistant turn pairs.
    pub turn_count: u32,
    /// Rolling history of context-window percentages (one per turn).
    /// Used by the sidebar sparkline widget. Max 50 entries.
    pub token_history: Vec<u8>,

    // -- Mouse capture disable mode (for OS text selection)
    pub mouse_capture_disabled: bool,

    /// Area occupied by the scrollable messages viewport in the last draw.
    /// Used by click-to-copy to map terminal coordinates to RenderLine indices.
    pub messages_area: Rect,

    /// Line highlight for copy confirmation: (line_index, created_at).
    /// Cleared after ~400 ms in draw_impl().
    pub copy_highlight: Option<(usize, std::time::Instant)>,

    // -- Visual Selection state
    pub selection_start: Option<(u16, u16)>,
    pub selection_current: Option<(u16, u16)>,
    pub selection_active: bool,
    /// Retained visual selection highlight and cached text after mouse drag release.
    pub selection_retained: bool,
    pub retained_selected_text: Option<String>,
    pub(crate) clipboard: Option<arboard::Clipboard>,

    // -- TUI Settings & Configurable Keymap (Phase 1 Parity)
    pub tui_settings: cade_core::settings::tui::TuiSettings,
    pub keymap: crate::keys::Keymap,

    // -- Display Toggles, Attention, Prompt Stash (Phases 4 & 5)
    pub show_timestamps: bool,
    pub conceal_secrets: bool,
    pub collapse_tools: bool,
    pub thinking_visibility: String,
    pub prompt_stash: crate::app::prompt_stash::PromptStashStore,
    pub notifier: crate::app::notifier::TerminalNotifier,
    pub has_focus: bool,

    // -- Layout engine
    pub(crate) layout_engine: crate::app::timeline::TimelineLayoutEngine,

    /// Monotonically increasing version counter for conversation content.
    /// Incremented on every `push()`, `commit_streaming()`, `append_live_output_line()`, `clear()`.
    /// Used to invalidate the prepared-entries cache.
    pub content_version: u64,

    /// Currently active focused region in the workspace
    pub focused_region: crate::slots::FocusRegion,

    /// Last received keypress timestamp for flood throttling.
    pub last_keypress: std::time::Instant,
    /// Stored state indicating whether high-velocity simulated pasting is active.
    pub is_pasting: bool,
    /// Timestamp of the last scroll action for velocity acceleration (Section F).
    pub last_scroll_at: Option<std::time::Instant>,
    /// Streak count of consecutive rapid scroll events.
    pub scroll_streak: u16,

    // -- Autocomplete (A-01)
    /// File autocomplete provider (Tab path completion + `@` fuzzy picker).
    pub file_ac: FileAutocompleteProvider,
    /// Agent and Model autocomplete provider (Tab for @ and #).
    pub agent_model_ac: crate::autocomplete::AgentModelAutocompleteProvider,
    /// Slash command autocomplete provider.
    pub slash_ac: crate::autocomplete::SlashCommandProvider,
    /// Connected MCP servers and tool names provider.
    pub tool_ac: crate::autocomplete::ToolAutocompleteProvider,
    /// Next step suggestion provider.
    pub next_step_ac: crate::autocomplete::NextStepAutocompleteProvider,

    // -- Leader Key Chord Engine (Slice 1)
    pub leader_engine: crate::app::leader::LeaderKeyEngine,

    // -- Live Modified Files Tracker (Slice 3)
    pub modified_files_tracker: crate::app::layout::modified_files::ModifiedFilesTracker,

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

    /// Live boot status of all configured MCP servers.
    pub mcp_boot_status: Option<
        std::sync::Arc<parking_lot::Mutex<std::collections::HashMap<String, ServerBootStatus>>>,
    >,
    /// When did all configured MCP servers settle?
    pub mcp_all_settled_at: Option<std::time::Instant>,
    /// Shared signal that the background MCP boot has finished.
    pub startup_ready: Option<std::sync::Arc<std::sync::atomic::AtomicBool>>,
    /// Flag indicating whether the completed MCP boot has been processed and popped as a sentinel.
    pub mcp_processed: bool,
    /// When true, the MCP Engine Status card has been closed/dismissed after settling.
    pub mcp_closed: bool,

    // -- Declarative signal system (Pillar 1)
    /// Registry of state signals for declarative render triggers.
    pub signals: crate::signals::SignalRegistry,
}

impl TuiApp {
    pub fn prune_completed_subagents(&mut self) {
        let mut dirty = false;
        self.subagent_trackers.retain(|t| match &t.status {
            crate::subagent_tracker::SubagentStatus::Running => true,
            crate::subagent_tracker::SubagentStatus::Completed { finished_at } => {
                if finished_at.elapsed().as_secs() < 10 {
                    true
                } else {
                    dirty = true;
                    false
                }
            }
            crate::subagent_tracker::SubagentStatus::Failed { finished_at, .. } => {
                if finished_at.elapsed().as_secs() < 10 {
                    true
                } else {
                    dirty = true;
                    false
                }
            }
        });
        if dirty {
            self.draw_dirty = true;
        }
    }

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
        // Leave mouse capture disabled by default so native terminal text selection
        // works immediately. Users can opt into TUI-owned mouse gestures with /mouse.
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
        let lua_engine = crate::lua_engine::LuaEngine::new().ok();
        let mut slots = crate::slots::SlotManager::new();
        if let Some(engine) = &lua_engine {
            let sidebar = Box::new(crate::lua_ui::LuaUiSlot::new(
                false,
                engine.ui_event_queue.clone(),
            ));
            slots.set(crate::slots::UiSlot::Sidebar, sidebar);
        }

        let mut tui_settings = dirs::home_dir()
            .map(|h| cade_core::settings::tui::TuiSettings::load_from_dir(&h.join(".cade")))
            .unwrap_or_default();
        let cwd_path = std::env::current_dir().unwrap_or_default();
        let project_settings =
            cade_core::settings::tui::TuiSettings::load_from_dir(&cwd_path.join(".cade")).merge(
                cade_core::settings::tui::TuiSettings::load_from_dir(&cwd_path),
            );
        tui_settings = tui_settings.merge(project_settings);

        let keymap = crate::keys::Keymap::with_settings(&tui_settings);

        Self {
            terminal,
            lines: Vec::new(),
            scroll: 0,
            scroll_target: 0,
            follow: true,
            expand_all: false,
            expanded_items: std::collections::HashSet::new(),
            active_plan: None,
            sidebar_hidden: false,
            streaming_text: String::new(),
            streaming_active: false,
            streaming_display: String::new(),
            reasoning_text: String::new(),
            reasoning_active: false,
            reasoning_display: String::new(),
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
            session_cost_usd: 0.0,
            session_cost_cap_usd: resolve_session_cost_cap(
                &std::env::current_dir().unwrap_or_default(),
            ),
            turn_count: 0,
            token_history: Vec::new(),
            mouse_capture_disabled: false,
            messages_area: Rect::default(),
            copy_highlight: None,
            selection_start: None,
            selection_current: None,
            selection_active: false,
            selection_retained: false,
            retained_selected_text: None,
            clipboard: None,
            show_timestamps: tui_settings.show_timestamps,
            conceal_secrets: tui_settings.conceal_secrets,
            collapse_tools: tui_settings.collapse_tools,
            thinking_visibility: tui_settings.thinking_visibility.clone(),
            prompt_stash: crate::app::prompt_stash::PromptStashStore::load_default(),
            notifier: crate::app::notifier::TerminalNotifier::new(
                tui_settings.notifications.enable_bell || tui_settings.attention_sounds_enabled(),
                tui_settings.notifications.enable_osc || tui_settings.attention_enabled(),
                false,
            ),
            has_focus: true,
            tui_settings,
            keymap,
            layout_engine: crate::app::timeline::TimelineLayoutEngine::new(),
            content_version: 0,
            focused_region: crate::slots::FocusRegion::Input,
            last_keypress: std::time::Instant::now(),
            is_pasting: false,
            last_scroll_at: None,
            scroll_streak: 0,
            file_ac: FileAutocompleteProvider::new(std::env::current_dir().unwrap_or_default()),
            agent_model_ac: crate::autocomplete::AgentModelAutocompleteProvider::new(
                vec![],
                vec![],
            ),
            slash_ac: crate::autocomplete::SlashCommandProvider::new(vec![]),
            tool_ac: crate::autocomplete::ToolAutocompleteProvider::new(vec![], vec![]),
            next_step_ac: crate::autocomplete::NextStepAutocompleteProvider::new(vec![]),
            leader_engine: crate::app::leader::LeaderKeyEngine::new(),
            modified_files_tracker: crate::app::layout::modified_files::ModifiedFilesTracker::new(),
            overlays: Vec::new(),
            pending_submit_images: Vec::new(),
            header_lines: Vec::new(),
            footer_extra: None,
            slots,
            lua_engine,
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
            mcp_boot_status: None,
            mcp_all_settled_at: None,
            startup_ready: None,
            mcp_processed: false,
            mcp_closed: false,
            signals: crate::signals::SignalRegistry::new(),
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

    /// Toggle mouse capture dynamically (V-03 / /mouse command).
    pub fn toggle_mouse_capture(&mut self) -> bool {
        self.mouse_capture_disabled = !self.mouse_capture_disabled;
        if self.mouse_capture_disabled {
            let _ = crossterm::execute!(std::io::stdout(), DisableMouseCapture);
            self.show_toast(
                "Mouse capture disabled (native terminal selection active)",
                ToastLevel::Info,
            );
        } else {
            let _ = crossterm::execute!(std::io::stdout(), EnableMouseCapture);
            self.show_toast(
                "Mouse capture enabled (TUI selection active)",
                ToastLevel::Info,
            );
        }
        self.mouse_capture_disabled
    }

    // -- Live output (streaming bash)

    // -- Config updates

    // -- Thinking animation

    // -- Rendering

    pub fn refresh_lua_ui(&mut self) {
        if let Some(lua) = &self.lua_engine {
            // Sync context_pct to Lua
            if let Some(pct) = self.context_pct {
                let _ = lua.set_state_u8("context_pct", pct);
            } else {
                let _ = lua.set_state_nil("context_pct");
            }

            if let Some(sidebar_widget) = lua.get_sidebar_ui() {
                let mut new_sidebar = Box::new(crate::lua_ui::LuaUiSlot::new(
                    false,
                    lua.ui_event_queue.clone(),
                ));
                new_sidebar.update(Some(sidebar_widget));
                self.slots.set(crate::slots::UiSlot::Sidebar, new_sidebar);
            } else {
                let _ = self.slots.take(crate::slots::UiSlot::Sidebar);
            }

            if let Some(header_widget) = lua.get_header_ui() {
                tracing::info!("Found CADE_UI.header with {} widgets", header_widget.len());
                let mut new_header = Box::new(crate::lua_ui::LuaUiSlot::new(
                    true,
                    lua.ui_event_queue.clone(),
                ));
                new_header.update(Some(header_widget));
                self.slots.set(crate::slots::UiSlot::Header, new_header);
            } else {
                let _ = self.slots.take(crate::slots::UiSlot::Header);
            }
        }
    }

    /// Redraw the full screen (unconditional — always redraws).
    pub fn draw(&mut self) -> Result<()> {
        // R-01/Perf: while live output is streaming (assistant chunks or
        // thinking), cap redraws at ~30 FPS.  The 16 ms tick task keeps
        // `draw_dirty` set so we retry; content still animates smoothly,
        // but the heavy per-frame layout work runs half as often.
        if (self.streaming_active || self.reasoning_active)
            && self.last_draw_at.elapsed() < DRAW_MIN_INTERVAL
        {
            self.draw_dirty = true;
            return Ok(());
        }
        self.draw_dirty = false;
        self.last_draw_at = Instant::now();
        // Auto-dismiss expired toasts
        if let Some(t) = &self.toast
            && t.is_expired()
        {
            self.toast = None;
        }
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

    /// Snapshot of the streaming text currently visible in the viewport.
    ///
    /// Uses the prompt-stripped [`TuiApp::streaming_display`] buffer (refreshed
    /// once per chunk).  Reveal-free: the full accumulated text is returned on
    /// every chunk arrival so the rendered markdown never drifts while in
    /// flight.
    fn streaming_view_snapshot(&self) -> Option<String> {
        if !self.streaming_active {
            None
        } else {
            Some(self.streaming_display.clone())
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
        // Borrow rendering data by reference (avoids cloning entire data per frame).
        let lines: &[RenderLine] = &self.lines;
        // Live assistant text: reveal-limited typewriter view of the cached
        // prompt-stripped buffer (never re-runs the strip regex per frame).
        let streaming = self.streaming_view_snapshot();
        // Live thinking block: full prompt-stripped reasoning streamed inline
        // while the model reasons; collapsed into a RenderLine::Reasoning row
        // on commit.
        let reasoning = if self.reasoning_active {
            Some(self.reasoning_display.clone())
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
            ta.set_wrap_mode(tui_textarea::WrapMode::WordOrGlyph);
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
        let agent_name: &str = &self.agent_name;
        let model: &str = &self.model;
        let last_status = &self.last_status;
        let thinking_text = self.thinking.as_ref().map(|ts| ts.text.lock().clone());
        let thinking_elapsed = self.thinking.as_ref().map(|ts| ts.started.elapsed());
        let expand_all = self.expand_all;
        let expanded_items = &self.expanded_items;
        let queued_count = self.queued_count;
        let cwd: &str = &self.cwd;
        let context_pct = self.context_pct;
        let turn_count = self.turn_count;
        let token_history: &[u8] = &self.token_history;
        let header_lines: &[RenderLine] = &self.header_lines;
        let footer_extra = self.footer_extra.clone().or_else(|| {
            self.lua_engine
                .as_ref()
                .and_then(|lua| lua.get_footer_text())
        });
        let reasoning_effort: Option<&str> = self.reasoning_effort.as_deref();
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
        // Expire copy highlight after 400 ms.
        if self
            .copy_highlight
            .is_some_and(|(_, t)| t.elapsed() >= std::time::Duration::from_millis(400))
        {
            self.copy_highlight = None;
        }

        if self
            .toast
            .as_ref()
            .is_some_and(|t| t.created_at.elapsed() >= t.ttl)
        {
            self.toast = None;
        }
        let is_processing = self.is_processing();
        let toast: Option<&Toast> = if is_processing {
            None
        } else {
            self.toast.as_ref()
        };
        let colors: &ThemeColors = &self.colors;
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
        let working_dir_buf = std::env::current_dir().unwrap_or_default();
        let modified_files_entries = self.modified_files_tracker.entries(&working_dir_buf);

        let mut overlay_stack = std::mem::take(&mut self.overlays);
        let mut slot_mgr = std::mem::take(&mut self.slots);

        let selection_active = self.selection_active;
        let selection_start = self.selection_start;
        let selection_current = self.selection_current;

        let mut messages_area = Rect::default();
        self.terminal.draw(|frame| {
            let render_ctx = RenderContext {
                lines,
                streaming: streaming.as_deref(),
                reasoning: reasoning.as_deref(),
                scroll,
                expand_all,
                input_mode,
                mode,
                agent_name,
                model,
                last_status,
                thinking_text: thinking_text.as_deref(),
                thinking_elapsed,
                top_overlay: overlay_stack
                    .last()
                    .map(|o| &**o as &dyn crate::overlay_component::OverlayComponent),
                queued_count,
                cwd,
                context_pct,
                session_tokens: self.session_tokens,
                session_cost_usd: self.session_cost_usd,
                session_cost_cap_usd: self.session_cost_cap_usd,
                turn_count,
                token_history,
                header_lines,
                footer_extra: footer_extra.as_deref(),
                reasoning_effort,
                active_plan: active_plan_snap.as_ref(),
                sidebar_hidden: self.sidebar_hidden,
                toast,
                is_processing,
                copy_highlight: self.copy_highlight,
                mouse_selection: None,
                expanded_items,
                colors,
                nerd,
                subagent_trackers: &self.subagent_trackers,
                content_version: self.content_version,
                modified_files: &modified_files_entries,
            };
            let (m_skip, cur_pos, msg_area) = render_frame(
                frame,
                render_ctx,
                &mut textarea,
                &mut self.last_input_width,
                &mut self.layout_engine,
            );
            max_skip = m_skip;
            input_cursor_pos = cur_pos;
            messages_area = msg_area;

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
                    hdr.render(frame, area, colors);
                }

                // -- Footer slot: bottom N rows
                if let Some(ftr) = slot_mgr.get_mut(UiSlot::Footer) {
                    let h = ftr.preferred_height().min(full.height / 4).max(1);
                    let y = full.y + full.height.saturating_sub(h);
                    let area = Rect::new(full.x, y, full.width, h);
                    frame.render_widget(Clear, area);
                    ftr.render(frame, area, colors);
                }

                // -- Sidebar slot: right edge (only when terminal is wide enough)
                if let Some(sb) = slot_mgr.get_mut(UiSlot::Sidebar) {
                    let mut sb_w = sb.preferred_width().min(full.width / 3);
                    if sb_w == 0 {
                        sb_w = 40.min(full.width / 3);
                    }
                    let x = full.x + full.width.saturating_sub(sb_w);
                    let area = Rect::new(x, full.y, sb_w, full.height);
                    frame.render_widget(Clear, area);
                    sb.render(frame, area, colors);
                }
            }

            // Render MCP boot status card floating in the top right (suppressed while processing)
            if !is_processing
                && let Some(ref progress) = self.mcp_boot_status
                && !self.mcp_closed
            {
                let boot_map = progress.lock().clone();

                // Determine if we should show the card.
                // We show it if any server is Loading, OR if it's been less than 3 seconds since all settled.
                let mut show_card = false;
                let mut all_done = true;
                for status in boot_map.values() {
                    if matches!(status, ServerBootStatus::Loading) {
                        show_card = true;
                        all_done = false;
                    }
                }

                // Get all_settled_at timestamp
                if all_done {
                    if self.mcp_all_settled_at.is_none() {
                        self.mcp_all_settled_at = Some(std::time::Instant::now());
                    }
                } else {
                    self.mcp_all_settled_at = None;
                }

                if let Some(settled) = self.mcp_all_settled_at
                    && settled.elapsed() < std::time::Duration::from_secs(3)
                {
                    show_card = true;
                }

                if show_card && !boot_map.is_empty() {
                    use ratatui::layout::Rect;
                    use ratatui::style::Style;
                    use ratatui::text::{Line, Span};
                    use ratatui::widgets::{Block, Borders, Clear, Paragraph};

                    // Draw floating panel in the top-right corner
                    let full = frame.area();

                    // We need a height based on the number of servers + 2 for borders
                    let card_h = (boot_map.len() as u16 + 2).min(full.height.saturating_sub(2));
                    let card_w = 46u16; // fixed width for a neat look

                    // Avoid overlapping or casting outside the screen
                    if full.width > card_w && full.height > card_h {
                        let card_area = Rect::new(
                            full.x + full.width.saturating_sub(card_w).saturating_sub(2), // 2 columns padding from right
                            full.y + 1, // 1 row padding from top
                            card_w,
                            card_h,
                        );

                        // Clear the area first so there's no bleed-through
                        frame.render_widget(Clear, card_area);

                        let block = Block::default()
                            .title(Span::styled(
                                " MCP Engine Status ",
                                Style::default().bold().fg(colors.c_primary()),
                            ))
                            .borders(Borders::ALL)
                            .border_style(Style::default().fg(colors.c_text_muted()));

                        // Sort server list for stable, deterministic display
                        let mut sorted_servers: Vec<(&String, &ServerBootStatus)> =
                            boot_map.iter().collect();
                        sorted_servers.sort_by_key(|(k, _)| k.as_str());

                        let mut lines = Vec::new();
                        for (key, status) in sorted_servers {
                            let icon = match status {
                                ServerBootStatus::Loading => {
                                    let ms = std::time::SystemTime::now()
                                        .duration_since(std::time::SystemTime::UNIX_EPOCH)
                                        .unwrap_or_default()
                                        .as_millis();
                                    let frames = ["◜", "◝", "◞", "◟"];
                                    let idx = ((ms / 100) % 4) as usize;
                                    Span::styled(frames[idx], Style::default().cyan().bold())
                                }
                                ServerBootStatus::Ready { .. } => {
                                    Span::styled("✔", Style::default().green().bold())
                                }
                                ServerBootStatus::Failed(_) => {
                                    Span::styled("✗", Style::default().red().bold())
                                }
                                ServerBootStatus::Timeout(_) => {
                                    Span::styled("⚠", Style::default().yellow().bold())
                                }
                            };
                            let status_text = match status {
                                ServerBootStatus::Loading => {
                                    Span::styled("connecting...", Style::default().dim())
                                }
                                ServerBootStatus::Ready { tool_count } => Span::styled(
                                    format!("{tool_count} tools ready"),
                                    Style::default().green(),
                                ),
                                ServerBootStatus::Failed(err) => {
                                    let trunc_err: String = err.chars().take(22).collect();
                                    let suffix = if err.len() > 22 { ".." } else { "" };
                                    Span::styled(
                                        format!("{trunc_err}{suffix}"),
                                        Style::default().red(),
                                    )
                                }
                                ServerBootStatus::Timeout(secs) => Span::styled(
                                    format!("timeout ({secs}s)"),
                                    Style::default().yellow(),
                                ),
                            };

                            // Align nicely: server name on the left, status on the right.
                            // Width is card_w - 2 (borders) - 4 (spacing/icon). Let's pad dynamically.
                            let max_key_len = 16;
                            let trunc_key: String = key.chars().take(max_key_len).collect();
                            let formatted_key = format!("{:<max_key_len$}", trunc_key);

                            lines.push(Line::from(vec![
                                Span::raw("  "),
                                icon,
                                Span::raw("  "),
                                Span::styled(
                                    formatted_key,
                                    Style::default().fg(colors.c_text_primary()),
                                ),
                                Span::raw("   "),
                                status_text,
                            ]));
                        }

                        let paragraph = Paragraph::new(lines).block(block);
                        frame.render_widget(paragraph, card_area);
                    }
                }
            }

            // -- Dynamic overlay stack (Phase 3: renders on top of everything)
            if !overlay_stack.is_empty() {
                let full_area = frame.area();
                for overlay in overlay_stack.iter_mut() {
                    overlay.render_overlay(frame, full_area, colors);
                }
                // When any overlay is open, hide the main cursor
                // (the overlay is responsible for its own cursor, if any).
                input_cursor_pos = None;
            }

            // -- Leader Key Which-Key Bar (Slice 1)
            self.leader_engine
                .render_hint_bar(frame, frame.area(), colors);

            // Apply selection highlight onto the buffer before the frame is drawn/flushed.
            apply_selection_highlight(
                selection_active,
                selection_start,
                selection_current,
                frame.buffer_mut(),
            );
        })?;

        // Restore the overlay stack and slot manager.
        self.overlays = overlay_stack;
        self.slots = slot_mgr;

        // Stash the messages area rect for click-to-copy.
        self.messages_area = messages_area;

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
        if let Ok(sz) = crossterm::terminal::size()
            && sz.0 != self.term_width
        {
            let old_width = self.term_width;
            self.term_width = sz.0;

            if !self.follow && self.scroll > 0 {
                let timeline_entries = crate::app::timeline::build_timeline_entries(lines);
                let old_timeline_w = (old_width as usize).saturating_sub(4).max(1);
                let mut temp_engine = crate::app::timeline::TimelineLayoutEngine::new();
                let prepared_old = temp_engine.prepare_entries(
                    &timeline_entries,
                    old_timeline_w,
                    self.expand_all,
                    &self.expanded_items,
                    &self.colors,
                    self.use_nerd_fonts,
                );

                let total_visual_old: u16 = prepared_old.iter().map(|p| p.rows).sum();
                let visible_h = sz.1.saturating_sub(
                    FIXED_ROWS + MAX_INPUT_ROWS + CONTENT_PAD_TOP + CONTENT_PAD_BOT,
                );
                let max_skip_old = total_visual_old.saturating_sub(visible_h);
                let visible_start = max_skip_old.saturating_sub(self.scroll as u16);

                let mut item_start = 0u16;
                let mut anchor_index = 0;
                let mut anchor_offset = 0u16;
                for (idx, item) in prepared_old.iter().enumerate() {
                    let item_end = item_start.saturating_add(item.rows);
                    if item_start <= visible_start && visible_start < item_end {
                        anchor_index = idx;
                        anchor_offset = visible_start.saturating_sub(item_start);
                        break;
                    }
                    item_start = item_end;
                }

                let new_timeline_w = (sz.0 as usize).saturating_sub(4).max(1);
                let prepared_new = self.layout_engine.prepare_entries(
                    &timeline_entries,
                    new_timeline_w,
                    self.expand_all,
                    &self.expanded_items,
                    &self.colors,
                    self.use_nerd_fonts,
                );
                let total_visual_new: u16 = prepared_new.iter().map(|p| p.rows).sum();
                let mut new_item_start = 0u16;
                for (idx, item) in prepared_new.iter().enumerate() {
                    if idx == anchor_index {
                        break;
                    }
                    new_item_start = new_item_start.saturating_add(item.rows);
                }

                let new_visible_start = new_item_start.saturating_add(anchor_offset);
                let max_skip_new = total_visual_new.saturating_sub(visible_h);
                let new_scroll = max_skip_new.saturating_sub(new_visible_start);

                self.scroll = new_scroll as usize;
                self.scroll_target = new_scroll as usize;
            }
        }
        Ok(())
    }
}

impl TuiApp {
    /// Build the prepared-timeline-entry list the same way `render_frame` does.
    /// Uses a content-version cache to avoid re-parsing markdown / ANSI on every frame and on every mouse click.
    pub(crate) fn build_prepared_entries(
        &mut self,
    ) -> Vec<crate::app::timeline::PreparedTimelineEntry> {
        let timeline_w = self.messages_area.width.saturating_sub(4).max(1) as usize;

        let streaming = self.streaming_view_snapshot();
        let reasoning = if self.reasoning_active {
            Some(self.reasoning_display.clone())
        } else {
            None
        };

        self.layout_engine.set_active_stream(streaming.as_deref());
        self.layout_engine
            .set_active_reasoning(reasoning.as_deref());
        self.layout_engine
            .layout_items(
                &self.lines,
                timeline_w,
                self.expand_all,
                &self.expanded_items,
                &self.colors,
                self.use_nerd_fonts,
                self.content_version,
            )
            .to_vec()
    }

    /// Extract highlighted character range from active buffer without clearing state.
    pub fn extract_selected_text(&mut self) -> Option<String> {
        if !self.selection_active {
            return None;
        }

        let (x1, y1) = self.selection_start?;
        let (x2, y2) = self.selection_current?;

        use ratatui::layout::Rect;
        let inner = Rect {
            x: self.messages_area.x + 2,
            y: self.messages_area.y + 1,
            width: self.messages_area.width.saturating_sub(4),
            height: self.messages_area.height.saturating_sub(2),
        };

        if inner.width == 0 || inner.height == 0 {
            return None;
        }

        // Clamp coordinates to message viewport boundary
        let cx1 = x1.clamp(inner.x, inner.x + inner.width.saturating_sub(1));
        let cy1 = y1.clamp(inner.y, inner.y + inner.height.saturating_sub(1));
        let cx2 = x2.clamp(inner.x, inner.x + inner.width.saturating_sub(1));
        let cy2 = y2.clamp(inner.y, inner.y + inner.height.saturating_sub(1));

        if cx1 == cx2 && cy1 == cy2 {
            return None;
        }

        let (start_col, start_row, end_col, end_row) = if cy1 < cy2 || (cy1 == cy2 && cx1 <= cx2) {
            (cx1, cy1, cx2, cy2)
        } else {
            (cx2, cy2, cx1, cy1)
        };

        let prepared = self.build_prepared_entries();
        let total_visual: u16 = prepared
            .iter()
            .map(|p| p.rows as u32)
            .sum::<u32>()
            .min(u16::MAX as u32) as u16;
        let visible = inner.height;
        let max_skip = total_visual.saturating_sub(visible);
        let effective_up = (self.scroll as u16).min(max_skip);
        let visible_start = max_skip.saturating_sub(effective_up);

        let start_visual_row = start_row.saturating_sub(inner.y) + visible_start;
        let end_visual_row = end_row.saturating_sub(inner.y) + visible_start;

        let mut selected_text = String::new();
        let mut current_row: u16 = 0;

        for entry in &prepared {
            let entry_rows = entry.rows;
            let entry_start = current_row;
            let entry_end = entry_start + entry_rows;

            if entry_end > start_visual_row && entry_start <= end_visual_row {
                let offset = match entry.card_style {
                    crate::app::timeline::CardStyle::None => 0u16,
                    _ => 2u16,
                };

                for (i, line) in entry.lines.iter().enumerate() {
                    let line_row = entry_start + i as u16;
                    if line_row >= start_visual_row && line_row <= end_visual_row {
                        let line_text: String =
                            line.spans.iter().map(|s| s.content.as_ref()).collect();

                        let chars: Vec<char> = line_text.chars().collect();
                        let char_len = chars.len();

                        let slice_start = if line_row == start_visual_row {
                            (start_col.saturating_sub(inner.x + offset) as usize).min(char_len)
                        } else {
                            0
                        };

                        let slice_end = if line_row == end_visual_row {
                            ((end_col.saturating_sub(inner.x + offset) + 1) as usize).min(char_len)
                        } else {
                            char_len
                        };

                        if slice_start < slice_end {
                            let text_slice: String = chars[slice_start..slice_end].iter().collect();
                            let trimmed = text_slice.trim_end().to_string();
                            if !selected_text.is_empty() {
                                selected_text.push('\n');
                            }
                            selected_text.push_str(&trimmed);
                        }
                    }
                }
            }
            current_row = entry_end;
        }

        if selected_text.is_empty() {
            None
        } else {
            Some(selected_text)
        }
    }

    /// Dismiss the active/retained visual selection and highlight.
    pub fn clear_selection(&mut self) {
        self.selection_active = false;
        self.selection_retained = false;
        self.selection_start = None;
        self.selection_current = None;
        self.retained_selected_text = None;
        self.draw_dirty = true;
    }

    /// Extract highlighted character range from active buffer, copy it, and retain state (B.1 & B.2)
    pub fn copy_selected_text(&mut self) -> bool {
        let extracted = self.extract_selected_text();
        if let Some(selected_text) = extracted {
            self.retained_selected_text = Some(selected_text.clone());
            self.selection_retained = true;
            self.selection_active = true;

            let ok = self.write_to_clipboard(&selected_text);
            crate::app::clipboard::write_to_file_fallback(&selected_text);
            if ok {
                self.show_toast(
                    "Copied selection to clipboard",
                    crate::app::ToastLevel::Success,
                );
            } else {
                self.show_toast(
                    "Failed to copy selection to clipboard",
                    crate::app::ToastLevel::Error,
                );
            }
            self.draw_dirty = true;
            true
        } else {
            self.clear_selection();
            false
        }
    }

    /// Quote the active or retained selection into the prompt editor as a collapsed quote block (B.3).
    pub fn quote_selection_to_prompt(&mut self) -> bool {
        let text = self
            .retained_selected_text
            .clone()
            .or_else(|| self.extract_selected_text())
            .filter(|t| !t.trim().is_empty());

        let Some(text) = text else {
            self.clear_selection();
            self.show_toast(
                "No active selection to quote",
                crate::app::ToastLevel::Warning,
            );
            return false;
        };

        let mut quoted = String::new();
        for (i, line) in text.lines().enumerate() {
            if i > 0 {
                quoted.push('\n');
            }
            let trimmed = line.trim_end();
            if trimmed.is_empty() {
                quoted.push('>');
            } else if trimmed.starts_with("> ") || trimmed.starts_with('>') {
                quoted.push_str(trimmed);
            } else {
                quoted.push_str("> ");
                quoted.push_str(trimmed);
            }
        }

        self.editor.handle_paste(&quoted);
        self.clear_selection();
        self.show_toast(
            "Quoted selection added to prompt",
            crate::app::ToastLevel::Success,
        );
        self.draw_dirty = true;
        true
    }

    /// Copy the last non-empty message in the conversation transcript to the clipboard (C.3).
    pub fn copy_last_message(&mut self) -> bool {
        if self.copy_selected_text() {
            return true;
        }

        for line in self.lines.iter().rev() {
            let text = crate::app::copy_overlay::render_line_plain_text(line);
            let trimmed = text.trim();
            if !trimmed.is_empty() {
                let ok = self.write_to_clipboard(trimmed);
                crate::app::clipboard::write_to_file_fallback(trimmed);
                if ok {
                    self.show_toast(
                        "Copied last message to clipboard",
                        crate::app::ToastLevel::Success,
                    );
                } else {
                    self.show_toast("Failed to copy last message", crate::app::ToastLevel::Error);
                }
                self.draw_dirty = true;
                return ok;
            }
        }

        self.show_toast("No messages to copy", crate::app::ToastLevel::Info);
        false
    }

    /// Toggle concealment of tokens, secrets, and sensitive strings (Section H).
    pub fn toggle_conceal(&mut self) {
        self.conceal_secrets = !self.conceal_secrets;
        self.tui_settings.conceal_secrets = self.conceal_secrets;
        let _ = self.tui_settings.save_default();
        let status = if self.conceal_secrets { "ON" } else { "OFF" };
        self.show_toast(
            format!("Conceal mode: {status}"),
            crate::app::ToastLevel::Info,
        );
        self.draw_dirty = true;
    }

    /// Toggle timestamps display on timeline messages (Section H).
    pub fn toggle_timestamps(&mut self) {
        self.show_timestamps = !self.show_timestamps;
        self.tui_settings.show_timestamps = self.show_timestamps;
        let _ = self.tui_settings.save_default();
        let status = if self.show_timestamps { "ON" } else { "OFF" };
        self.show_toast(
            format!("Timestamps: {status}"),
            crate::app::ToastLevel::Info,
        );
        self.draw_dirty = true;
    }

    /// Toggle default collapse of tool call outputs (Section H).
    pub fn toggle_tools(&mut self) {
        self.collapse_tools = !self.collapse_tools;
        self.tui_settings.collapse_tools = self.collapse_tools;
        let _ = self.tui_settings.save_default();
        let status = if self.collapse_tools {
            "collapsed"
        } else {
            "expanded"
        };
        self.show_toast(
            format!("Tool outputs: {status}"),
            crate::app::ToastLevel::Info,
        );
        self.draw_dirty = true;
    }

    /// Cycle thinking block visibility: collapse -> full -> hide -> collapse (Section H).
    pub fn cycle_thinking_visibility(&mut self) {
        self.thinking_visibility = match self.thinking_visibility.as_str() {
            "collapse" => "full".to_string(),
            "full" => "hide".to_string(),
            _ => "collapse".to_string(),
        };
        self.tui_settings.thinking_visibility = self.thinking_visibility.clone();
        let _ = self.tui_settings.save_default();
        self.show_toast(
            format!("Thinking view: {}", self.thinking_visibility),
            crate::app::ToastLevel::Info,
        );
        self.draw_dirty = true;
    }

    /// Explicitly save runtime settings back to ~/.cade/tui.toml (Section H).
    pub fn save_settings(&mut self) -> bool {
        self.tui_settings.show_timestamps = self.show_timestamps;
        self.tui_settings.conceal_secrets = self.conceal_secrets;
        self.tui_settings.collapse_tools = self.collapse_tools;
        self.tui_settings.thinking_visibility = self.thinking_visibility.clone();
        match self.tui_settings.save_default() {
            Ok(_) => {
                self.show_toast(
                    "Saved settings to ~/.cade/tui.toml",
                    crate::app::ToastLevel::Success,
                );
                true
            }
            Err(e) => {
                self.show_toast(
                    format!("Failed to save settings: {e}"),
                    crate::app::ToastLevel::Error,
                );
                false
            }
        }
    }

    /// Trigger terminal attention notification if the app window currently lacks focus (Section I).
    pub fn notify_if_unfocused(
        &self,
        cue: crate::app::notifier::AttentionCue,
        title: &str,
        body: &str,
    ) {
        if !self.has_focus && self.tui_settings.attention_enabled() {
            self.notifier.notify(cue, title, body);
        }
    }

    /// Stash or pop prompt text buffer into/from prompt stash ring (Section J).
    pub fn stash_prompt(&mut self) {
        let current = self.editor.text();
        if !current.trim().is_empty() {
            self.prompt_stash.push(current);
            self.editor.clear();
            self.show_toast(
                "Prompt stashed to ~/.cade/prompt_stash.json",
                crate::app::ToastLevel::Success,
            );
        } else if let Some(stashed) = self.prompt_stash.pop() {
            self.editor.set_text(stashed);
            self.show_toast(
                "Prompt restored from stash",
                crate::app::ToastLevel::Success,
            );
        } else {
            self.show_toast("Prompt stash is empty", crate::app::ToastLevel::Warning);
        }
        self.draw_dirty = true;
    }

    /// Open external editor ($VISUAL or $EDITOR) on the current prompt (Section J).
    pub fn edit_in_external_editor(&mut self) -> std::result::Result<(), String> {
        let editor = std::env::var("VISUAL")
            .or_else(|_| std::env::var("EDITOR"))
            .unwrap_or_else(|_| {
                #[cfg(windows)]
                {
                    "notepad".to_string()
                }
                #[cfg(not(windows))]
                {
                    "nano".to_string()
                }
            });

        let mut temp_file = tempfile::Builder::new()
            .prefix("cade_prompt_")
            .suffix(".md")
            .tempfile()
            .map_err(|e| format!("Failed to create temp file: {e}"))?;

        use std::io::Write;
        temp_file
            .write_all(self.editor.text().as_bytes())
            .map_err(|e| format!("Failed to write to temp file: {e}"))?;
        let temp_path = temp_file.path().to_path_buf();

        // Suspend raw mode and execute editor
        let _ = crossterm::terminal::disable_raw_mode();
        let _ = crossterm::execute!(
            std::io::stdout(),
            crossterm::terminal::LeaveAlternateScreen,
            crossterm::cursor::Show
        );

        let status = std::process::Command::new(&editor).arg(&temp_path).status();

        let _ = crossterm::execute!(
            std::io::stdout(),
            crossterm::terminal::EnterAlternateScreen,
            crossterm::cursor::Hide
        );
        let _ = crossterm::terminal::enable_raw_mode();

        match status {
            Ok(s) if s.success() => {
                if let Ok(content) = std::fs::read_to_string(&temp_path) {
                    self.editor.set_text(content.trim_end().to_string());
                    self.show_toast(
                        "Prompt updated from external editor",
                        crate::app::ToastLevel::Success,
                    );
                    self.draw_dirty = true;
                }
                Ok(())
            }
            Ok(_) => {
                self.show_toast(
                    "External editor exited with error",
                    crate::app::ToastLevel::Error,
                );
                Err("External editor exited with non-zero status".to_string())
            }
            Err(e) => {
                self.show_toast(
                    format!("Failed to launch editor {editor}: {e}"),
                    crate::app::ToastLevel::Error,
                );
                Err(format!("Failed to launch editor: {e}"))
            }
        }
    }

    /// Redact API keys and secret tokens when conceal mode is active.
    pub fn conceal_if_active<'a>(&self, text: &'a str) -> std::borrow::Cow<'a, str> {
        if !self.conceal_secrets {
            return std::borrow::Cow::Borrowed(text);
        }
        let mut result = text.to_string();
        for prefix in &["sk-", "ghp_", "gho_", "glpat-", "xoxb-", "xoxp-", "AIza"] {
            while let Some(start) = result.find(prefix) {
                let end = result[start..]
                    .find(|c: char| {
                        c.is_whitespace() || c == '"' || c == '\'' || c == ',' || c == ')'
                    })
                    .map(|o| start + o)
                    .unwrap_or(result.len());
                result.replace_range(start..end, "***REDACTED***");
            }
        }
        std::borrow::Cow::Owned(result)
    }
}

/// Highlight the currently selected visual terminal cells (called during draw)
fn apply_selection_highlight(
    selection_active: bool,
    selection_start: Option<(u16, u16)>,
    selection_current: Option<(u16, u16)>,
    buffer: &mut ratatui::buffer::Buffer,
) {
    if selection_active
        && let (Some((x1, y1)), Some((x2, y2))) = (selection_start, selection_current)
    {
        let width = buffer.area.width;
        let height = buffer.area.height;
        if width == 0 || height == 0 {
            return;
        }

        // Sort start and end coordinates
        let (start_x, start_y, end_x, end_y) = if y1 < y2 || (y1 == y2 && x1 <= x2) {
            (x1, y1, x2, y2)
        } else {
            (x2, y2, x1, y1)
        };

        for y in start_y..=end_y {
            if y >= height {
                continue;
            }
            let min_x = if y == start_y { start_x } else { 0 };
            let max_x = if y == end_y {
                end_x
            } else {
                width.saturating_sub(1)
            };
            for x in min_x..=max_x {
                if x >= width {
                    continue;
                }
                let cell = &mut buffer[(x, y)];
                let fg = cell.fg;
                let bg = cell.bg;
                cell.set_fg(bg);
                cell.set_bg(fg);
                let current_style = cell.style();
                cell.set_style(current_style.add_modifier(ratatui::style::Modifier::REVERSED));
            }
        }
    }
}

/// Called from repl.rs after each usage_statistics SSE event.
impl TuiApp {}

impl Drop for TuiApp {
    fn drop(&mut self) {
        let _ = crossterm::execute!(std::io::stdout(), PopKeyboardEnhancementFlags);
        let _ = crossterm::execute!(
            std::io::stdout(),
            DisableBracketedPaste,
            DisableFocusChange,
            DisableMouseCapture
        );
        ratatui::restore();
    }
}

// region:    --- Tests

#[cfg(test)]
#[path = "app_tests.rs"]
mod tests;

// endregion: --- Tests
