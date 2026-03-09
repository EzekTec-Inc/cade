/// TuiApp — single-terminal, pure ratatui fullscreen rendering for CADE.
///
/// Replaces the old hybrid (OutputRenderer DECSTBM + InputWidget Inline viewport +
/// ThinkingBar raw crossterm).  A single `Terminal<CrosstermBackend<Stdout>>`
/// (alternate screen, raw mode) is owned here.  Every piece of output — agent
/// streaming, tool results, slash-command text, errors — is represented as a
/// `RenderLine` pushed into `lines`.  `draw()` redraws the whole screen on every
/// state change, eliminating all the CPR / DECSTBM / blank-row-tracking hacks.
///
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

use std::sync::{Arc, Mutex};
use std::time::Instant;

use anyhow::Result;
use crossterm::event::{
    self, EnableMouseCapture, DisableMouseCapture,
    Event, KeyCode, KeyEvent, KeyModifiers, MouseEventKind,
    PushKeyboardEnhancementFlags, PopKeyboardEnhancementFlags,
    KeyboardEnhancementFlags,
};
use crossterm::terminal::supports_keyboard_enhancement;
use ratatui::{
    DefaultTerminal, Frame,
    layout::{Constraint, Layout, Rect},
    style::{Color as RC, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Padding, Paragraph, Wrap},
};
use unicode_width::UnicodeWidthStr;

use crate::permissions::PermissionMode;

// ── Constants ─────────────────────────────────────────────────────────────────

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

// ── Skills overlay ────────────────────────────────────────────────────────────

/// Three-mode state machine for the skills overlay.
#[derive(Debug, Clone, PartialEq)]
pub enum SkillsMode { List, Detail, Edit }

/// Full-screen overlay for browsing and editing skills.
#[derive(Debug, Clone)]
pub struct SkillsOverlayState {
    pub skills:        Vec<crate::skills::Skill>,
    pub cursor:        usize,        // selected index in List
    pub list_scroll:   usize,
    pub detail_scroll: usize,
    pub mode:          SkillsMode,
    // 6 editable fields: [name, desc, category, tags_csv, triggers_csv, body]
    pub edit_fields:   Vec<String>,
    pub field_cursor:  usize,        // active field 0-5
    pub field_pos:     usize,        // byte cursor in active field
    pub dirty:         bool,
}

impl SkillsOverlayState {
    pub fn new(skills: Vec<crate::skills::Skill>) -> Self {
        Self {
            skills,
            cursor: 0, list_scroll: 0, detail_scroll: 0,
            mode: SkillsMode::List,
            edit_fields: vec![String::new(); 6],
            field_cursor: 0, field_pos: 0, dirty: false,
        }
    }

    /// Populate edit_fields from the currently selected skill.
    pub fn load_edit_fields(&mut self) {
        if let Some(s) = self.skills.get(self.cursor) {
            self.edit_fields = vec![
                s.name.clone(),
                s.description.clone(),
                s.category.clone().unwrap_or_default(),
                s.tags.join(", "),
                s.triggers.join(", "),
                s.body.clone(),
            ];
            self.field_cursor = 0;
            self.field_pos = 0;
            self.dirty = false;
        }
    }
}

// ── RenderLine ────────────────────────────────────────────────────────────────

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
    /// Interactive question completed result.
    QuestionResult { header: String, answer: String },
}

// ── PickerState (A-01) ────────────────────────────────────────────────────────

/// State for the `@` file fuzzy picker overlay.
#[derive(Debug, Clone)]
pub struct PickerState {
    /// Byte position of the `@` in `app.input` that activated the picker.
    pub at_pos:  usize,
    /// The query typed after `@` (grows as user types).
    pub query:   String,
    /// Matching file paths (relative to CWD), filtered by `query`.
    pub matches: Vec<String>,
    /// Index of the highlighted entry.
    pub cursor:  usize,
}

// ── ThinkingState ─────────────────────────────────────────────────────────────

/// Active thinking animation state.
pub struct ThinkingState {
    /// Shared text updated by the assessing timer and on_event.
    pub text:    Arc<Mutex<String>>,
    /// When this turn started (for elapsed time display).
    pub started: Instant,
}

// ── ActiveQuestionState ───────────────────────────────────────────────────────
#[derive(Debug, Clone)]
pub struct ActiveQuestionDrawState {
    pub question: crate::ui::question::Question,
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
    pub tx: Option<tokio::sync::oneshot::Sender<Option<crate::ui::question::QuestionAnswer>>>,
    /// For blocking questions: key events forwarded from the tick task.
    pub key_tx: Option<std::sync::mpsc::SyncSender<crossterm::event::KeyEvent>>,
}

// ── TuiApp ────────────────────────────────────────────────────────────────────

pub struct TuiApp {
    /// The single ratatui terminal (alternate screen, raw mode).
    pub terminal: DefaultTerminal,

    // ── Content state ──────────────────────────────────────────────────────
    pub lines:    Vec<RenderLine>,
    /// Lines scrolled up from the bottom.  0 = show latest content.
    pub scroll:   usize,
    pub expand_all: bool,
    pub active_question: Option<ActiveQuestionState>,

    // ── Streaming state ────────────────────────────────────────────────────
    streaming_text:   String,
    streaming_active: bool,
    reasoning_text:   String,
    reasoning_active: bool,

    // ── Input state ────────────────────────────────────────────────────────
    pub input:      String,
    pub cursor_pos: usize,
    /// Last known terminal width — kept in sync during draw() so that
    /// Up/Down cursor navigation uses the real column width.
    term_width: u16,

    // ── Status / thinking ──────────────────────────────────────────────────
    pub thinking:    Option<ThinkingState>,
    pub last_status: Option<String>,

    // ── Footer info ────────────────────────────────────────────────────────
    pub mode:        PermissionMode,
    pub agent_name:  String,
    pub model:       String,
    /// Abbreviated working directory shown in the footer.
    pub cwd:         String,
    /// Context window usage (0–99 %) updated after each turn's usage event.
    pub context_pct: Option<u8>,

    // ── Copy mode (disables mouse capture for OS text selection) ───────────
    pub copy_mode: bool,

    // ── File picker (A-01) ────────────────────────────────────────────────
    /// Active `@` file picker overlay. `None` when inactive.
    pub picker: Option<PickerState>,

    // ── Extensibility slots (A-02) ────────────────────────────────────────
    /// Pinned header rendered as a fixed strip above the messages pane.
    /// Populated by the caller (e.g. startup banner). Does not scroll.
    pub header_lines: Vec<RenderLine>,
    /// Optional extra row rendered below the footer (plugin/extension status).
    pub footer_extra: Option<String>,

    // ── Scroll indicator ──────────────────────────────────────────────────
    /// Number of committed lines pushed while the user was scrolled up.
    /// Reset to 0 whenever scroll returns to 0 (bottom).
    pending_lines: usize,
    /// Number of follow-up messages currently queued (typed during a running turn).
    /// Shown as a badge in the status row so the user knows their input was accepted.
    pub queued_count: usize,

    // ── Skills overlay ─────────────────────────────────────────────────────
    /// Full-screen skills browser/editor overlay. `None` when inactive.
    pub skills_overlay: Option<SkillsOverlayState>,
}

impl TuiApp {
    /// Create the TuiApp and initialise the ratatui terminal
    /// (enters alternate screen + enables raw mode).
    pub fn new(mode: PermissionMode, agent_name: String, model: String) -> Self {
        let terminal = ratatui::init();
        let _ = crossterm::execute!(std::io::stdout(), EnableMouseCapture);
        if supports_keyboard_enhancement().unwrap_or(false) {
            let _ = crossterm::execute!(
                std::io::stdout(),
                PushKeyboardEnhancementFlags(
                    KeyboardEnhancementFlags::DISAMBIGUATE_ESCAPE_CODES
                )
            );
        }
        Self {
            terminal,
            lines: Vec::new(),
            scroll: 0,
            expand_all: false,
            active_question: None,
            streaming_text: String::new(),
            streaming_active: false,
            reasoning_text: String::new(),
            reasoning_active: false,
            input: String::new(),
            cursor_pos: 0,
            term_width: 80,
            thinking: None,
            last_status: None,
            mode,
            agent_name,
            model,
            cwd:         abbreviate_cwd(&std::env::current_dir().unwrap_or_default()),
            context_pct: None,
            copy_mode: false,
            picker: None,
            header_lines: Vec::new(),
            footer_extra: None,
            pending_lines: 0,
            queued_count: 0,
            skills_overlay: None,
        }
    }

    // ── Content mutation ──────────────────────────────────────────────────

    /// Commit any in-progress streaming, push a line, and redraw.
    pub fn push(&mut self, line: RenderLine) -> Result<()> {
        self.commit_streaming_inner();
        self.commit_reasoning_inner();
        let is_tool_result = matches!(line, RenderLine::ToolResult { .. });
        self.lines.push(line);

        if is_tool_result {
            // Scroll to show the associated ToolCall header at the top of the
            // visible area.  When diff-preview lines sit between the ToolCall
            // and ToolResult (e.g. for file edits), a plain scroll=0 would
            // show only the bottom of the diff and clip the ToolCall off-screen.
            // rows_from_last_tool_call() counts visual rows from the most recent
            // ToolCall to the end of lines so the whole tool group scrolls into
            // view as a unit.
            self.scroll = self.rows_from_last_tool_call();
        } else {
            self.scroll = 0;
        }
        self.pending_lines = 0;
        let scroll_before = self.scroll;
        self.draw()?;
        // V-05: If V-04 clamped self.scroll during draw (rows_from_last_tool_call
        // overshot max_skip in short conversations), redraw immediately so the
        // first visible frame always uses the correct scroll value.
        if is_tool_result && self.scroll != scroll_before {
            return self.draw();
        }
        Ok(())
    }

    /// Count visual rows from the most recent `ToolCall` entry (inclusive) to
    /// the end of `self.lines`.  The result is used as the scroll offset so
    /// that the ToolCall header appears at the top of the viewport when the
    /// corresponding ToolResult is pushed.
    fn rows_from_last_tool_call(&self) -> usize {
        let w = self.term_width.max(20) as usize;
        let cw = self.term_width.max(20);
        let mut total: u16 = 0;
        for rl in self.lines.iter().rev() {
            let mut text_lines: Vec<ratatui::text::Line<'static>> = Vec::new();
            render_line_to_text(rl, w, self.expand_all, &mut text_lines);
            for tl in &text_lines {
                total = total.saturating_add(count_wrapped_rows(tl, cw));
            }
            if matches!(rl, RenderLine::ToolCall { .. }) {
                return total as usize;
            }
        }
        0 // no ToolCall found — stay at bottom
    }

    /// Push without redrawing (for bulk initialisation / banner).
    pub fn push_silent(&mut self, line: RenderLine) {
        self.commit_streaming_inner();
        self.commit_reasoning_inner();
        self.lines.push(line);
    }

    /// Append a streaming chunk and redraw.
    pub fn push_streaming_chunk(&mut self, text: &str) -> Result<()> {
        self.commit_reasoning_inner();
        if !self.streaming_active {
            // First chunk of a new agent response — always snap to bottom so the
            // analysis is immediately visible.  push(ToolResult) may have scrolled
            // up to show the ToolCall header; that view is correct while the tool
            // was running, but as soon as the agent starts responding the viewport
            // must follow the output.
            self.scroll        = 0;
            self.pending_lines = 0;
        }
        // Subsequent chunks of the same response preserve scroll (V-01):
        // if the user scrolled up mid-stream to read history, leave them there.
        self.streaming_active = true;
        self.streaming_text.push_str(text);
        self.draw()
    }

    /// Append a reasoning chunk (accumulated; committed as header on done).
    pub fn push_reasoning_chunk(&mut self, text: &str) {
        self.reasoning_active = true;
        self.reasoning_text.push_str(text);
    }

    /// Commit any in-progress assistant streaming to `lines`.
    pub fn commit_streaming(&mut self) -> Result<()> {
        self.commit_streaming_inner();
        // Snap to bottom when streaming commits — the completed response must
        // be visible.  Only mid-stream chunks (push_streaming_chunk) preserve
        // the user's scroll position; once the response is fully committed here
        // we always show it.
        self.scroll        = 0;
        self.pending_lines = 0;
        self.draw()
    }

    /// Commit reasoning block as a collapsed header.
    pub fn commit_reasoning(&mut self) -> Result<()> {
        self.commit_reasoning_inner();
        self.draw()
    }

    /// Discard streaming state without committing (on cancel / error).
    pub fn discard_streaming(&mut self) {
        self.streaming_text.clear();
        self.streaming_active = false;
        self.reasoning_text.clear();
        self.reasoning_active = false;
    }

    pub fn has_streaming(&self) -> bool { self.streaming_active }

    /// Toggle OS text-selection copy mode on/off.
    /// When ON: mouse capture is disabled so the terminal lets the user select text.
    /// When OFF: mouse capture is restored so scroll wheel works normally.
    pub fn toggle_copy_mode(&mut self) {
        self.copy_mode = !self.copy_mode;
        if self.copy_mode {
            let _ = crossterm::execute!(std::io::stdout(), DisableMouseCapture);
        } else {
            let _ = crossterm::execute!(std::io::stdout(), EnableMouseCapture);
        }
    }

