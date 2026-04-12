pub mod clipboard;
pub mod input;
pub mod questions;
pub mod render;
pub mod layout;
pub mod timeline;
pub mod state;
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

/// TuiApp — single-terminal, pure ratatui fullscreen rendering for CADE.
/// Replaces the old hybrid (OutputRenderer DECSTBM + InputWidget Inline viewport +
/// ThinkingBar raw crossterm).  A single `Terminal<CrosstermBackend<Stdout>>`
/// (alternate screen, raw mode) is owned here.  Every piece of output — agent
/// streaming, tool results, slash-command text, errors — is represented as a
/// `RenderLine` pushed into `lines`.  `draw()` redraws the whole screen on every
/// state change, eliminating all the CPR / DECSTBM / blank-row-tracking hacks.
/// Layout (each frame):
/// ```text
/// ┌─────────────────────────────────────────┐
/// │       Content area  (scrollable)        │  term_h - (4 + input_rows)
/// ├─────────────────────────────────────────┤
/// │  ⠋ assessing…  OR  ✻ Considered for…   │  1  (status row)
/// ├─────────────────────────────────────────┤
/// │  ──────────────────────────── (sep)     │  1
/// │  > user input                           │  1..MAX_INPUT_ROWS
/// │  ──────────────────────────── (sep)     │  1
/// │  mode ✦          AgentName [model]      │  1  (footer)
/// └─────────────────────────────────────────┘
/// ```
use std::io::Write;
use std::sync::Arc;
use parking_lot::Mutex;
use std::time::Instant;

use crate::Result;
use crossterm::event::{
    DisableBracketedPaste, DisableFocusChange, DisableMouseCapture, EnableBracketedPaste,
    EnableFocusChange, EnableMouseCapture,
    KeyboardEnhancementFlags, PopKeyboardEnhancementFlags,
    PushKeyboardEnhancementFlags,
};
use ratatui::{
    DefaultTerminal, Frame,
    layout::Rect,
    style::{Color as RC, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, BorderType, Borders, Padding, Paragraph, Wrap},
};
use unicode_width::UnicodeWidthStr;

use crate::autocomplete::FileAutocompleteProvider;
use crate::colors::ThemeColors;
use crate::editor::{Editor, ImageEntry};
// Re-export for child modules that `use super::*`
pub(crate) use crate::editor::InputMode;
use cade_core::permissions::PermissionMode;

pub use layout::helpers::{cycle_mode, cycle_mode_back, truncate_str};
use layout::helpers::{abbreviate_cwd, display_tool_name, mode_sep_color};
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
}

// -- ThemePickerState

/// State for the `/theme` floating picker overlay.
#[derive(Debug, Clone)]
pub struct ThemePickerState {
    pub query: String,
    pub themes: Vec<cade_core::resources::themes::Theme>,
    pub filtered_indices: Vec<usize>,
    pub cursor: usize,
    /// If cancelled, restore to this
    pub original_theme: crate::colors::ThemeColors,
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

pub struct ActiveQuestionState {
    pub draw_state: ActiveQuestionDrawState,
    /// For async questions (ask_question_async).
    pub tx: Option<tokio::sync::oneshot::Sender<Option<crate::question::QuestionAnswer>>>,
    /// For blocking questions: key events forwarded from the tick task.
    pub key_tx: Option<std::sync::mpsc::SyncSender<crossterm::event::KeyEvent>>,
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
}

use regex::Regex;
use std::sync::OnceLock;

fn done_regex() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| Regex::new(r"(?i)\[DONE:(\d+)\]").expect("valid regex"))
}

// -- TuiApp

pub struct TuiApp {
    /// The single ratatui terminal (alternate screen, raw mode).
    pub terminal: DefaultTerminal,

