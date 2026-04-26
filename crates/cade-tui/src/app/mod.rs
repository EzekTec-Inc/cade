pub mod clipboard;
pub mod command_palette;
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
    widgets::{Block, Borders, Padding, Paragraph, Wrap},
};
use unicode_width::UnicodeWidthStr;

use crate::autocomplete::FileAutocompleteProvider;
use crate::colors::ThemeColors;
use crate::editor::{Editor, ImageEntry};
// Re-export for child modules that `use super::*`
pub(crate) use crate::editor::InputMode;
use cade_core::permissions::PermissionMode;

pub use layout::helpers::{cycle_mode, cycle_mode_back, truncate_str};
use layout::helpers::{abbreviate_cwd, display_tool_name};
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

// -- SummaryState

/// State for the `/summarize` floating modal.
#[derive(Debug, Clone)]
pub struct SummaryState {
    pub text: String,
    pub scroll_y: u16,
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
    pub active_question: Option<ActiveQuestionState>,
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
    /// Active `@` file picker overlay. `None` when inactive.
    pub picker: Option<PickerState>,
    /// Active `/theme` picker overlay. `None` when inactive.
    pub theme_picker: Option<ThemePickerState>,
    /// Active command palette overlay (`Ctrl+P`). `None` when inactive.
    pub command_palette: Option<command_palette::CommandPaletteState>,
    /// Active `/summarize` overlay. `None` when inactive.
    pub summary_overlay: Option<SummaryState>,

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
            scroll_target: 0,
            follow: true,
            expand_all: false,
            expanded_items: std::collections::HashSet::new(),
            active_question: None,
            active_plan: None,
            streaming_text: String::new(),
            streaming_active: false,
            streaming_reveal_len: 0,
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
            session_tokens: (0, 0),
            turn_count: 0,
            token_history: Vec::new(),
            mouse_capture_disabled: false,
            file_ac: FileAutocompleteProvider::new(std::env::current_dir().unwrap_or_default()),
            picker: None,
            theme_picker: None,
            command_palette: None,
            summary_overlay: None,
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
            bg_pending_count: None,
            bg_last_announced: 0,
        }
    }

    // -- Content mutation

    /// Apply a new theme without re-initializing the terminal.
    pub fn apply_theme(&mut self, colors: ThemeColors) {
        self.colors = colors;
        self.draw_dirty = true;
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
        // Auto-dismiss expired toasts
        if let Some(t) = &self.toast
            && t.is_expired() {
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
        let turn_count = self.turn_count;
        let token_history = self.token_history.clone();
        let picker = self.picker.clone();
        let theme_picker = self.theme_picker.clone();
        let command_palette = self.command_palette.clone();
        let summary_overlay = self.summary_overlay.clone();
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
                active_question.as_ref(),
                pending_lines,
                queued_count,
                &cwd,
                context_pct,
                self.session_tokens,
                turn_count,
                &token_history,
                picker.as_ref(),
                theme_picker.as_ref(),
                command_palette.as_ref(),
                summary_overlay.as_ref(),
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
            );
            max_skip = m_skip;
            input_cursor_pos = cur_pos;
        })?;

        if let Some((x, y)) = input_cursor_pos {
            let _ = crossterm::execute!(
                std::io::stdout(),
                crossterm::cursor::MoveTo(x, y),
                crossterm::cursor::Show
            );
        } else {
            let _ = crossterm::execute!(
                std::io::stdout(),
                crossterm::cursor::Hide
            );
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
        let s = "a🎉b";  // 🎉 is 4 bytes
        // Byte layout: a(1) 🎉(4) b(1) = 6 bytes
        assert_eq!(snap_to_char_boundary(s, 1), 1);  // after 'a'
        assert_eq!(snap_to_char_boundary(s, 2), 1);  // inside emoji, snap back to after 'a'
        assert_eq!(snap_to_char_boundary(s, 3), 1);  // still inside emoji
        assert_eq!(snap_to_char_boundary(s, 4), 1);  // still inside emoji
        assert_eq!(snap_to_char_boundary(s, 5), 5);  // after emoji — valid
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
            toast.as_ref().unwrap().message.contains("4 subagents finished"),
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
}

// endregion: --- Tests