    /// Clear all content (e.g. /clear).
    pub fn clear_content(&mut self) -> Result<()> {
        self.lines.clear();
        self.discard_streaming();
        self.scroll = 0;
        self.draw()
    }

    fn commit_streaming_inner(&mut self) {
        if self.streaming_active {
            let text = std::mem::take(&mut self.streaming_text);
            if !text.trim().is_empty() {
                self.lines.push(RenderLine::AssistantText(text));
            }
            self.streaming_active = false;
        }
    }

    fn commit_reasoning_inner(&mut self) {
        if self.reasoning_active {
            let text = std::mem::take(&mut self.reasoning_text);
            let words = text.split_whitespace().count();
            if words > 0 {
                self.lines.push(RenderLine::Reasoning { words, content: text });
            }
            self.reasoning_active = false;
        }
    }

    // ── Config updates ────────────────────────────────────────────────────

    pub fn update_model(&mut self, model: String)           { self.model = model; }
    pub fn update_mode(&mut self, mode: PermissionMode)     { self.mode  = mode; }
    pub fn update_agent_name(&mut self, name: String)       { self.agent_name = name; }
    pub fn set_last_status(&mut self, s: Option<String>)    { self.last_status = s; }

    // ── Thinking animation ────────────────────────────────────────────────

    /// Start the thinking animation.  Returns the shared text Arc so callers
    /// can update the status text (e.g. assessing timer, tool name updates).
    pub fn start_thinking(&mut self, text: impl Into<String>) -> Arc<Mutex<String>> {
        self.scroll = 0; // snap to bottom at the start of every agent turn
        let arc = Arc::new(Mutex::new(text.into()));
        self.thinking = Some(ThinkingState { text: arc.clone(), started: Instant::now() });
        arc
    }

    /// Update the thinking text from the animation/assessing timer.
    pub fn update_thinking_text(&mut self, text: String) {
        if let Some(ref ts) = self.thinking {
            *ts.text.lock().unwrap() = text;
        }
    }

    /// Stop the thinking animation.  Returns elapsed seconds (for summary line).
    pub fn stop_thinking(&mut self) -> u64 {
        let secs = self.thinking.as_ref()
            .map(|ts| ts.started.elapsed().as_secs())
            .unwrap_or(0);
        self.thinking = None;
        secs
    }

    // ── Rendering ─────────────────────────────────────────────────────────

    /// Redraw the full screen.
    pub fn draw(&mut self) -> Result<()> { self.draw_impl() }

    pub fn draw_impl(&mut self) -> Result<()> {
        // Snapshot all rendering data (avoids borrow conflicts).
        let lines            = self.lines.clone();
        let streaming        = if self.streaming_active { Some(self.streaming_text.clone()) } else { None };
        let scroll           = self.scroll;
        let input            = self.input.clone();
        let cursor_pos       = self.cursor_pos;
        let mode             = self.mode;
        let agent_name       = self.agent_name.clone();
        let model            = self.model.clone();
        let last_status      = self.last_status.clone();
        let thinking_text    = self.thinking.as_ref().map(|ts| ts.text.lock().unwrap().clone());
        let thinking_elapsed = self.thinking.as_ref().map(|ts| ts.started.elapsed());
        let expand_all       = self.expand_all;
        let active_question  = self.active_question.as_ref().map(|s| s.draw_state.clone());
        let pending_lines    = self.pending_lines;
        let queued_count     = self.queued_count;
        let cwd              = self.cwd.clone();
        let context_pct      = self.context_pct;
        let picker           = self.picker.clone();
        let header_lines     = self.header_lines.clone();
        let footer_extra     = self.footer_extra.clone();
        let skills_overlay_snap = self.skills_overlay.clone();

        // V-04: capture max_skip returned by render_frame to clamp self.scroll.
        let mut max_skip: u16 = 0;
        self.terminal.draw(|frame| {
            max_skip = render_frame(
                frame,
                &lines,
                streaming.as_deref(),
                scroll,
                expand_all,
                &input,
                cursor_pos,
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
                &header_lines,
                footer_extra.as_deref(),
                skills_overlay_snap.as_ref(),
            );
        })?;
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

    // ── Interactive Question ──────────────────────────────────────────────

    pub fn ask_question(&mut self, question: &crate::ui::question::Question) -> Result<Option<crate::ui::question::QuestionAnswer>> {
        let n_real     = question.options.len();
        let has_other  = question.allow_other;
        let has_submit = question.multi_select;
        let total_items = n_real + usize::from(has_other) + usize::from(has_submit);

        let other_idx  = if has_other  { n_real } else { usize::MAX };
        let submit_idx = if has_submit { n_real + usize::from(has_other) } else { usize::MAX };

        let mut cursor_pos: usize = 0;
        let mut custom_text = String::new();
        let mut checked: Vec<bool> = vec![false; n_real];

        // snap to bottom when asking
        self.scroll = 0;

        let answer: Option<crate::ui::question::QuestionAnswer> = 'widget: loop {
            self.active_question = Some(ActiveQuestionState {
                draw_state: ActiveQuestionDrawState {
                    question: question.clone(),
                    cursor_pos,
                    custom_text: custom_text.clone(),
                    checked: checked.clone(),
                    n_real,
                    has_other,
                    has_submit,
                    total_items,
                    other_idx,
                    submit_idx,
                },
                tx: None,
                key_tx: None,
            });

            self.draw()?;

            if !event::poll(std::time::Duration::from_millis(50))? {
                continue;
            }
            match event::read()? {
                Event::Key(KeyEvent { code, modifiers, .. }) => {
                    match (code, modifiers) {
                        (KeyCode::Esc, _) | (KeyCode::Char('c'), KeyModifiers::CONTROL) => {
                            break 'widget None;
                        }
                        (KeyCode::Up, _) => { if cursor_pos > 0 { cursor_pos -= 1; } }
                        (KeyCode::Down, _) => { if cursor_pos + 1 < total_items { cursor_pos += 1; } }
                        (KeyCode::Tab, _) => { cursor_pos = (cursor_pos + 1) % total_items; }
                        (KeyCode::BackTab, _) => { cursor_pos = if cursor_pos == 0 { total_items - 1 } else { cursor_pos - 1 }; }
                        (KeyCode::Char(c), KeyModifiers::NONE) if c.is_ascii_digit() && c != '0' => {
                            let idx = (c as usize) - ('0' as usize) - 1;
                            if idx < total_items {
                                if question.multi_select {
                                    if idx < n_real {
                                        checked[idx] = !checked[idx];
                                        cursor_pos = idx;
                                    }
                                } else if idx != other_idx {
                                    let label = question.options[idx].label.clone();
                                    break 'widget Some(crate::ui::question::QuestionAnswer::Single(label));
                                } else {
                                    cursor_pos = idx;
                                }
                            }
                        }
                        (KeyCode::Backspace, _) if cursor_pos == other_idx => { custom_text.pop(); }
                        (KeyCode::Enter, _) => {
                            if question.multi_select {
                                if cursor_pos == submit_idx {
                                    let selected: Vec<String> = checked.iter().enumerate()
                                        .filter(|(_, c)| **c)
                                        .map(|(i, _)| question.options[i].label.clone())
                                        .collect();
                                    if !selected.is_empty() {
                                        break 'widget Some(crate::ui::question::QuestionAnswer::Multi(selected));
                                    }
                                } else if cursor_pos == other_idx {
                                    if !custom_text.is_empty() {
                                        break 'widget Some(crate::ui::question::QuestionAnswer::Multi(vec![custom_text.clone()]));
                                    }
                                } else if cursor_pos < n_real {
                                    checked[cursor_pos] = !checked[cursor_pos];
                                }
                            } else if cursor_pos == other_idx {
                                if !custom_text.is_empty() {
                                    break 'widget Some(crate::ui::question::QuestionAnswer::Single(custom_text.clone()));
                                }
                            } else {
                                let label = question.options[cursor_pos].label.clone();
                                break 'widget Some(crate::ui::question::QuestionAnswer::Single(label));
                            }
                        }
                        (KeyCode::Char(c), m) if cursor_pos == other_idx && (m == KeyModifiers::NONE || m == KeyModifiers::SHIFT) => {
                            custom_text.push(c);
                        }
                        _ => {}
                    }
                }
                _ => {}
            }
        };

        self.active_question = None;

        if let Some(ref ans) = answer {
            self.push(RenderLine::QuestionResult {
                header: question.header.to_string(),
                answer: ans.as_str(),
            })?;
        } else {
            self.draw()?; // clear question ui on cancel
        }

        Ok(answer)
    }