    // -- Content state
    pub lines: Vec<RenderLine>,
    /// Lines scrolled up from the bottom.  0 = show latest content.
    pub scroll: usize,
    /// When true, snap to bottom on new content; disabled by manual scroll.
    pub follow: bool,
    pub expand_all: bool,
    /// Per-item expansion overrides keyed by stable timeline identity.
    expanded_items: std::collections::HashSet<TimelineKey>,
    pub active_question: Option<ActiveQuestionState>,
    pub active_plan: Option<PlanState>,

    // -- Streaming state
    streaming_text: String,
    streaming_active: bool,
    reasoning_text: String,
    reasoning_active: bool,

    // -- Input state
    pub editor: Editor<'static>,
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

    // -- Copy mode (disables mouse capture for OS text selection)
    pub copy_mode: bool,

    // -- Autocomplete (A-01)
    /// File autocomplete provider (Tab path completion + `@` fuzzy picker).
    pub file_ac: FileAutocompleteProvider,
    /// Active `@` file picker overlay. `None` when inactive.
    pub picker: Option<PickerState>,
    /// Active `/theme` picker overlay. `None` when inactive.
    pub theme_picker: Option<ThemePickerState>,

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
            ThemeColors::dark(),
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
        let _ = crossterm::execute!(
            std::io::stdout(),
            EnableMouseCapture,
            EnableBracketedPaste,
            EnableFocusChange
        );
        // Many terminals (including Ghostty and WezTerm in some configs) fail to respond
        // to `supports_keyboard_enhancement()` within the timeout, or the user's setup
        // swallows the query. Unrecognized escape codes are safely ignored by VT100
        // terminals, so we unconditionally push the enhancement flags to ensure
        // Shift+Enter works where supported.
        let _ = crossterm::execute!(
            std::io::stdout(),
            PushKeyboardEnhancementFlags(
                KeyboardEnhancementFlags::DISAMBIGUATE_ESCAPE_CODES
            )
        );
        Self {
            terminal,
            lines: Vec::new(),
            scroll: 0,
            follow: true,
            expand_all: false,
            expanded_items: std::collections::HashSet::new(),
            active_question: None,
            active_plan: None,
            streaming_text: String::new(),
            streaming_active: false,
            reasoning_text: String::new(),
            reasoning_active: false,
            editor: Editor::new(),
            term_width: 80,
            thinking: None,
            last_status: None,
            mode,
            agent_name,
            model,
            reasoning_effort,
            cwd: abbreviate_cwd(&std::env::current_dir().unwrap_or_default()),
            context_pct: None,
            copy_mode: false,
            file_ac: FileAutocompleteProvider::new(std::env::current_dir().unwrap_or_default()),
            picker: None,
            theme_picker: None,
            pending_submit_images: Vec::new(),
            header_lines: Vec::new(),
            footer_extra: None,
            pending_lines: 0,
            queued_count: 0,
            toast: None,
            last_input_width: 80,
            draw_dirty: false,
            last_draw_at: Instant::now(),
            colors,
            use_nerd_fonts: true,
        }
    }

    // -- Content mutation

    /// Apply a new theme without re-initializing the terminal.
    pub fn apply_theme(&mut self, colors: ThemeColors) {
        self.colors = colors;
        let _ = self.draw();
    }





















    // -- Live output (streaming bash)




    // -- Config updates



    // -- Thinking animation




    // -- Rendering

    /// Redraw the full screen (unconditional — always redraws).
    pub fn draw(&mut self) -> Result<()> {
        self.draw_dirty = false;
        self.last_draw_at = Instant::now();
        self.draw_impl()
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
            Some(crate::app::strip_orchestrator_prompts(&self.streaming_text).into_owned())
        } else {
            None
        };
        let mut scroll = self.scroll;
        if self.follow {
            self.scroll = 0;
            scroll = 0;
        }
        let mut textarea = self.editor.textarea.clone();
        let input_mode = self.editor.detect_mode();
        let mode = self.mode;
        let agent_name = self.agent_name.clone();
        let model = self.model.clone();
        let last_status = self.last_status.clone();
        let thinking_text = self
            .thinking
            .as_ref()
            .map(|ts| ts.text.lock().clone());
        let thinking_elapsed = self.thinking.as_ref().map(|ts| ts.started.elapsed());
        let expand_all = self.expand_all;
        let expanded_items = self.expanded_items.clone();
        let active_question = self.active_question.as_ref().map(|s| s.draw_state.clone());
        let pending_lines = self.pending_lines;
        let queued_count = self.queued_count;
        let cwd = self.cwd.clone();
        let context_pct = self.context_pct;
        let picker = self.picker.clone();
        let theme_picker = self.theme_picker.clone();
        let header_lines = self.header_lines.clone();
        let footer_extra = self.footer_extra.clone();
        let reasoning_effort = self.reasoning_effort.clone();
        let active_plan_snap = self.active_plan.clone();
        if self
            .toast
            .as_ref()
            .is_some_and(|t| t.created_at.elapsed() >= t.ttl)
        {
            self.toast = None;
        }
        let toast = self.toast.clone();
        let copy_mode = self.copy_mode;
        let colors = self.colors.clone();
        let nerd = self.use_nerd_fonts;

        // V-04: capture max_skip returned by render_frame to clamp self.scroll.
        let mut max_skip: u16 = 0;

        // CSI 2026: begin synchronized output — the terminal emulator buffers
        // all writes until the matching end sequence, then paints the entire
        // frame atomically.  Eliminates single-frame visual artifacts (tearing,
        // V-05 input field jump) on terminals that support it (kitty, WezTerm,
        // foot, ghostty, etc.).  Unsupported terminals silently ignore the
        // sequence — no feature detection needed.
        let _ = write!(std::io::stdout(), "\x1b[?2026h");

        self.terminal.draw(|frame| {
            max_skip = render_frame(
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
                active_question.as_ref(),
                pending_lines,
                queued_count,
                &cwd,
                context_pct,
                picker.as_ref(),
                theme_picker.as_ref(),
                &header_lines,
                footer_extra.as_deref(),
                reasoning_effort.as_deref(),
                active_plan_snap.as_ref(),
                copy_mode,
                toast.as_ref(),
                &expanded_items,
                &colors,
                &mut self.last_input_width,
                nerd,
            );
        })?;

        // CSI 2026: end synchronized output — terminal flushes the buffered
        // frame to the screen in one atomic paint.
        let _ = write!(std::io::stdout(), "\x1b[?2026l");
        let _ = std::io::stdout().flush();
        // V-04: clamp self.scroll so stale over-scroll doesn't trap the user.
        if self.scroll > max_skip as usize {
            self.scroll = max_skip as usize;
        }
        // Keep term_width in sync so Up/Down cursor navigation is accurate.
        if let Ok(sz) = crossterm::terminal::size() {
            self.term_width = sz.0;
        }
        Ok(())
    }





}


/// Called from repl.rs after each usage_statistics SSE event.
impl TuiApp {
}

impl Drop for TuiApp {
    fn drop(&mut self) {
        let _ = crossterm::execute!(std::io::stdout(), PopKeyboardEnhancementFlags);
        let _ = crossterm::execute!(
            std::io::stdout(),
            DisableBracketedPaste,
            DisableMouseCapture,
            DisableFocusChange
        );
        ratatui::restore();
    }
}


// region:    --- Tests

#[cfg(test)]
mod tests {
    #[allow(unused)]
    type Result<T> = core::result::Result<T, Box<dyn std::error::Error>>; // For tests.

    use super::*;
    use super::render::count_wrapped_segment;

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
        assert!(item.visual_rows(80, false, &ThemeColors::dark(), true) >= 1);
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
        let colors = ThemeColors::dark();
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
        let colors = ThemeColors::dark();
        let expanded = std::collections::HashSet::new();
        let prepared = prepare_timeline_entries(&entries, 80, false, &expanded, &colors, true);
        assert_eq!(prepared.len(), 3);
        let total: u16 = prepared.iter().map(|p| p.rows).sum();
        assert!(total >= 3, "at least 1 row per item; got {total}");
    }
}

// endregion: --- Tests