    /// Blocking question modal — driven by key events forwarded through `key_rx`.
    ///
    /// Safe to call from `tokio::task::spawn_blocking`.  Does NOT poll the
    /// crossterm event queue directly; instead the tick task forwards
    /// `KeyEvent`s via the `SyncSender` half of the channel.  This avoids the
    /// deadlock where the tick task consumes an Esc from the EventStream while
    /// this function is waiting on `event::read()`.
    ///
    /// Sets `active_question.tx = None` so the tick task's spin-wait branch
    /// is never entered for this modal.
    ///
    /// This is the canonical path for `prompt_approval` and `handle_ask_user_question`.
    pub fn ask_question_blocking(
        &mut self,
        question: &crate::ui::question::Question,
        key_rx: std::sync::mpsc::Receiver<crossterm::event::KeyEvent>,
    ) -> Result<Option<crate::ui::question::QuestionAnswer>> {
        let n_real      = question.options.len();
        let has_other   = question.allow_other;
        let has_submit  = question.multi_select;
        let total_items = n_real + usize::from(has_other) + usize::from(has_submit);
        let other_idx   = if has_other  { n_real } else { usize::MAX };
        let submit_idx  = if has_submit { n_real + usize::from(has_other) } else { usize::MAX };

        let mut cursor_pos:  usize      = 0;
        let mut custom_text: String     = String::new();
        let mut checked:     Vec<bool>  = vec![false; n_real];

        self.scroll = 0;

        let answer: Option<crate::ui::question::QuestionAnswer> = 'widget: loop {
            // Render with tx = None — tick task will not intercept events.
            self.active_question = Some(ActiveQuestionState {
                draw_state: ActiveQuestionDrawState {
                    question:    question.clone(),
                    cursor_pos,
                    custom_text: custom_text.clone(),
                    checked:     checked.clone(),
                    n_real,
                    has_other,
                    has_submit,
                    total_items,
                    other_idx,
                    submit_idx,
                },
                tx: None,   // ← blocking path: no channel needed
                key_tx: None,
            });

            self.draw()?;

            let key_event = match key_rx.recv_timeout(std::time::Duration::from_millis(50)) {
                Ok(k) => k,
                Err(std::sync::mpsc::RecvTimeoutError::Timeout) => continue,
                Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => break 'widget None,
            };
            let crossterm::event::KeyEvent { code, modifiers, .. } = key_event;
            match (code, modifiers) {
                (KeyCode::Esc, _)
                | (KeyCode::Char('c'), KeyModifiers::CONTROL) => {
                    break 'widget None;
                }
                (KeyCode::Up, _) => {
                    if cursor_pos > 0 { cursor_pos -= 1; }
                }
                (KeyCode::Down, _) => {
                    if cursor_pos + 1 < total_items { cursor_pos += 1; }
                }
                (KeyCode::Tab, _) => {
                    cursor_pos = (cursor_pos + 1) % total_items;
                }
                (KeyCode::BackTab, _) => {
                    cursor_pos = if cursor_pos == 0 {
                        total_items - 1
                    } else {
                        cursor_pos - 1
                    };
                }
                (KeyCode::Char(c), KeyModifiers::NONE)
                    if c.is_ascii_digit() && c != '0' =>
                {
                    let idx = (c as usize) - ('1' as usize);
                    if idx < total_items {
                        if question.multi_select {
                            if idx < n_real {
                                checked[idx] = !checked[idx];
                                cursor_pos = idx;
                            }
                        } else if idx != other_idx {
                            let label = question.options[idx].label.clone();
                            break 'widget Some(
                                crate::ui::question::QuestionAnswer::Single(label),
                            );
                        } else {
                            cursor_pos = idx;
                        }
                    }
                }
                (KeyCode::Backspace, _) if cursor_pos == other_idx => {
                    custom_text.pop();
                }
                (KeyCode::Enter, _) => {
                    if question.multi_select {
                        if cursor_pos == submit_idx {
                            let selected: Vec<String> = checked
                                .iter()
                                .enumerate()
                                .filter(|(_, c)| **c)
                                .map(|(i, _)| question.options[i].label.clone())
                                .collect();
                            if !selected.is_empty() {
                                break 'widget Some(
                                    crate::ui::question::QuestionAnswer::Multi(selected),
                                );
                            }
                        } else if cursor_pos == other_idx {
                            if !custom_text.is_empty() {
                                break 'widget Some(
                                    crate::ui::question::QuestionAnswer::Multi(vec![
                                        custom_text.clone(),
                                    ]),
                                );
                            }
                        } else if cursor_pos < n_real {
                            checked[cursor_pos] = !checked[cursor_pos];
                        }
                    } else if cursor_pos == other_idx {
                        if !custom_text.is_empty() {
                            break 'widget Some(
                                crate::ui::question::QuestionAnswer::Single(
                                    custom_text.clone(),
                                ),
                            );
                        }
                    } else {
                        let label = question.options[cursor_pos].label.clone();
                        break 'widget Some(
                            crate::ui::question::QuestionAnswer::Single(label),
                        );
                    }
                }
                (KeyCode::Char(c), m)
                    if cursor_pos == other_idx
                        && (m == KeyModifiers::NONE || m == KeyModifiers::SHIFT) =>
                {
                    custom_text.push(c);
                }
                _ => {}
            }
        };

        self.active_question = None;

        // V-01 respects the user's scroll position during normal streaming, but
        // after a blocking modal the user MUST see the tool result and agent
        // response immediately — they just took an explicit action (approved /
        // denied / answered).  Reset scroll unconditionally so subsequent pushes
        // land in the visible viewport rather than below it.
        self.scroll        = 0;
        self.pending_lines = 0;

        if let Some(ref ans) = answer {
            self.push(RenderLine::QuestionResult {
                header: question.header.clone(),
                answer: ans.as_str(),
            })?;
        } else {
            self.draw()?; // clear overlay on cancel
        }

        Ok(answer)
    }

    /// Async question via oneshot channel.
    ///
    /// ONLY valid when an external event driver (the tick task's spin-wait
    /// loop) is concurrently calling `handle_question_key`.  For tool-call
    /// approval use `ask_question_blocking` via `spawn_blocking` instead.
    #[deprecated(
        note = "Use ask_question_blocking (via spawn_blocking) for prompt_approval. \
                ask_question_async is only safe when the tick-task spin-wait is \
                the sole event driver and no async lock contention can occur."
    )]
    pub fn ask_question_async(
        &mut self,
        question: crate::ui::question::Question,
    ) -> Result<tokio::sync::oneshot::Receiver<Option<crate::ui::question::QuestionAnswer>>> {
        let n_real     = question.options.len();
        let has_other  = question.allow_other;
        let has_submit = question.multi_select;
        let total_items = n_real + usize::from(has_other) + usize::from(has_submit);

        let other_idx  = if has_other  { n_real } else { usize::MAX };
        let submit_idx = if has_submit { n_real + usize::from(has_other) } else { usize::MAX };

        let cursor_pos: usize = 0;
        let custom_text = String::new();
        let checked: Vec<bool> = vec![false; n_real];

        // snap to bottom when asking
        self.scroll = 0;

        let (tx, rx) = tokio::sync::oneshot::channel();

        self.active_question = Some(ActiveQuestionState {
            draw_state: ActiveQuestionDrawState {
                question,
                cursor_pos,
                custom_text,
                checked,
                n_real,
                has_other,
                has_submit,
                total_items,
                other_idx,
                submit_idx,
            },
            tx: Some(tx),
            key_tx: None,
        });

        self.draw()?;
        Ok(rx)
    }

    pub fn handle_question_key(&mut self, k: crossterm::event::KeyEvent) {
        use crossterm::event::{KeyCode, KeyModifiers};
        let mut ans_opt: Option<Option<crate::ui::question::QuestionAnswer>> = None;

        if let Some(aq) = &mut self.active_question {
            let st = &mut aq.draw_state;
            match (k.code, k.modifiers) {
                (KeyCode::Esc, _) | (KeyCode::Char('c'), KeyModifiers::CONTROL) => {
                    ans_opt = Some(None);
                }
                (KeyCode::Up, _) => { if st.cursor_pos > 0 { st.cursor_pos -= 1; } }
                (KeyCode::Down, _) => { if st.cursor_pos + 1 < st.total_items { st.cursor_pos += 1; } }
                (KeyCode::Tab, _) => { st.cursor_pos = (st.cursor_pos + 1) % st.total_items; }
                (KeyCode::BackTab, _) => { st.cursor_pos = if st.cursor_pos == 0 { st.total_items - 1 } else { st.cursor_pos - 1 }; }
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
                            ans_opt = Some(Some(crate::ui::question::QuestionAnswer::Single(label)));
                        } else {
                            st.cursor_pos = idx;
                        }
                    }
                }
                (KeyCode::Backspace, _) if st.cursor_pos == st.other_idx => { st.custom_text.pop(); }
                (KeyCode::Enter, _) => {
                    if st.question.multi_select {
                        if st.cursor_pos == st.submit_idx {
                            let selected: Vec<String> = st.checked.iter().enumerate()
                                .filter(|(_, c)| **c)
                                .map(|(i, _)| st.question.options[i].label.clone())
                                .collect();
                            if !selected.is_empty() {
                                ans_opt = Some(Some(crate::ui::question::QuestionAnswer::Multi(selected)));
                            }
                        } else if st.cursor_pos == st.other_idx {
                            if !st.custom_text.is_empty() {
                                ans_opt = Some(Some(crate::ui::question::QuestionAnswer::Multi(vec![st.custom_text.clone()])));
                            }
                        } else if st.cursor_pos < st.n_real {
                            st.checked[st.cursor_pos] = !st.checked[st.cursor_pos];
                        }
                    } else if st.cursor_pos == st.other_idx {
                        if !st.custom_text.is_empty() {
                            ans_opt = Some(Some(crate::ui::question::QuestionAnswer::Single(st.custom_text.clone())));
                        }
                    } else {
                        let label = st.question.options[st.cursor_pos].label.clone();
                        ans_opt = Some(Some(crate::ui::question::QuestionAnswer::Single(label)));
                    }
                }
                (KeyCode::Char(c), m) if st.cursor_pos == st.other_idx && (m == KeyModifiers::NONE || m == KeyModifiers::SHIFT) => {
                    st.custom_text.push(c);
                }
                _ => {}
            }
        }

        if let Some(ans) = ans_opt {
            if let Some(mut aq) = self.active_question.take() {
                if let Some(tx) = aq.tx.take() {
                    let _ = tx.send(ans.clone());
                }
                if let Some(ref a) = ans {
                    let _ = self.push(RenderLine::QuestionResult {
                        header: aq.draw_state.question.header.clone(),
                        answer: a.as_str(),
                    });
                } else {
                    let _ = self.draw(); // clear question ui on cancel
                }
            }
        } else {
            let _ = self.draw();
        }
    }

    // ── Input loop ────────────────────────────────────────────────────────

    /// Block until the user submits input or presses Ctrl+D.
    /// Returns `None` on Ctrl+D (exit signal).
    pub fn read_input(
        &mut self,
        history:  &mut Vec<String>,
        hist_idx: &mut Option<usize>,
    ) -> Result<Option<String>> {
        self.input.clear();
        self.cursor_pos = 0;
        *hist_idx = None;

        loop {
            self.draw()?;
            // 50 ms poll: allows animation ticks without burning CPU.
            if !event::poll(std::time::Duration::from_millis(50))? {
                continue;
            }
            match event::read()? {
                Event::Key(k) => {
                    if self.skills_overlay.is_some() {
                        self.handle_skills_key(k);
                    } else if self.active_question.is_some() {
                        self.handle_question_key(k);
                    } else if let Some(result) = self.handle_key_input(k, history, hist_idx)? {
                        return Ok(result);
                    }
                }
                Event::Resize(_, _) => { /* ratatui picks up resize on next draw */ }
                Event::Mouse(m) => {
                    match m.kind {
                        MouseEventKind::ScrollUp   => { self.scroll = self.scroll.saturating_add(3); }
                        MouseEventKind::ScrollDown => {
                            self.scroll = self.scroll.saturating_sub(3);
                            if self.scroll == 0 {
                                self.pending_lines = 0;
                            }
                        }
                        _ => {}
                    }
                }
                _ => {}
            }
        }
    }

    fn handle_key_input(
        &mut self,
        k:        KeyEvent,
        history:  &mut Vec<String>,
        hist_idx: &mut Option<usize>,
    ) -> Result<Option<Option<String>>> {
        // Some(None)        = Ctrl+D (exit)
        // Some(Some(s))     = line submitted
        // None              = continue reading

        // ── A-01: file picker routing ──────────────────────────────────────
        if self.picker.is_some() {
            match (k.code, k.modifiers) {
                (KeyCode::Esc, _) => {
                    self.picker = None;
                }
                (KeyCode::Up, _) => {
                    if let Some(ref mut pk) = self.picker {
                        if pk.cursor > 0 { pk.cursor -= 1; }
                    }
                }
                (KeyCode::Down, _) => {
                    if let Some(ref mut pk) = self.picker {
                        if !pk.matches.is_empty() && pk.cursor + 1 < pk.matches.len() {
                            pk.cursor += 1;
                        }
                    }
                }
                (KeyCode::Enter, m) if m == KeyModifiers::NONE => {
                    if let Some(pk) = self.picker.take() {
                        if let Some(selected) = pk.matches.get(pk.cursor).cloned() {
                            let query_end = pk.at_pos + 1 + pk.query.len();
                            self.input.drain(pk.at_pos..query_end.min(self.input.len()));
                            self.input.insert_str(pk.at_pos, &selected);
                            self.cursor_pos = pk.at_pos + selected.len();
                        }
                        // dismiss whether or not a match was selected
                    }
                }
                (KeyCode::Backspace, _) => {
                    if let Some(ref mut pk) = self.picker {
                        if pk.query.is_empty() {
                            // Delete the @ and dismiss
                            if pk.at_pos < self.input.len() {
                                self.input.remove(pk.at_pos);
                                self.cursor_pos = pk.at_pos;
                            }
                            self.picker = None;
                        } else {
                            // Remove last query char from both query and input
                            let query_end = pk.at_pos + 1 + pk.query.len();
                            let remove_at = query_end.saturating_sub(1);
                            if remove_at < self.input.len() {
                                self.input.remove(remove_at);
                            }
                            pk.query.pop();
                            pk.cursor = 0;
                            let root = std::env::current_dir().unwrap_or_default();
                            pk.matches = collect_files(&root, &pk.query);
                        }
                    }
                }
                (KeyCode::Char(c), m)
                    if m == KeyModifiers::NONE || m == KeyModifiers::SHIFT =>
                {
                    // Append char to both input and picker query
                    if let Some(ref mut pk) = self.picker {
                        let query_end = pk.at_pos + 1 + pk.query.len();
                        self.input.insert(query_end, c);
                        self.cursor_pos = query_end + c.len_utf8();
                        pk.query.push(c);
                        pk.cursor = 0;
                        let root = std::env::current_dir().unwrap_or_default();
                        pk.matches = collect_files(&root, &pk.query);
                    }
                }
                _ => {}
            }
            let _ = self.draw();
            return Ok(None);
        }

        match (k.code, k.modifiers) {
            // ── Submit ────────────────────────────────────────────────────
            // Alt+Enter is the universal cross-terminal newline (reliably
            // transmitted by all terminals).  Shift+Enter works only in
            // terminals that support the kitty keyboard protocol; we accept
            // both so users on either kind of terminal are covered.
            (KeyCode::Enter, m)
                if m == KeyModifiers::ALT
                || m == KeyModifiers::SHIFT
                || m == (KeyModifiers::SHIFT | KeyModifiers::ALT) =>
            {
                self.input.insert(self.cursor_pos, '\n');
                self.cursor_pos += 1;
            }
            (KeyCode::Enter, _) => {
                let line = self.input.clone();
                self.input.clear();   // clear input immediately so it's empty during agent turn
                self.cursor_pos = 0;
                self.scroll = 0;        // snap to bottom on submit
                self.pending_lines = 0; // user is following the conversation
                return Ok(Some(Some(line)));
            }

            // ── Exit ──────────────────────────────────────────────────────
            (KeyCode::Char('d'), KeyModifiers::CONTROL) if self.input.is_empty() => {
                return Ok(Some(None));
            }

            // ── Cancel / clear ────────────────────────────────────────────
            (KeyCode::Char('c'), KeyModifiers::CONTROL) => {
                self.input.clear();
                self.cursor_pos = 0;
                return Ok(Some(Some(String::new())));
            }
            (KeyCode::Esc, _) => {
                self.input.clear();
                self.cursor_pos = 0;
            }

            // ── Edit shortcuts ────────────────────────────────────────────
            (KeyCode::Char('u'), KeyModifiers::CONTROL) => {
                self.input.drain(..self.cursor_pos);
                self.cursor_pos = 0;
            }
            (KeyCode::Char('w'), KeyModifiers::CONTROL) => {
                let end   = self.cursor_pos;
                let start = self.input[..end]
                    .rfind(|c: char| !c.is_whitespace())
                    .and_then(|p| self.input[..p].rfind(char::is_whitespace).map(|q| q + 1))
                    .unwrap_or(0);
                self.input.drain(start..end);
                self.cursor_pos = start;
            }
            (KeyCode::Home, _) | (KeyCode::Char('a'), KeyModifiers::CONTROL) => {
                self.cursor_pos = 0;
            }
            (KeyCode::End, _) | (KeyCode::Char('e'), KeyModifiers::CONTROL) => {
                self.cursor_pos = self.input.len();
            }

            // ── Cursor movement ───────────────────────────────────────────
            (KeyCode::Left, _) if self.cursor_pos > 0 => {
                self.cursor_pos -= self.input[..self.cursor_pos]
                    .chars().last().map(|c| c.len_utf8()).unwrap_or(1);
            }
            (KeyCode::Right, _) if self.cursor_pos < self.input.len() => {
                self.cursor_pos += self.input[self.cursor_pos..]
                    .chars().next().map(|c| c.len_utf8()).unwrap_or(1);
            }

            // ── History / cursor-up ───────────────────────────────────────
            // When the cursor is NOT on the first visual row of the input,
            // Up/Down move the cursor one visual row up/down within the
            // multiline buffer.  Only when already on the first/last visual
            // row do we switch to history navigation.
            (KeyCode::Up, _) => {
                let available_w = self.term_width.saturating_sub(2).max(1);
                let text_w = (available_w.saturating_sub(2).max(1)) as usize;
                let before  = &self.input[..self.cursor_pos];
                let (cur_row, cur_col) = calc_visual_cursor(before, available_w);

                if cur_row == 0 {
                    // Already on the first visual row → history navigation
                    if !history.is_empty() {
                        let new_idx = match *hist_idx {
                            None        => history.len() - 1,
                            Some(i) if i > 0 => i - 1,
                            Some(i)     => i,
                        };
                        *hist_idx       = Some(new_idx);
                        self.input      = history[new_idx].clone();
                        self.cursor_pos = self.input.len();
                    }
                } else {
                    // Move cursor up one visual row: target column = cur_col
                    // Walk backwards through the byte string to find the char
                    // at (cur_row-1, cur_col).
                    let target_row = cur_row - 1;
                    // Rebuild visual-row byte-offset map
                    let new_pos = find_cursor_at_visual_row_col(
                        &self.input, text_w, target_row, cur_col,
                    );
                    self.cursor_pos = new_pos;
                }
            }
            (KeyCode::Down, _) => {
                let available_w = self.term_width.saturating_sub(2).max(1);
                let text_w = (available_w.saturating_sub(2).max(1)) as usize;
                let total_rows = {
                    let (tr, _) = calc_visual_cursor(&self.input, available_w);
                    tr
                };
                let before = &self.input[..self.cursor_pos];
                let (cur_row, cur_col) = calc_visual_cursor(before, available_w);

                if cur_row >= total_rows {
                    // Already on the last visual row → history navigation
                    if let Some(i) = *hist_idx {
                        if i + 1 < history.len() {
                            *hist_idx = Some(i + 1);
                            self.input = history[i + 1].clone();
                            self.cursor_pos = self.input.len();
                        } else {
                            *hist_idx = None;
                            self.input.clear();
                            self.cursor_pos = 0;
                        }
                    }
                } else {
                    let target_row = cur_row + 1;
                    let new_pos = find_cursor_at_visual_row_col(
                        &self.input, text_w, target_row, cur_col,
                    );
                    self.cursor_pos = new_pos;
                }
            }

            // ── Content scroll ────────────────────────────────────────────
            // Shift+K = up 10 rows,  Shift+J = down 10 rows
            (KeyCode::Char('K'), _) => {
                self.scroll = self.scroll.saturating_add(10);
            }
            (KeyCode::Char('J'), _) => {
                self.scroll = self.scroll.saturating_sub(10);
                if self.scroll == 0 {
                    self.pending_lines = 0;
                }
            }

            // ── Mode cycle / path completion ──────────────────────────────
            (KeyCode::Tab, _) => {
                // I-02: if cursor is on a path token, complete it; otherwise
                // fall through to the mode-cycle sentinel.
                if let Some((new_input, new_cursor)) = complete_path(&self.input, self.cursor_pos) {
                    self.input      = new_input;
                    self.cursor_pos = new_cursor;
                } else {
                    self.scroll = 0;
                    return Ok(Some(Some("__TAB__".to_string())));
                }
            }
            (KeyCode::BackTab, _) => {
                self.scroll = 0;
                return Ok(Some(Some("__BACKTAB__".to_string())));
            }

            // ── Expand/Collapse Tool Outputs ──────────────────────────────
            (KeyCode::Char('o'), KeyModifiers::CONTROL) => {
                self.expand_all = !self.expand_all;
            }

            // ── Editing ───────────────────────────────────────────────────
            (KeyCode::Backspace, _) if self.cursor_pos > 0 => {
                let char_len = self.input[..self.cursor_pos]
                    .chars().last().map(|c| c.len_utf8()).unwrap_or(1);
                self.cursor_pos -= char_len;
                self.input.remove(self.cursor_pos);
            }
            (KeyCode::Delete, _) if self.cursor_pos < self.input.len() => {
                self.input.remove(self.cursor_pos);
            }
            (KeyCode::Char(c), m)
                if m == KeyModifiers::NONE || m == KeyModifiers::SHIFT =>
            {
                let pos = self.cursor_pos;
                self.input.insert(pos, c);
                self.cursor_pos = pos + c.len_utf8();
                // A-01: activate file picker when '@' is typed.
                if c == '@' && self.picker.is_none() {
                    let root = std::env::current_dir().unwrap_or_default();
                    let matches = collect_files(&root, "");
                    self.picker = Some(PickerState {
                        at_pos:  pos,
                        query:   String::new(),
                        matches,
                        cursor:  0,
                    });
                }
            }
            _ => {}
        }
        Ok(None)
    }

    fn handle_skills_key(&mut self, key: crossterm::event::KeyEvent) {
        use crossterm::event::{KeyCode, KeyModifiers};
        let ov = match self.skills_overlay.as_mut() { Some(o) => o, None => return };

        match ov.mode {
            SkillsMode::List => match (key.code, key.modifiers) {
                (KeyCode::Char('j'), _) | (KeyCode::Down, _) => {
                    if ov.cursor + 1 < ov.skills.len() { ov.cursor += 1; }
                    let visible = 8usize;
                    if ov.cursor >= ov.list_scroll + visible { ov.list_scroll += 1; }
                }
                (KeyCode::Char('k'), _) | (KeyCode::Up, _) => {
                    if ov.cursor > 0 { ov.cursor -= 1; }
                    if ov.cursor < ov.list_scroll { ov.list_scroll = ov.list_scroll.saturating_sub(1); }
                }
                (KeyCode::Enter, _) => {
                    if !ov.skills.is_empty() {
                        ov.mode = SkillsMode::Detail;
                        ov.detail_scroll = 0;
                    }
                }
                (KeyCode::Char('e'), _) => {
                    if !ov.skills.is_empty() {
                        ov.load_edit_fields();
                        ov.mode = SkillsMode::Edit;
                    }
                }
                (KeyCode::Esc, _) | (KeyCode::Char('q'), _) => {
                    self.skills_overlay = None;
                }
                _ => {}
            },

            SkillsMode::Detail => match (key.code, key.modifiers) {
                (KeyCode::Char('j'), _) | (KeyCode::Down, _) => { ov.detail_scroll += 1; }
                (KeyCode::Char('k'), _) | (KeyCode::Up, _) => { ov.detail_scroll = ov.detail_scroll.saturating_sub(1); }
                (KeyCode::Char('e'), _) => {
                    ov.load_edit_fields();
                    ov.mode = SkillsMode::Edit;
                }
                (KeyCode::Esc, _) => { ov.mode = SkillsMode::List; }
                _ => {}
            },

            SkillsMode::Edit => match (key.code, key.modifiers) {
                (KeyCode::Char('s'), KeyModifiers::CONTROL) => {
                    let skill = match ov.skills.get(ov.cursor) { Some(s) => s.clone(), None => return };
                    let fields = ov.edit_fields.clone();
                    match crate::skills::write_skill_to_disk(&skill, &fields) {
                        Ok(_) => {
                            ov.dirty = false;
                            ov.mode = SkillsMode::Detail;
                        }
                        Err(e) => {
                            self.lines.push(crate::ui::RenderLine::ErrorMsg(format!("Failed to save: {e}")));
                        }
                    }
                }
                (KeyCode::Esc, _) => {
                    ov.dirty = false;
                    ov.mode = SkillsMode::Detail;
                }
                (KeyCode::Tab, _) => {
                    ov.field_cursor = (ov.field_cursor + 1) % 6;
                    ov.field_pos = ov.edit_fields.get(ov.field_cursor).map(|f| f.len()).unwrap_or(0);
                }
                (KeyCode::BackTab, _) => {
                    ov.field_cursor = (ov.field_cursor + 5) % 6;
                    ov.field_pos = ov.edit_fields.get(ov.field_cursor).map(|f| f.len()).unwrap_or(0);
                }
                (KeyCode::Enter, _) => {
                    if ov.field_cursor == 5 {
                        let pos = ov.field_pos;
                        if let Some(f) = ov.edit_fields.get_mut(5) {
                            let pos = pos.min(f.len());
                            f.insert(pos, '\n');
                            ov.field_pos = pos + 1;
                            ov.dirty = true;
                        }
                    } else {
                        ov.field_cursor = (ov.field_cursor + 1) % 6;
                        ov.field_pos = ov.edit_fields.get(ov.field_cursor).map(|f| f.len()).unwrap_or(0);
                    }
                }
                (KeyCode::Left, _) => { if ov.field_pos > 0 { ov.field_pos -= 1; } }
                (KeyCode::Right, _) => {
                    let max = ov.edit_fields.get(ov.field_cursor).map(|f| f.len()).unwrap_or(0);
                    if ov.field_pos < max { ov.field_pos += 1; }
                }
                (KeyCode::Up, _) if ov.field_cursor == 5 => {
                    ov.detail_scroll = ov.detail_scroll.saturating_sub(1);
                }
                (KeyCode::Down, _) if ov.field_cursor == 5 => {
                    ov.detail_scroll += 1;
                }
                (KeyCode::Backspace, _) => {
                    let pos = ov.field_pos;
                    if pos > 0 {
                        if let Some(f) = ov.edit_fields.get_mut(ov.field_cursor) {
                            let new_pos = f[..pos].char_indices().next_back().map(|(i, _)| i).unwrap_or(0);
                            f.drain(new_pos..pos);
                            ov.field_pos = new_pos;
                            ov.dirty = true;
                        }
                    }
                }
                (KeyCode::Char(c), m) if m == KeyModifiers::NONE || m == KeyModifiers::SHIFT => {
                    let pos = ov.field_pos;
                    if let Some(f) = ov.edit_fields.get_mut(ov.field_cursor) {
                        let pos = pos.min(f.len());
                        f.insert(pos, c);
                        ov.field_pos = pos + c.len_utf8();
                        ov.dirty = true;
                    }
                }
                _ => {}
            },
        }
        let _ = self.draw();
    }
}

impl Drop for TuiApp {
    fn drop(&mut self) {
        if supports_keyboard_enhancement().unwrap_or(false) {
            let _ = crossterm::execute!(std::io::stdout(), PopKeyboardEnhancementFlags);
        }
        let _ = crossterm::execute!(std::io::stdout(), DisableMouseCapture);
        ratatui::restore();
    }
}

// ── Scroll helpers ────────────────────────────────────────────────────────────

/// Count the number of visual (terminal) rows a single `Line` occupies when
/// word-wrapped to `content_w` columns.  Uses unicode display-width so emoji
/// and CJK characters are measured correctly.
///
/// Matches ratatui's `WordWrapper` behaviour: words are broken on whitespace;
/// a word that would overflow the current row starts a new row.
fn count_wrapped_rows(line: &Line<'_>, content_w: u16) -> u16 {
    if content_w == 0 { return 1; }
    // Concatenate all spans into a single string for word counting.
    let text: String = line.spans.iter().map(|s| s.content.as_ref()).collect();
    if text.is_empty() { return 1; }
    // V-03: split on \n first — each newline forces a new visual row regardless
    // of wrapping, matching ratatui's behaviour for embedded newlines in spans.
    text.split('\n')
        .map(|segment| count_wrapped_segment(segment, content_w))
        .sum::<u16>()
        .max(1)
}

/// Count wrapped rows for a single line segment (no embedded newlines).
fn count_wrapped_segment(text: &str, content_w: u16) -> u16 {
    if text.is_empty() { return 1; }
    let width = content_w as usize;
    let mut rows: u16 = 1;
    let mut row_w: usize = 0;
    // split_inclusive preserves the trailing space/tab on each "word" token,
    // which keeps the total width calculation correct.
    for word in text.split_inclusive(|c: char| c == ' ' || c == '\t') {
        let word_w = UnicodeWidthStr::width(word);
        if row_w > 0 && row_w + word_w > width {
            rows += 1;
            row_w = word_w;
        } else {
            row_w += word_w;
        }
    }
    rows
}

// ── Frame renderer ────────────────────────────────────────────────────────────

#[allow(clippy::too_many_arguments)]
fn render_frame(
    frame:            &mut Frame,
    lines:            &[RenderLine],
    streaming:        Option<&str>,
    scroll:           usize,
    expand_all:       bool,
    input:            &str,
    cursor_pos:       usize,
    mode:             PermissionMode,
    agent_name:       &str,
    model:            &str,
    last_status:      &Option<String>,
    thinking_text:    Option<&str>,
    thinking_elapsed: Option<std::time::Duration>,
    active_question:  Option<&ActiveQuestionDrawState>,
    pending_lines:    usize,
    queued_count:     usize,
    cwd:              &str,
    context_pct:      Option<u8>,
    picker:           Option<&PickerState>,
    header_lines:     &[RenderLine],
    footer_extra:     Option<&str>,
    skills_overlay:   Option<&SkillsOverlayState>,
) -> u16 {   // returns max_skip for V-04 scroll clamping
    let area = frame.area();
    let w    = area.width as usize;

    let available_w      = area.width.saturating_sub(2).max(1);
    let input_rows       = calc_input_rows(input, available_w).clamp(1, MAX_INPUT_ROWS);
    // A-02: footer_extra adds one row below the normal footer when present.
    let footer_extra_h: u16 = if footer_extra.is_some() { 1 } else { 0 };
    let bottom_rows      = FIXED_ROWS + input_rows + footer_extra_h;

    if area.height <= bottom_rows + 1 {
        frame.render_widget(Paragraph::new("Terminal too small"), area);
        return 0;
    }

    let content_height = area.height - bottom_rows;

    // When a question is active, carve the inline panel out of the content area.
    // Layout becomes 8 slots; without a question it stays 6 (question slots = 0).
    let inline_h = active_question
        .map(|aq| question_height(aq, content_height))
        .unwrap_or(0);
    let shrunk_content = content_height.saturating_sub(inline_h);

    let chunks = if inline_h > 0 {
        Layout::vertical([
            Constraint::Length(shrunk_content), // [0] content  (shrunk)
            Constraint::Length(1),              // [1] inline separator ╌╌╌
            Constraint::Length(inline_h - 1),   // [2] question panel
            Constraint::Length(1),              // [3] status
            Constraint::Length(1),              // [4] top separator
            Constraint::Length(input_rows),     // [5] input
            Constraint::Length(1),              // [6] bottom separator
            Constraint::Length(1),              // [7] footer
        ])
        .split(area)
    } else {
        // No question: same 6-slot layout, pad with two dummy zero-height slots
        // so all index references below are uniform (we only use 0,3..7 in this branch).
        Layout::vertical([
            Constraint::Length(content_height), // [0] content
            Constraint::Length(0),              // [1] (unused)
            Constraint::Length(0),              // [2] (unused)
            Constraint::Length(1),              // [3] status
            Constraint::Length(1),              // [4] top separator
            Constraint::Length(input_rows),     // [5] input
            Constraint::Length(1),              // [6] bottom separator
            Constraint::Length(1),              // [7] footer
        ])
        .split(area)
    };

    // ── A-02: Header strip — pinned above the scrollable messages pane ───────
    let content_w = area.width.saturating_sub(0).max(1);
    let (header_area_opt, messages_area) = {
        let mut header_text: Vec<Line<'static>> = Vec::new();
        for rl in header_lines {
            render_line_to_text(rl, w, false, &mut header_text);
        }
        if header_text.is_empty() {
            (None, chunks[0])
        } else {
            let hh: u16 = header_text.iter()
                .map(|l| count_wrapped_rows(l, content_w))
                .sum::<u16>()
                .min(chunks[0].height / 3)
                .max(1);
            let split = Layout::vertical([
                Constraint::Length(hh),
                Constraint::Min(0),
            ]).split(chunks[0]);
            // Render the pinned header now (before message rendering).
            frame.render_widget(
                Paragraph::new(header_text).wrap(Wrap { trim: false }),
                split[0],
            );
            (Some(split[0]), split[1])
        }
    };
    let _ = header_area_opt; // used above for rendering

    // ── Content area ─────────────────────────────────────────────────────────
    let mut text_lines: Vec<Line<'static>> = Vec::new();
    for rl in lines {
        render_line_to_text(rl, w, expand_all, &mut text_lines);
    }
    if let Some(s) = streaming {
        render_assistant_lines(s, w, &mut text_lines);
    }

    // Count visual rows (word-wrap at content width, matching ratatui's WordWrapper).
    let total_visual: u16 = text_lines.iter()
        .map(|l| count_wrapped_rows(l, content_w))
        .sum();

    // V-04 / A-02: use messages_area height (excludes pinned header).
    let messages_h = messages_area.height;
    let visible  = messages_h.saturating_sub(CONTENT_PAD_TOP + CONTENT_PAD_BOT);
    let max_skip = if total_visual > visible { total_visual - visible } else { 0 };
    let effective_up = (scroll as u16).min(max_skip);
    let para_scroll  = max_skip - effective_up;

    frame.render_widget(
        Paragraph::new(text_lines)
            .block(Block::new().padding(Padding::vertical(1)))
            .wrap(Wrap { trim: false })
            .scroll((para_scroll, 0)),
        messages_area,
    );

    // ── A-01: File picker overlay ─────────────────────────────────────────────
    if let Some(pk) = picker {
        let n = pk.matches.len().min(6);
        let picker_h = ((2 + n) as u16).clamp(2, messages_area.height.saturating_sub(1));
        let picker_rect = ratatui::layout::Rect {
            x:      messages_area.x,
            y:      messages_area.y + messages_area.height.saturating_sub(picker_h),
            width:  messages_area.width,
            height: picker_h,
        };
        render_picker(frame, pk, picker_rect);
    }

    // ── Status row ────────────────────────────────────────────────────────────
    let (status_text, status_style) = if let Some(elapsed) = thinking_elapsed {
        let text = thinking_text.unwrap_or("thinking…");
        let ms = elapsed.as_millis();
        
        // Dynamic spinner selection based on elapsed time or just variety
        let spinner = if (ms / 3000) % 2 == 0 {
            BRAILLE[(ms / 80) as usize % BRAILLE.len()]
        } else {
            DOTS[(ms / 100) as usize % DOTS.len()]
        };

        // Pulse through bright-cyan shades (~400ms per step)
        let palette: &[(u8, u8, u8)] = &[
            (80,  190, 255),
            (120, 215, 255),
            (160, 235, 255),
            (100, 200, 255),
        ];
        let (r, g, b) = palette[(ms / 400) as usize % palette.len()];
        
        (
            format!("{spinner} {text}"),
            Style::default().fg(RC::Rgb(r, g, b)).add_modifier(Modifier::BOLD),
        )
    } else if let Some(s) = last_status {
        (
            s.clone(),
            Style::default().fg(RC::Rgb(100, 170, 120)).add_modifier(Modifier::DIM),
        )
    } else {
        (String::new(), Style::default())
    };

    // Append queued-message badge so the user knows their input was accepted.
    let status_text = if queued_count > 0 {
        format!("{status_text}  · {queued_count} queued")
    } else {
        status_text
    };

    // V-02: Append scroll indicator when user is scrolled up and content is arriving.
    let status_text = if scroll > 0 {
        let hint = if streaming.is_some() {
            "  ↓ streaming…  (Shift+J to follow)".to_string()
        } else if pending_lines > 0 {
            format!("  ↓ {pending_lines} new  (Shift+J to follow)")
        } else {
            String::new()
        };
        if hint.is_empty() { status_text } else { format!("{status_text}{hint}") }
    } else {
        status_text
    };

    frame.render_widget(
        Paragraph::new(Span::styled(status_text, status_style)),
        chunks[3],
    );

    // ── Separators ────────────────────────────────────────────────────────────
    // U-02: Top separator pulses cyan when the agent is thinking or streaming,
    // giving a peripheral activity signal without cluttering the status bar.
    // Bottom separator always uses the mode color (stable reference point).
    let mode_color = mode_sep_color(mode);
    let top_sep_color = if let Some(elapsed) = thinking_elapsed {
        // Thinking / tool-calling: animated cyan pulse matching the spinner.
        let ms = elapsed.as_millis();
        let palette: &[(u8, u8, u8)] = &[
            (80,  190, 255),
            (120, 215, 255),
            (160, 235, 255),
            (100, 200, 255),
        ];
        let (r, g, b) = palette[(ms / 400) as usize % palette.len()];
        RC::Rgb(r, g, b)
    } else if streaming.is_some() {
        // Pure text streaming (thinking animation already stopped): fixed bright cyan.
        RC::Rgb(80, 190, 255)
    } else {
        mode_color
    };
    let sep = "─".repeat(area.width as usize);
    frame.render_widget(
        Paragraph::new(Span::styled(sep.clone(), Style::default().fg(top_sep_color))),
        chunks[4],
    );
    frame.render_widget(
        Paragraph::new(Span::styled(sep, Style::default().fg(mode_color))),
        chunks[6],
    );

    // ── Input area ────────────────────────────────────────────────────────────
    // Build one ratatui Line per logical line so wrapping is correct and the
    // "> " prefix only appears on the first line.  Subsequent lines get a
    // "  " (2-space) indent so text columns align with the first line.
    let input_placeholder = if queued_count > 0 {
        format!("{queued_count} queued — type another or Ctrl+Enter to redirect")
    } else {
        "Type a message…".to_string()
    };
    let input_paragraph: Vec<Line<'static>> = if input.is_empty() {
        vec![Line::from(vec![
            Span::styled("> ", Style::default().fg(RC::White)),
            Span::styled(input_placeholder, Style::default().fg(RC::DarkGray)),
        ])]
    } else {
        input
            .split('\n')
            .enumerate()
            .map(|(i, seg)| {
                let prefix = if i == 0 { "> " } else { "  " };
                Line::from(vec![
                    Span::styled(prefix, Style::default().fg(RC::Rgb(120, 120, 120))),
                    Span::styled(seg.to_string(), Style::default().fg(RC::White)),
                ])
            })
            .collect()
    };
    frame.render_widget(
        Paragraph::new(input_paragraph).wrap(Wrap { trim: false }),
        chunks[5],
    );

    // Cursor position
    let before = &input[..cursor_pos.min(input.len())];
    let (vis_row, vis_col) = calc_visual_cursor(before, available_w);
    let cx = (chunks[5].x + vis_col).min(chunks[5].x + chunks[5].width.saturating_sub(1));
    let cy = (chunks[5].y + vis_row).min(chunks[5].y + chunks[5].height.saturating_sub(1));
    frame.set_cursor_position((cx, cy));

    // ── Footer ────────────────────────────────────────────────────────────────
    let (left_label, left_glyph, left_color) = mode_footer_left(mode);
    let right_agent = agent_name.to_string();
    let right_model  = format!(" [{}]", truncate_str(model, 30));
    let right_ctx    = context_pct.map(|p| format!(" {p}%")).unwrap_or_default();
    // CWD segment — shown in the centre of the footer in dark gray
    let mid_cwd      = format!("  {cwd}  ");

    let left_base_len: u16 = left_label.chars().count() as u16
        + if left_glyph.is_empty() { 0 } else { 1 + left_glyph.chars().count() as u16 };
    let right_len: u16 = (mid_cwd.chars().count()
        + right_agent.chars().count()
        + right_model.chars().count()
        + right_ctx.chars().count()) as u16;
    let pad = chunks[7].width.saturating_sub(left_base_len + right_len) as usize;

    let mut footer: Vec<Span<'static>> = vec![
        Span::styled(left_label, Style::default().fg(left_color).add_modifier(Modifier::BOLD)),
    ];
    if !left_glyph.is_empty() {
        footer.push(Span::styled(
            format!(" {left_glyph}"),
            Style::default().fg(left_color),
        ));
    }
    footer.push(Span::raw(" ".repeat(pad)));
    footer.push(Span::styled(mid_cwd,    Style::default().fg(RC::Rgb(90, 90, 90))));
    footer.push(Span::styled(right_agent, Style::default().fg(RC::Rgb(140, 140, 249))));
    footer.push(Span::styled(right_model, Style::default().fg(RC::DarkGray)));
    if !right_ctx.is_empty() {
        footer.push(Span::styled(right_ctx, Style::default().fg(RC::Rgb(90, 90, 90))));
    }

    frame.render_widget(Paragraph::new(Line::from(footer)), chunks[7]);

    // ── A-02: Footer extra row ────────────────────────────────────────────────
    if let Some(extra) = footer_extra {
        let extra_rect = ratatui::layout::Rect {
            x:      chunks[7].x,
            y:      chunks[7].y + 1,
            width:  chunks[7].width,
            height: 1,
        };
        frame.render_widget(
            Paragraph::new(Span::styled(
                extra.to_string(),
                Style::default().fg(RC::Rgb(80, 80, 80)),
            )),
            extra_rect,
        );
    }

    // ── Inline question panel (anchored to bottom of content viewport) ────────
    if let Some(aq) = active_question {
        // chunks[1] = dashed separator, chunks[2] = panel body
        render_question_inline(frame, aq, chunks[1], chunks[2]);
    }

    // ── Skills overlay (full-screen, drawn last so it covers everything) ─────
    if let Some(ov) = skills_overlay {
        render_skills_overlay(frame, ov, area);
    }

    max_skip  // V-04: returned so draw_impl can clamp self.scroll
}

// ── Overlay helpers ───────────────────────────────────────────────────────────

/// Calculate the number of rows needed for the inline question panel.
///
/// Counts: 1 header + 1 blank + wrapped-question-rows + 1 blank
///       + per-option rows (label + optional description)
///       + submit row (multi-select) + other row + 1 blank + 1 hint.
/// Clamped to at most half the content viewport so content is never fully hidden.
fn question_height(aq: &ActiveQuestionDrawState, content_height: u16) -> u16 {
    let q = &aq.question;

    // Fixed rows: separator-row is accounted for by the caller (inline_h - 1 for body).
    // Here we return the total including the separator row.
    let mut rows: u16 = 0;

    // header chip + blank
    rows += 2;
    // question text (treat as 1 row; long questions word-wrap but we keep it simple)
    rows += 1;
    // blank after question
    rows += 1;

    // progress indicator
    if q.progress.is_some() {
        rows += 2; // "Question N of M" + blank
    }

    // options: label row always, description row only if non-empty
    for idx in 0..aq.total_items {
        if idx == aq.submit_idx {
            rows += 2; // label + blank
        } else if idx == aq.other_idx {
            rows += 2; // label + blank
        } else {
            rows += 1; // label
            if idx < q.options.len() && !q.options[idx].description.is_empty() {
                rows += 1; // description
            }
        }
    }

    // blank + hint
    rows += 2;

    // +1 for the dashed separator row itself
    rows += 1;

    rows.min(content_height / 2).max(6)
}

/// Render the inline question panel — no border box, anchored to the bottom
/// of the content viewport via the layout split in `render_frame`.
///
/// `sep_area`  — the single row reserved for the dashed separator (chunks[1]).
/// `body_area` — the panel body rows (chunks[2]).
fn render_question_inline(
    frame:     &mut Frame,
    aq:        &ActiveQuestionDrawState,
    sep_area:  Rect,
    body_area: Rect,
) {
    let q = &aq.question;

    // ── Dashed separator ─────────────────────────────────────────────────────
    // Use a dimmer, shorter dash to visually distinguish from the hard ─ separators.
    let dash_w   = sep_area.width as usize;
    let dash_str = "╌".repeat(dash_w);
    frame.render_widget(
        Paragraph::new(Line::from(Span::styled(
            dash_str,
            Style::default().fg(RC::Rgb(70, 70, 100)),
        ))),
        sep_area,
    );

    // ── Panel body ───────────────────────────────────────────────────────────
    let mut lines: Vec<Line<'static>> = Vec::new();

    // Header chip — left-aligned, yellow bold with a diamond glyph
    lines.push(Line::from(vec![
        Span::styled("◆ ", Style::default().fg(RC::Yellow)),
        Span::styled(
            q.header.clone(),
            Style::default().fg(RC::Yellow).add_modifier(Modifier::BOLD),
        ),
    ]));
    lines.push(Line::from(""));

    // Question text
    lines.push(Line::from(Span::styled(
        q.text.clone(),
        Style::default().fg(RC::White),
    )));
    lines.push(Line::from(""));

    // Progress indicator
    if let Some((cur, tot)) = q.progress {
        lines.push(Line::from(Span::styled(
            format!("Question {cur} of {tot}"),
            Style::default().fg(RC::DarkGray),
        )));
        lines.push(Line::from(""));
    }

    // Options
    for idx in 0..aq.total_items {
        let is_selected = aq.cursor_pos == idx;
        let selector    = if is_selected { "❯" } else { " " };

        // Submit item (multi-select only)
        if idx == aq.submit_idx {
            let style = if is_selected {
                Style::default().fg(RC::Green).add_modifier(Modifier::BOLD)
            } else {
                Style::default().fg(RC::DarkGray)
            };
            lines.push(Line::from(Span::styled(
                format!(" {selector} {}.  Submit", idx + 1),
                style,
            )));
            lines.push(Line::from(""));
            continue;
        }

        // Free-text "Other" item
        if idx == aq.other_idx {
            let display = if is_selected {
                if aq.custom_text.is_empty() {
                    "Type something.█".to_string()
                } else {
                    format!("{}█", aq.custom_text)
                }
            } else if !aq.custom_text.is_empty() {
                aq.custom_text.clone()
            } else {
                "Type something.".to_string()
            };
            lines.push(Line::from(vec![
                Span::styled(
                    format!(" {selector} {}.  ", idx + 1),
                    Style::default().fg(if is_selected { RC::Green } else { RC::DarkGray }),
                ),
                Span::styled(
                    display,
                    Style::default().fg(RC::DarkGray).add_modifier(Modifier::ITALIC),
                ),
            ]));
            lines.push(Line::from(""));
            continue;
        }

        // Regular option
        let opt      = &q.options[idx];
        let checkbox = if q.multi_select {
            if aq.checked[idx] { "[✓] " } else { "[ ] " }
        } else {
            ""
        };
        let num_style   = if is_selected {
            Style::default().fg(RC::Green)
        } else {
            Style::default().fg(RC::DarkGray)
        };
        let label_style = if is_selected {
            Style::default().fg(RC::White).add_modifier(Modifier::BOLD)
        } else {
            Style::default().fg(RC::White)
        };

        lines.push(Line::from(vec![
            Span::styled(
                format!(" {selector} "),
                Style::default().fg(RC::Green),
            ),
            Span::styled(format!("{}. ", idx + 1), num_style),
            Span::styled(checkbox.to_string(), Style::default().fg(RC::Green)),
            Span::styled(opt.label.clone(), label_style),
        ]));
        if !opt.description.is_empty() {
            lines.push(Line::from(Span::styled(
                format!("       {}", opt.description),
                Style::default().fg(RC::DarkGray),
            )));
        }
    }

    // Hint line
    lines.push(Line::from(""));
    let hint = if q.multi_select {
        "Enter toggle · ↑↓ navigate · Enter on Submit to confirm · Esc cancel"
    } else {
        "Enter select · ↑↓ navigate · 1-N quick-pick · Esc cancel"
    };
    lines.push(Line::from(Span::styled(
        hint,
        Style::default().fg(RC::DarkGray).add_modifier(Modifier::DIM),
    )));

    frame.render_widget(
        Paragraph::new(lines).wrap(Wrap { trim: false }),
        body_area,
    );
}

// ── Line renderers ────────────────────────────────────────────────────────────

fn render_line_to_text(rl: &RenderLine, width: usize, expand_all: bool, out: &mut Vec<Line<'static>>) {
    match rl {
        RenderLine::Separator => {
            out.push(Line::from(Span::styled(
                "─".repeat(width),
                Style::default().fg(RC::DarkGray),
            )));
        }
        RenderLine::Blank => {
            out.push(Line::from(""));
        }
        RenderLine::UserMessage(text) => {
            let sep = "─".repeat(width);
            out.push(Line::from(Span::styled(sep, Style::default().fg(RC::DarkGray))));
            out.extend(crate::ui::markdown::parse_markdown_lines(text));
        }
        RenderLine::AssistantText(text) => {
            out.push(Line::from(""));
            out.extend(crate::ui::markdown::parse_markdown_lines(text));
            out.push(Line::from(""));
        }
        RenderLine::ToolCall { name, preview } => {
            // Blank spacer before each tool group.
            out.push(Line::from(""));
            let display = display_tool_name(name);
            let name_style = Style::default().add_modifier(Modifier::BOLD).fg(RC::Rgb(140, 140, 249));
            // Budget: width minus "● " (2) + name len + "()" (2) + a little breathing room.
            let budget = width.saturating_sub(display.len() + 6);
            let dot_color = RC::Rgb(100, 207, 180); // teal — distinct from assistant purple
            // Format: ● Name(args)  — matches Claude Code style (no space before paren).
            let args_span = if preview.is_empty() {
                Span::styled(")", Style::default().fg(RC::DarkGray))
            } else if expand_all || preview.len() < budget {
                Span::styled(format!("{})", preview), Style::default().fg(RC::DarkGray))
            } else {
                let truncated = truncate_str(preview, budget.saturating_sub(1));
                Span::styled(format!("{truncated}…)"), Style::default().fg(RC::DarkGray))
            };
            let spans: Vec<Span<'static>> = vec![
                Span::styled("● ", Style::default().fg(dot_color)),
                Span::styled(format!("{display}("), name_style),
                args_span,
            ];
            out.push(Line::from(spans));
        }
        RenderLine::ToolResult { is_error, content } => {
            let color = if *is_error {
                RC::Rgb(241, 104, 159)
            } else {
                RC::Rgb(100, 207, 100)
            };
            let inner_w  = width.saturating_sub(5);
            let lns: Vec<&str> = content.lines().collect();
            // Collapsed: show up to 3 lines; Expanded (ctrl+o): show up to 20.
            // Format matches Claude Code: first line prefixed with ⎿, subsequent
            // lines indented, trailing "… +N lines (ctrl+o to expand)" hint.
            if lns.is_empty() {
                out.push(Line::from(vec![
                    Span::styled("  ⎿  ", Style::default().fg(RC::DarkGray)),
                    Span::styled("(no output)", Style::default().fg(color).add_modifier(Modifier::ITALIC)),
                ]));
            } else {
                let show_limit = if expand_all { 20 } else { 3 };
                let show = lns.len().min(show_limit);

                // First line — bold, with ⎿ gutter
                out.push(Line::from(vec![
                    Span::styled("  ⎿  ", Style::default().fg(RC::DarkGray)),
                    Span::styled(truncate_str(lns[0], inner_w), Style::default().fg(color).add_modifier(Modifier::BOLD)),
                ]));
                // Subsequent shown lines — indented to match the ⎿ gutter
                for ln in &lns[1..show] {
                    out.push(Line::from(vec![
                        Span::raw("     "),
                        Span::styled(truncate_str(ln, inner_w), Style::default().fg(color)),
                    ]));
                }
                // Truncation hint
                let remaining = lns.len().saturating_sub(show);
                if remaining > 0 {
                    let hint = if expand_all {
                        format!("… +{remaining} lines")
                    } else {
                        format!("… +{remaining} lines (ctrl+o to expand)")
                    };
                    out.push(Line::from(vec![
                        Span::raw("     "),
                        Span::styled(hint, Style::default().fg(RC::DarkGray).add_modifier(Modifier::ITALIC)),
                    ]));
                }
            }
        }
        RenderLine::Reasoning { words: _, content } => {
            out.push(Line::from(Span::styled(
                format!("💭 Thinking…"),
                Style::default().fg(RC::DarkGray).add_modifier(Modifier::ITALIC),
            )));
            if expand_all {
                let inner_w = width.saturating_sub(5);
                for ln in content.lines() {
                    out.push(Line::from(vec![
                        Span::raw("   "),
                        Span::styled(truncate_str(ln, inner_w), Style::default().fg(RC::DarkGray).add_modifier(Modifier::ITALIC)),
                    ]));
                }
            }
        }
        RenderLine::SystemMsg(text) => {
            for ln in text.lines() {
                out.push(Line::from(Span::styled(
                    ln.to_string(),
                    Style::default().fg(RC::Gray),
                )));
            }
        }
        RenderLine::SuccessMsg(text) => {
            for ln in text.lines() {
                out.push(Line::from(Span::styled(
                    ln.to_string(),
                    Style::default().fg(RC::Green),
                )));
            }
        }
        RenderLine::InfoHeader(text) => {
            for ln in text.lines() {
                out.push(Line::from(Span::styled(
                    ln.to_string(),
                    Style::default().fg(RC::Cyan).add_modifier(Modifier::BOLD),
                )));
            }
        }
        RenderLine::DimMsg(text) => {
            for ln in text.lines() {
                out.push(Line::from(Span::styled(
                    ln.to_string(),
                    Style::default().fg(RC::DarkGray).add_modifier(Modifier::DIM),
                )));
            }
        }
        RenderLine::Pair { label, value } => {
            out.push(Line::from(vec![
                Span::styled(format!("  {label:<20}"), Style::default().fg(RC::DarkGray)),
                Span::styled(value.clone(), Style::default().fg(RC::White)),
            ]));
        }
        RenderLine::ErrorMsg(text) => {
            for ln in text.lines() {
                out.push(Line::from(Span::styled(
                    format!("  ✗ {ln}"),
                    Style::default().fg(RC::Red),
                )));
            }
        }
        RenderLine::QuestionResult { header, answer } => {
            out.push(Line::from(vec![
                Span::styled("● ", Style::default().fg(RC::Green).add_modifier(Modifier::BOLD)),
                Span::styled(format!("{header}: "), Style::default().fg(RC::Cyan).add_modifier(Modifier::BOLD)),
                Span::styled(answer.clone(), Style::default().fg(RC::White)),
            ]));
        }
        RenderLine::Table { headers, rows } => {
            if rows.is_empty() { return; }
            let n_cols = headers.len();
            let mut widths = vec![0; n_cols];
            for (i, h) in headers.iter().enumerate() {
                widths[i] = h.len();
            }
            for row in rows {
                for (i, cell) in row.iter().enumerate() {
                    if i < n_cols {
                        widths[i] = widths[i].max(cell.len());
                    }
                }
            }

            // Draw header
            let mut header_spans = Vec::new();
            for (i, h) in headers.iter().enumerate() {
                header_spans.push(Span::styled(
                    format!("  {:<width$}  ", h, width = widths[i]),
                    Style::default().fg(RC::Cyan).add_modifier(Modifier::BOLD | Modifier::UNDERLINED),
                ));
            }
            out.push(Line::from(header_spans));

            // Draw rows
            for row in rows {
                let mut row_spans = Vec::new();
                for (i, cell) in row.iter().enumerate() {
                    if i < n_cols {
                        row_spans.push(Span::styled(
                            format!("  {:<width$}  ", cell, width = widths[i]),
                            Style::default().fg(RC::Gray),
                        ));
                    }
                }
                out.push(Line::from(row_spans));
            }
            out.push(Line::from(""));
        }
    }
}

fn render_assistant_lines(text: &str, _width: usize, out: &mut Vec<Line<'static>>) {
    let md_lines = crate::ui::markdown::parse_markdown_lines(text);
    if md_lines.is_empty() {
        out.push(Line::from(Span::styled(
            "● ",
            Style::default().fg(RC::Rgb(140, 100, 200)),
        )));
        return;
    }
    for (i, ml) in md_lines.into_iter().enumerate() {
        if i == 0 {
            // Prepend "● " (purple dot) to the first line of the response.
            let mut spans = vec![Span::styled(
                "● ",
                Style::default().fg(RC::Rgb(140, 100, 200)),
            )];
            spans.extend(ml.spans.into_iter());
            out.push(Line::from(spans));
        } else {
            out.push(ml);
        }
    }
}

// ── Input helpers (ported from input.rs) ──────────────────────────────────────

fn calc_input_rows(buf: &str, available_width: u16) -> u16 {
    // available_width is the inner width of the input chunk (border already
    // subtracted by the caller).  Each logical line is rendered as its own
    // ratatui Line with a 2-char prefix ("> " or "  "), so every logical line
    // has an effective text width of (available_width - 2).  We count the
    // number of visual rows that ratatui will produce for each logical line.
    let w = available_width.max(1) as usize;
    let text_w = w.saturating_sub(2).max(1);   // width after "  " / "> " prefix
    if buf.is_empty() { return 1; }
    let mut total: u16 = 0;
    for seg in buf.split('\n') {
        let chars = seg.chars().count();
        let rows  = if chars == 0 { 1 } else { ((chars + text_w - 1) / text_w) as u16 };
        total += rows;
    }
    total.max(1).min(MAX_INPUT_ROWS)
}

fn calc_visual_cursor(before_cursor: &str, available_width: u16) -> (u16, u16) {
    // Mirror exactly how render_frame builds the Paragraph:
    //   • Each logical line (split on '\n') is its own ratatui Line.
    //   • Every line has a 2-char prefix (">" / "  "), so text wraps at
    //     (available_width - 2) columns.
    //   • The cursor column is 2 + (chars_on_current_visual_row - 1) when
    //     still on the first visual row of a logical line, or
    //     2 + (chars_on_wrap_row - 1) on subsequent wrap rows.
    let w      = available_width.max(1) as usize;
    let text_w = w.saturating_sub(2).max(1);  // text columns per visual row

    let mut vis_row: u16 = 0;
    let mut vis_col: u16 = 2; // starts after the "  " / "> " prefix

    for (li, seg) in before_cursor.split('\n').enumerate() {
        if li > 0 {
            // Crossed a \n: start a new logical line → new visual row, prefix col
            vis_row += 1;
            vis_col  = 2;
        }
        // Walk through the segment, wrapping when we exceed text_w
        let mut chars_on_row: usize = 0;
        for _ch in seg.chars() {
            chars_on_row += 1;
            if chars_on_row > text_w {
                // Wrap to next visual row within this logical line
                vis_row += 1;
                chars_on_row = 1;
                vis_col = 2 + 1; // prefix (2) + 1st char of new wrap row
            } else {
                vis_col = 2 + chars_on_row as u16;
            }
        }
        // After processing all chars of this segment, vis_col is already set
        // correctly for the end of the segment.  If the segment was empty
        // (bare \n), vis_col stays at 2 (just the prefix).
    }

    (vis_row, vis_col)
}

/// Given the full input `buf`, the visual text-column width `text_w`
/// (= available_width - 2, matching `calc_visual_cursor`), and a target
/// `(row, col)` in visual space, return the **byte offset** in `buf` of the
/// character at that visual position.
///
/// Used by the Up/Down cursor-movement logic.
fn find_cursor_at_visual_row_col(buf: &str, text_w: usize, target_row: u16, target_col: u16) -> usize {
    let mut vis_row: u16 = 0;
    let mut chars_on_row: usize = 0;
    let mut byte_offset: usize = 0;

    for (li, seg) in buf.split('\n').enumerate() {
        if li > 0 {
            vis_row += 1;
            chars_on_row = 0;
            byte_offset += 1; // the '\n' byte
        }
        if vis_row > target_row {
            break;
        }
        let seg_start = byte_offset;
        for ch in seg.chars() {
            chars_on_row += 1;
            if chars_on_row > text_w {
                // visual wrap
                vis_row += 1;
                chars_on_row = 1;
            }
            if vis_row == target_row {
                // We're on the target row — check column
                // col is 1-based relative to content (after the 2-char prefix)
                let content_col = target_col.saturating_sub(2) as usize;
                if chars_on_row > content_col {
                    return byte_offset;
                }
            }
            if vis_row > target_row {
                // Overshot — return last valid position on target row
                return byte_offset;
            }
            byte_offset += ch.len_utf8();
        }
        // If we passed through the whole segment without overshooting, the
        // cursor target is at the end of the segment (or beyond — clamp to end).
        if vis_row == target_row {
            // Return end of this segment (before the next \n or end of string)
            return byte_offset;
        }
        let _ = seg_start; // suppress unused warning
    }
    // Clamp to end of buffer
    buf.len()
}

/// Update context window usage percentage (0–99).
/// Called from repl.rs after each usage_statistics SSE event.
impl TuiApp {
    pub fn set_context_pct(&mut self, pct: u8) {
        self.context_pct = Some(pct.min(99));
    }
}

// ── File picker helpers (A-01) ────────────────────────────────────────────────

/// Walk `root` up to `max_depth` levels deep, collecting files whose names
/// contain `query` (case-insensitive).  Skips hidden paths and common noise
/// directories (`target`, `node_modules`, `.git`).  Returns relative paths.
fn collect_files(root: &std::path::Path, query: &str) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    collect_files_inner(root, root, 0, 3, query, &mut out);
    out.sort();
    out.truncate(50);
    out
}

fn collect_files_inner(
    root:      &std::path::Path,
    dir:       &std::path::Path,
    depth:     u32,
    max_depth: u32,
    query:     &str,
    out:       &mut Vec<String>,
) {
    if depth > max_depth { return; }
    let Ok(entries) = std::fs::read_dir(dir) else { return; };
    for entry in entries.filter_map(|e| e.ok()) {
        let name = entry.file_name().to_string_lossy().to_string();
        if name.starts_with('.') { continue; }
        if matches!(name.as_str(), "target" | "node_modules" | ".git") { continue; }
        let path = entry.path();
        if path.is_dir() {
            collect_files_inner(root, &path, depth + 1, max_depth, query, out);
        } else if query.is_empty() || name.to_lowercase().contains(&query.to_lowercase()) {
            let rel = path.strip_prefix(root)
                .ok()
                .and_then(|p| p.to_str())
                .map(|s| s.to_string())
                .unwrap_or(name);
            out.push(rel);
        }
    }
}

/// Render the `@` file picker as a floating overlay at the bottom of `area`.
fn render_picker(frame: &mut Frame, pk: &PickerState, area: Rect) {
    if area.height == 0 { return; }
    let w = area.width as usize;
    let mut lines: Vec<Line<'static>> = Vec::new();

    // Top dashed separator (matches question-panel style)
    lines.push(Line::from(Span::styled(
        "╌".repeat(w),
        Style::default().fg(RC::Rgb(70, 70, 110)),
    )));

    // Header: "@ <query>" + no-match hint
    let no_match = if pk.matches.is_empty() && !pk.query.is_empty() {
        "  (no matches)"
    } else {
        ""
    };
    lines.push(Line::from(vec![
        Span::styled(
            format!(" @ {}", pk.query),
            Style::default().fg(RC::Rgb(140, 140, 249)).add_modifier(Modifier::BOLD),
        ),
        Span::styled(no_match, Style::default().fg(RC::DarkGray)),
    ]));

    // Match entries — fill remaining rows (minus sep + header already pushed)
    let max_entries = (area.height as usize).saturating_sub(lines.len());
    for (i, m) in pk.matches.iter().take(max_entries).enumerate() {
        let selected = i == pk.cursor;
        let (glyph, style) = if selected {
            ("❯", Style::default().fg(RC::White).add_modifier(Modifier::BOLD))
        } else {
            (" ", Style::default().fg(RC::Rgb(130, 130, 130)))
        };
        lines.push(Line::from(Span::styled(
            format!(" {glyph} {m}"),
            style,
        )));
    }

    frame.render_widget(
        Paragraph::new(lines).style(Style::default().bg(RC::Rgb(18, 18, 32))),
        area,
    );
}

// ── Skills overlay rendering ──────────────────────────────────────────────────

fn render_skills_overlay(frame: &mut Frame, ov: &SkillsOverlayState, area: Rect) {
    // Dark background covering the entire terminal
    frame.render_widget(
        Paragraph::new("").style(Style::default().bg(RC::Rgb(10, 10, 18))),
        area,
    );

    let inner = Rect {
        x: area.x + 1,
        y: area.y + 1,
        width: area.width.saturating_sub(2),
        height: area.height.saturating_sub(3), // leave 2 rows for hint at bottom
    };
    let hint_area = Rect {
        x: area.x + 2,
        y: area.y + area.height.saturating_sub(2),
        width: area.width.saturating_sub(4),
        height: 1,
    };

    match &ov.mode {
        SkillsMode::List => {
            render_skills_list(frame, ov, inner, hint_area);
        }
        SkillsMode::Detail => {
            render_skills_detail(frame, ov, inner, hint_area, false);
        }
        SkillsMode::Edit => {
            render_skills_detail(frame, ov, inner, hint_area, true);
        }
    }
}

fn render_skills_list(frame: &mut Frame, ov: &SkillsOverlayState, area: Rect, hint_area: Rect) {
    // Title bar
    let title_area = Rect { x: area.x, y: area.y, width: area.width, height: 1 };
    frame.render_widget(
        Paragraph::new(Line::from(vec![
            Span::styled("  ◆ Skills  ", Style::default().fg(RC::Cyan).add_modifier(Modifier::BOLD)),
            Span::styled(
                format!("({} loaded)", ov.skills.len()),
                Style::default().fg(RC::DarkGray),
            ),
        ])),
        title_area,
    );

    if ov.skills.is_empty() {
        let msg_area = Rect { x: area.x, y: area.y + 2, width: area.width, height: 3 };
        frame.render_widget(
            Paragraph::new(vec![
                Line::from(Span::styled("  No skills found.", Style::default().fg(RC::DarkGray))),
                Line::from(""),
                Line::from(Span::styled("  /skills create <name>  to scaffold your first skill", Style::default().fg(RC::DarkGray))),
            ]),
            msg_area,
        );
        render_hint(frame, "Esc close", hint_area);
        return;
    }

    let card_h: u16 = 5;
    let cards_area = Rect { x: area.x, y: area.y + 1, width: area.width, height: area.height.saturating_sub(1) };
    let visible = (cards_area.height / card_h) as usize;
    let start   = ov.list_scroll;
    let end     = (start + visible).min(ov.skills.len());

    let constraints: Vec<Constraint> = (start..end).map(|_| Constraint::Length(card_h)).collect();
    if constraints.is_empty() {
        render_hint(frame, "j/k navigate  ·  Enter detail  ·  Esc close", hint_area);
        return;
    }
    let card_rects = Layout::vertical(constraints).split(cards_area);

    for (i, sk) in ov.skills[start..end].iter().enumerate() {
        let selected = (start + i) == ov.cursor;
        let border_style = if selected {
            Style::default().fg(RC::Cyan).add_modifier(Modifier::BOLD)
        } else {
            Style::default().fg(RC::Rgb(60, 60, 90))
        };
        let scope_str = sk.scope.to_string();
        let title_str = format!(" {} ", sk.name);
        let scope_tag = format!(" [{}] ", scope_str);

        let block = Block::new()
            .borders(Borders::ALL)
            .border_style(border_style)
            .title(Line::from(vec![
                Span::styled(title_str, Style::default().fg(if selected { RC::White } else { RC::Rgb(180,180,180) }).add_modifier(Modifier::BOLD)),
                Span::styled(scope_tag, Style::default().fg(RC::Rgb(100, 140, 100))),
            ]));

        let card_inner = block.inner(card_rects[i]);
        frame.render_widget(block, card_rects[i]);

        let tags_str = if sk.tags.is_empty() { "—".to_string() } else { sk.tags.join(", ") };
        let trig_str = if sk.triggers.is_empty() { "—".to_string() } else { sk.triggers.join(", ") };
        let w = card_inner.width as usize;
        let trunc = |s: &str| -> String {
            if s.len() > w.saturating_sub(2) { format!("{}…", &s[..w.saturating_sub(3).min(s.len())]) } else { s.to_string() }
        };

        let content = vec![
            Line::from(Span::styled(trunc(&sk.description), Style::default().fg(RC::Rgb(200,200,200)))),
            Line::from(vec![
                Span::styled("Tags: ", Style::default().fg(RC::DarkGray)),
                Span::styled(trunc(&tags_str), Style::default().fg(RC::Rgb(160,160,160))),
            ]),
            Line::from(vec![
                Span::styled("Triggers: ", Style::default().fg(RC::DarkGray)),
                Span::styled(trunc(&trig_str), Style::default().fg(RC::Rgb(160,160,160))),
            ]),
        ];
        frame.render_widget(Paragraph::new(content), card_inner);
    }

    render_hint(frame, "j/k navigate  ·  Enter detail  ·  Esc close", hint_area);
}

fn render_skills_detail(frame: &mut Frame, ov: &SkillsOverlayState, area: Rect, hint_area: Rect, edit_mode: bool) {
    let skill = match ov.skills.get(ov.cursor) {
        Some(s) => s,
        None => return,
    };

    let dirty_mark = if ov.dirty { " ● " } else { " " };
    let title = if edit_mode {
        format!(" Edit: {}{}", skill.id, dirty_mark)
    } else {
        format!(" {} ", skill.id)
    };

    let block = Block::new()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(if edit_mode { RC::Rgb(100, 100, 200) } else { RC::Rgb(60, 100, 160) }))
        .title(Line::from(vec![
            Span::styled(title, Style::default().fg(RC::White).add_modifier(Modifier::BOLD)),
            Span::styled(format!(" [{}] ", skill.scope), Style::default().fg(RC::Rgb(100, 140, 100))),
        ]));

    let detail_inner = block.inner(area);
    frame.render_widget(block, area);

    const FIELD_NAMES: [&str; 6] = ["Name", "Description", "Category", "Tags", "Triggers", "Body"];
    const LABEL_W: usize = 14;

    let mut lines: Vec<Line<'static>> = Vec::new();

    // Render meta fields (0-4)
    for fi in 0..5 {
        let is_active = edit_mode && ov.field_cursor == fi;
        let label = format!("  {:<width$} ", FIELD_NAMES[fi], width = LABEL_W);

        let value_str: String = if edit_mode {
            ov.edit_fields.get(fi).cloned().unwrap_or_default()
        } else {
            match fi {
                0 => skill.name.clone(),
                1 => skill.description.clone(),
                2 => skill.category.clone().unwrap_or_default(),
                3 => skill.tags.join(", "),
                4 => skill.triggers.join(", "),
                _ => String::new(),
            }
        };

        let value_with_cursor: String = if is_active {
            let pos = ov.field_pos.min(value_str.len());
            format!("{}█{}", &value_str[..pos], &value_str[pos..])
        } else {
            value_str.clone()
        };

        let val_style = if is_active {
            Style::default().fg(RC::White).bg(RC::Rgb(30, 30, 60))
        } else {
            Style::default().fg(RC::Rgb(200, 200, 200))
        };

        lines.push(Line::from(vec![
            Span::styled(label, Style::default().fg(RC::DarkGray)),
            Span::styled(value_with_cursor, val_style),
        ]));
    }

    // Body separator
    lines.push(Line::from(Span::styled(
        format!("  {}──", "─".repeat(LABEL_W + 2)),
        Style::default().fg(RC::Rgb(60, 60, 80)),
    )));

    // Body content (field 5)
    let body_str = if edit_mode {
        ov.edit_fields.get(5).cloned().unwrap_or_default()
    } else {
        skill.body.clone()
    };
    let is_body_active = edit_mode && ov.field_cursor == 5;

    let body_lines: Vec<&str> = body_str.split('\n').collect();
    let body_start = ov.detail_scroll;
    let body_visible = (detail_inner.height as usize).saturating_sub(lines.len() + 1);

    for (bi, bline) in body_lines.iter().enumerate().skip(body_start).take(body_visible) {
        let line_style = if is_body_active {
            Style::default().fg(RC::White).bg(RC::Rgb(20, 20, 50))
        } else {
            Style::default().fg(RC::Rgb(170, 170, 190))
        };
        let displayed = if is_body_active {
            let cursor_line = body_str[..ov.field_pos.min(body_str.len())]
                .chars().filter(|&c| c == '\n').count();
            if bi == cursor_line {
                let line_start: usize = body_str[..ov.field_pos.min(body_str.len())]
                    .rfind('\n').map(|p| p + 1).unwrap_or(0);
                let col = ov.field_pos.saturating_sub(line_start).min(bline.len());
                format!("{}█{}", &bline[..col], &bline[col..])
            } else { bline.to_string() }
        } else { bline.to_string() };

        lines.push(Line::from(Span::styled(format!("  {}", displayed), line_style)));
    }

    frame.render_widget(
        Paragraph::new(lines).wrap(Wrap { trim: false }),
        detail_inner,
    );

    let hint = if edit_mode {
        "Tab next field  ·  Ctrl+S save  ·  Esc cancel"
    } else {
        "e edit  ·  j/k scroll  ·  Esc back  ·  d delete"
    };
    render_hint(frame, hint, hint_area);
}

fn render_hint(frame: &mut Frame, hint: &str, area: Rect) {
    frame.render_widget(
        Paragraph::new(Span::styled(hint.to_string(), Style::default().fg(RC::Rgb(90, 90, 110)))),
        area,
    );
}

// ── Path completion (I-02) ────────────────────────────────────────────────────

/// Try to complete a filesystem path token at `cursor` in `input`.
/// Returns `(new_input, new_cursor)` if a completion was found, `None` otherwise.
/// Only triggers when the token at the cursor starts with `/`, `./`, `~/`, or
/// contains `/` (looks like a path).
fn complete_path(input: &str, cursor: usize) -> Option<(String, usize)> {
    let cursor = cursor.min(input.len());
    let before = &input[..cursor];

    // Find start of the current token (split on whitespace).
    let word_start = before
        .rfind(|c: char| c.is_whitespace())
        .map(|i| i + 1)
        .unwrap_or(0);
    let partial = &before[word_start..];

    // Only attempt if the token looks like a path.
    if !partial.starts_with('/')
        && !partial.starts_with("./")
        && !partial.starts_with("~/")
        && !partial.contains('/')
    {
        return None;
    }

    // Expand leading ~/
    let home = dirs::home_dir();
    let expanded: std::path::PathBuf = if partial.starts_with("~/") {
        let h = home.as_deref()?;
        h.join(&partial[2..])
    } else {
        std::path::PathBuf::from(partial)
    };

    // Split into parent directory and filename prefix to match.
    let (parent, file_prefix, dir_suffix) = if partial.ends_with('/') {
        (expanded.clone(), "", true)
    } else {
        let p = expanded.parent().unwrap_or(std::path::Path::new(".")).to_path_buf();
        let f = expanded.file_name().and_then(|n| n.to_str()).unwrap_or("");
        (p, f, false)
    };

    // List the parent directory.
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

    // Common prefix of all match names.
    let names: Vec<String> = matches.iter().map(|(n, _)| n.clone()).collect();
    let prefix_str = common_prefix(&names);
    // If exactly one match, add trailing / for directories.
    let suffix = if matches.len() == 1 && matches[0].1 { "/" } else { "" };
    let completed_name = format!("{prefix_str}{suffix}");

    // Rebuild the token, preserving the original ~/ or ./ prefix style.
    let parent_display: String = {
        let parent_str = parent.to_string_lossy();
        if let Some(ref h) = home {
            if parent.starts_with(h) {
                let rel = parent.strip_prefix(h).ok().and_then(|p| p.to_str()).unwrap_or("");
                if rel.is_empty() {
                    "~/".to_string()
                } else {
                    format!("~/{rel}/")
                }
            } else if dir_suffix {
                // partial ended with /; parent is the full expanded path
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

    let new_token = if dir_suffix {
        // partial was "dir/"; parent is already the dir
        format!("{}{}", parent_display, completed_name)
    } else if partial.ends_with('/') {
        format!("{}{}", parent_display, completed_name)
    } else {
        // Restore the leading ./ or ~/ prefix from the original partial.
        let leading: &str = if partial.starts_with("~/") {
            "~/"
        } else if partial.starts_with("./") {
            "./"
        } else if partial.starts_with('/') {
            ""  // absolute — parent_display already has the /
        } else {
            ""
        };
        let _ = leading; // parent_display already encodes origin
        format!("{}{}", parent_display, completed_name)
    };

    let new_cursor = word_start + new_token.len();
    let new_input  = format!("{}{}{}", &input[..word_start], new_token, &input[cursor..]);
    Some((new_input, new_cursor))
}

/// Longest common prefix of a non-empty slice of strings.
fn common_prefix(words: &[String]) -> String {
    if words.is_empty() { return String::new(); }
    let first = &words[0];
    let len = words.iter().skip(1).map(|w| {
        first.chars().zip(w.chars()).take_while(|(a, b)| a == b).count()
    }).min().unwrap_or(first.chars().count());
    first.chars().take(len).collect()
}

/// Abbreviate a filesystem path for the footer: last 2 components, with ~/
/// prefix when the path is under the user's home directory.
fn abbreviate_cwd(path: &std::path::Path) -> String {
    let home = dirs::home_dir();
    let (prefix, rel_path) = if let Some(ref h) = home {
        if let Ok(rel) = path.strip_prefix(h) {
            ("~/".to_string(), rel.to_path_buf())
        } else {
            (String::new(), path.to_path_buf())
        }
    } else {
        (String::new(), path.to_path_buf())
    };

    let parts: Vec<std::ffi::OsString> = rel_path.components()
        .map(|c| c.as_os_str().to_owned())
        .collect();

    if parts.is_empty() {
        return if prefix.is_empty() { "/".to_string() } else { "~".to_string() };
    }

    let display: String = if parts.len() <= 2 {
        parts.iter()
            .map(|p| p.to_string_lossy())
            .collect::<Vec<_>>()
            .join("/")
    } else {
        let last2: String = parts[parts.len() - 2..]
            .iter()
            .map(|p| p.to_string_lossy())
            .collect::<Vec<_>>()
            .join("/");
        format!("…/{last2}")
    };

    format!("{prefix}{display}")
}

fn mode_sep_color(mode: PermissionMode) -> RC {
    match mode {
        PermissionMode::Default           => RC::Rgb(70, 72, 74),
        PermissionMode::AcceptEdits       => RC::Rgb(140, 140, 249),
        PermissionMode::Plan              => RC::Green,
        PermissionMode::BypassPermissions => RC::Red,
    }
}

fn mode_footer_left(mode: PermissionMode) -> (&'static str, &'static str, RC) {
    match mode {
        PermissionMode::Default           => ("Press / for commands", "",    RC::Rgb(70, 72, 74)),
        PermissionMode::AcceptEdits       => ("accept edits",         "⏵⏵", RC::Rgb(140, 140, 249)),
        PermissionMode::Plan              => ("plan mode",            "⏸",  RC::Green),
        PermissionMode::BypassPermissions => ("bypass (allow all)",   "⚡",  RC::Red),
    }
}

pub fn cycle_mode(mode: PermissionMode) -> PermissionMode {
    match mode {
        PermissionMode::Default           => PermissionMode::AcceptEdits,
        PermissionMode::AcceptEdits       => PermissionMode::Plan,
        PermissionMode::Plan              => PermissionMode::BypassPermissions,
        PermissionMode::BypassPermissions => PermissionMode::Default,
    }
}

pub fn cycle_mode_back(mode: PermissionMode) -> PermissionMode {
    match mode {
        PermissionMode::Default           => PermissionMode::BypassPermissions,
        PermissionMode::AcceptEdits       => PermissionMode::Default,
        PermissionMode::Plan              => PermissionMode::AcceptEdits,
        PermissionMode::BypassPermissions => PermissionMode::Plan,
    }
}

// ── Misc helpers ──────────────────────────────────────────────────────────────

fn display_tool_name(name: &str) -> String {
    // Strip MCP server prefix: "developer__shell" → "shell"
    let stripped = if let Some(pos) = name.rfind("__") {
        &name[pos + 2..]
    } else {
        name
    };
    stripped.to_string()
}

pub fn truncate_str(s: &str, max: usize) -> String {
    let chars: Vec<char> = s.chars().collect();
    if chars.len() <= max {
        s.to_string()
    } else {
        format!("{}…", chars[..max.saturating_sub(1)].iter().collect::<String>())
    }
}


#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_question_result_formatting() {
        let line = RenderLine::QuestionResult { 
            header: "Decision".to_string(), 
            answer: "Yes".to_string() 
        };
        
        match line {
            RenderLine::QuestionResult { header, answer } => {
                assert_eq!(header, "Decision");
                assert_eq!(answer, "Yes");
            },
            _ => panic!("Expected QuestionResult"),
        }
    }
}
