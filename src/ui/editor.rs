//! Standalone text-editor component for the CADE TUI.
//!
//! Owns the input buffer (`input`) and cursor position (`cursor_pos`),
//! provides pure text-manipulation methods (insert, delete, cursor
//! movement, word ops), bracketed-paste collapsing, and undo/redo.
//!
//! Implements [`Component`] so it can participate in the unified render /
//! input cycle.
//!
//! The `TuiApp` embeds a single `Editor` instance.  UI-coupled concerns
//! (history navigation, `@` file picker, Tab path completion) remain in
//! `TuiApp::handle_key_input`; only the text-editing primitives live here.

use std::collections::VecDeque;
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use super::component::{Component, RenderedLine};

// ── Input mode ────────────────────────────────────────────────────────────────

/// Semantic mode of the current input buffer, determined by its prefix.
///
/// `TuiApp` can query this to show visual feedback (e.g. a `[bash]` badge).
/// `repl.rs` already handles the actual `!`/`!!` dispatch at submission time.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InputMode {
    /// Regular user message (default).
    Regular,
    /// Starts with `!` (send output to LLM) or `!!` (silent execution).
    BashCommand { silent: bool },
    /// Starts with `/` — a slash command.
    SlashCommand,
}

// ── Paste entry ───────────────────────────────────────────────────────────────

/// A collapsed paste marker stored for later expansion.
#[derive(Debug, Clone)]
pub struct PasteEntry {
    /// 1-based paste ID shown in the marker token.
    pub id: usize,
    /// The full original pasted text.
    pub text: String,
}

// ── Editor ────────────────────────────────────────────────────────────────────

/// Standalone multi-line text editor component.
pub struct Editor {
    /// The raw text buffer (UTF-8).
    pub input: String,
    /// Byte-offset cursor position within `input`.
    pub cursor_pos: usize,

    // ── Bracketed paste ───────────────────────────────────────────────────
    /// Monotonically increasing paste counter (for `[paste #N …]` markers).
    paste_counter: usize,
    /// Stored paste buffers keyed by their marker ID.
    pub paste_buffers: Vec<PasteEntry>,

    // ── Undo / redo ───────────────────────────────────────────────────────
    /// Snapshots of (input, cursor_pos) taken *before* each edit (max 100).
    /// `undo()` pops the top and restores it.
    undo_stack: VecDeque<(String, usize)>,
    /// States saved by `undo()` so `redo()` can reapply them.
    redo_stack: VecDeque<(String, usize)>,
}

/// Maximum number of paste lines shown verbatim before collapsing.
const PASTE_COLLAPSE_THRESHOLD: usize = 10;
/// Maximum entries kept in the undo / redo stacks.
const UNDO_LIMIT: usize = 100;

impl Editor {
    pub fn new() -> Self {
        Self {
            input: String::new(),
            cursor_pos: 0,
            paste_counter: 0,
            paste_buffers: Vec::new(),
            undo_stack: VecDeque::with_capacity(UNDO_LIMIT),
            redo_stack: VecDeque::with_capacity(UNDO_LIMIT),
        }
    }

    // ── Undo / redo ───────────────────────────────────────────────────────

    /// Save current `(input, cursor_pos)` to the undo stack **before** a
    /// destructive edit.  Clears the redo stack (new edit invalidates
    /// any undone future).  Silently caps the stack at `UNDO_LIMIT`.
    fn snapshot(&mut self) {
        if self.undo_stack.len() >= UNDO_LIMIT {
            self.undo_stack.pop_front();
        }
        self.undo_stack.push_back((self.input.clone(), self.cursor_pos));
        self.redo_stack.clear();
    }

    /// Undo the last edit.  Returns `true` if a state was restored.
    pub fn undo(&mut self) -> bool {
        if let Some((input, pos)) = self.undo_stack.pop_back() {
            // Save current state so redo() can reapply it.
            if self.redo_stack.len() >= UNDO_LIMIT {
                self.redo_stack.pop_front();
            }
            self.redo_stack.push_back((self.input.clone(), self.cursor_pos));
            self.input = input;
            self.cursor_pos = pos;
            true
        } else {
            false
        }
    }

    /// Redo the last undone edit.  Returns `true` if a state was reapplied.
    pub fn redo(&mut self) -> bool {
        if let Some((input, pos)) = self.redo_stack.pop_back() {
            if self.undo_stack.len() >= UNDO_LIMIT {
                self.undo_stack.pop_front();
            }
            self.undo_stack.push_back((self.input.clone(), self.cursor_pos));
            self.input = input;
            self.cursor_pos = pos;
            true
        } else {
            false
        }
    }

    // ── Insert / delete ───────────────────────────────────────────────────

    /// Insert a character at the current cursor position.
    pub fn insert_char(&mut self, c: char) {
        self.snapshot();
        let pos = self.cursor_pos;
        self.input.insert(pos, c);
        self.cursor_pos = pos + c.len_utf8();
    }

    /// Insert a string at the current cursor position.
    pub fn insert_str(&mut self, s: &str) {
        if s.is_empty() {
            return;
        }
        self.snapshot();
        self.input.insert_str(self.cursor_pos, s);
        self.cursor_pos += s.len();
    }

    /// Insert a newline at the current cursor position (delegates to `insert_char`).
    pub fn insert_newline(&mut self) {
        self.insert_char('\n');
    }

    /// Delete the character before the cursor (Backspace).
    /// Returns `true` if a character was deleted.
    pub fn delete_back(&mut self) -> bool {
        if self.cursor_pos == 0 {
            return false;
        }
        let char_len = self.input[..self.cursor_pos]
            .chars()
            .last()
            .map(|c| c.len_utf8())
            .unwrap_or(1);
        self.snapshot();
        self.cursor_pos -= char_len;
        self.input.remove(self.cursor_pos);
        true
    }

    /// Delete the character at the cursor (Delete key).
    /// Returns `true` if a character was deleted.
    pub fn delete_forward(&mut self) -> bool {
        if self.cursor_pos >= self.input.len() {
            return false;
        }
        self.snapshot();
        self.input.remove(self.cursor_pos);
        true
    }

    /// Delete from cursor to start of buffer (Ctrl+U).
    pub fn delete_to_start(&mut self) {
        if self.cursor_pos == 0 {
            return;
        }
        self.snapshot();
        self.input.drain(..self.cursor_pos);
        self.cursor_pos = 0;
    }

    /// Delete from cursor to end of current line (Ctrl+K).
    ///
    /// Stops at the next `\n`; if on the last line, deletes to end of buffer.
    pub fn delete_to_end(&mut self) {
        let end = self.input[self.cursor_pos..]
            .find('\n')
            .map(|i| self.cursor_pos + i)
            .unwrap_or(self.input.len());
        if end == self.cursor_pos {
            return; // nothing to delete — keep redo_stack intact
        }
        self.snapshot();
        self.input.drain(self.cursor_pos..end);
    }

    /// Delete word backwards (Ctrl+W).
    pub fn delete_word_back(&mut self) {
        let end = self.cursor_pos;
        if end == 0 {
            return;
        }
        let start = self.input[..end]
            .rfind(|c: char| !c.is_whitespace())
            .and_then(|p| self.input[..p].rfind(char::is_whitespace).map(|q| q + 1))
            .unwrap_or(0);
        self.snapshot();
        self.input.drain(start..end);
        self.cursor_pos = start;
    }

    // ── Cursor movement ───────────────────────────────────────────────────
    // Cursor movements do NOT snapshot (they don't modify text).

    /// Move cursor one character to the left.
    pub fn move_left(&mut self) {
        if self.cursor_pos > 0 {
            self.cursor_pos -= self.input[..self.cursor_pos]
                .chars()
                .last()
                .map(|c| c.len_utf8())
                .unwrap_or(1);
        }
    }

    /// Move cursor one character to the right.
    pub fn move_right(&mut self) {
        if self.cursor_pos < self.input.len() {
            self.cursor_pos += self.input[self.cursor_pos..]
                .chars()
                .next()
                .map(|c| c.len_utf8())
                .unwrap_or(1);
        }
    }

    /// Move cursor one word to the left (Alt+← / Ctrl+←).
    ///
    /// Skips trailing whitespace, then jumps to the start of the preceding word.
    pub fn move_word_left(&mut self) {
        if self.cursor_pos == 0 {
            return;
        }
        let before = &self.input[..self.cursor_pos];
        // Trim trailing whitespace, then find the last whitespace before the word.
        let trimmed = before.trim_end_matches(|c: char| c.is_whitespace() && c != '\n');
        let new_pos = if trimmed.is_empty() {
            // Only whitespace before cursor; jump to 0 or previous newline.
            before
                .rfind('\n')
                .map(|i| i + 1)
                .unwrap_or(0)
        } else {
            trimmed
                .rfind(|c: char| c.is_whitespace())
                .map(|i| i + 1)
                .unwrap_or(0)
        };
        self.cursor_pos = new_pos;
    }

    /// Move cursor one word to the right (Alt+→ / Ctrl+→).
    ///
    /// Skips any leading whitespace at the cursor, then jumps past the next word.
    pub fn move_word_right(&mut self) {
        if self.cursor_pos >= self.input.len() {
            return;
        }
        let after = &self.input[self.cursor_pos..];
        // Skip current word chars, then skip whitespace.
        let word_end = after
            .find(|c: char| c.is_whitespace())
            .unwrap_or(after.len());
        let rest = &after[word_end..];
        let ws_end = rest
            .find(|c: char| !c.is_whitespace())
            .unwrap_or(rest.len());
        self.cursor_pos += word_end + ws_end;
    }

    /// Move cursor to the start of the buffer (Home / Ctrl+A).
    pub fn move_home(&mut self) {
        self.cursor_pos = 0;
    }

    /// Move cursor to the end of the buffer (End / Ctrl+E).
    pub fn move_end(&mut self) {
        self.cursor_pos = self.input.len();
    }

    // ── Bulk operations ───────────────────────────────────────────────────

    /// Clear the entire buffer and reset cursor.
    /// Does NOT snapshot (used by submit / Ctrl+C — not undoable by design).
    pub fn clear(&mut self) {
        self.input.clear();
        self.cursor_pos = 0;
        // Don't touch the undo/redo stacks: they survive a clear so the user
        // can still undo within the same session window.
    }

    /// Replace the buffer contents and move cursor to end.
    /// Does NOT snapshot (used for history navigation).
    pub fn set(&mut self, text: String) {
        self.input = text;
        self.cursor_pos = self.input.len();
    }

    // ── Bracketed paste ───────────────────────────────────────────────────

    /// Handle a bracketed-paste event.
    ///
    /// If the pasted text is ≤ `PASTE_COLLAPSE_THRESHOLD` lines it is
    /// inserted verbatim (via `insert_str`, which snapshots).  Otherwise
    /// the full text is stored in `paste_buffers` and a compact marker
    /// `[paste #N +M lines]` is inserted instead.
    pub fn handle_paste(&mut self, text: &str) {
        let line_count = text.lines().count();
        if line_count <= PASTE_COLLAPSE_THRESHOLD {
            // Short paste — insert verbatim (snapshot happens inside insert_str).
            self.insert_str(text);
        } else {
            // Long paste — collapse into a marker.
            self.paste_counter += 1;
            let id = self.paste_counter;
            self.paste_buffers.push(PasteEntry {
                id,
                text: text.to_string(),
            });
            let marker = format!("[paste #{id} +{line_count} lines]");
            self.insert_str(&marker); // snapshot happens inside insert_str
        }
    }

    /// Expand all paste markers in the input buffer, replacing each
    /// `[paste #N …]` token with the original pasted text.
    /// Called just before submission — does NOT snapshot.
    pub fn expand_pastes(&mut self) {
        for entry in &self.paste_buffers {
            let marker = format!("[paste #{} +", entry.id);
            if let Some(start) = self.input.find(&marker) {
                if let Some(end) = self.input[start..].find(']') {
                    self.input.replace_range(start..start + end + 1, &entry.text);
                }
            }
        }
        self.paste_buffers.clear();
        self.cursor_pos = self.cursor_pos.min(self.input.len());
    }

    // ── Input-mode detection ──────────────────────────────────────────────

    /// Detect the semantic mode of the current buffer based on its prefix.
    pub fn detect_mode(&self) -> InputMode {
        let t = self.input.trim_start();
        if t.starts_with("!!") {
            InputMode::BashCommand { silent: true }
        } else if t.starts_with('!') {
            InputMode::BashCommand { silent: false }
        } else if t.starts_with('/') {
            InputMode::SlashCommand
        } else {
            InputMode::Regular
        }
    }
}

// ── Component impl ────────────────────────────────────────────────────────────

impl Component for Editor {
    /// Render the editor as a single-line (or multi-line) input field.
    ///
    /// Returns the visible text split by `\n`, each line truncated to `width`.
    /// A reverse-video block cursor is rendered at `cursor_pos`.
    fn render(&self, width: u16) -> Vec<RenderedLine> {
        let w = width as usize;
        if w == 0 {
            return vec![String::new()];
        }

        // Build the display string with a visible cursor marker.
        let before = &self.input[..self.cursor_pos.min(self.input.len())];
        let at_cursor = self.input[self.cursor_pos..]
            .chars()
            .next()
            .unwrap_or(' ');
        let after_start = self.cursor_pos
            + at_cursor.len_utf8().min(self.input.len().saturating_sub(self.cursor_pos));
        let after = &self.input[after_start..];
        let display = format!("{before}\x1b[7m{at_cursor}\x1b[27m{after}");

        // Split on newlines and truncate each visual line to `width`.
        display
            .split('\n')
            .map(|line| {
                let visible: String = line.chars().take(w).collect();
                visible
            })
            .collect()
    }

    /// Handle a key event.  Returns `true` for events consumed here,
    /// `false` for events that should bubble up to `TuiApp`.
    ///
    /// Note: `TuiApp::handle_key_input` also dispatches to Editor methods
    /// directly (for UI-coupled keys).  This method covers the pure-editing
    /// subset and is useful for testing and future refactoring.
    fn handle_input(&mut self, key: KeyEvent) -> bool {
        match (key.code, key.modifiers) {
            // ── Text editing (consumed) ───────────────────────────────────
            (KeyCode::Char('u'), KeyModifiers::CONTROL) => { self.delete_to_start(); true }
            (KeyCode::Char('k'), KeyModifiers::CONTROL) => { self.delete_to_end(); true }
            (KeyCode::Char('w'), KeyModifiers::CONTROL) => { self.delete_word_back(); true }
            (KeyCode::Char('z'), KeyModifiers::CONTROL) => { self.undo(); true }
            (KeyCode::Char('y'), KeyModifiers::CONTROL) => { self.redo(); true }
            (KeyCode::Char('a'), KeyModifiers::CONTROL) |
            (KeyCode::Home, _)                          => { self.move_home(); true }
            (KeyCode::Char('e'), KeyModifiers::CONTROL) |
            (KeyCode::End, _)                           => { self.move_end(); true }
            // Word navigation (Alt+Arrow or Ctrl+Arrow)
            (KeyCode::Left,  m) if m.intersects(KeyModifiers::ALT | KeyModifiers::CONTROL) => {
                self.move_word_left(); true
            }
            (KeyCode::Right, m) if m.intersects(KeyModifiers::ALT | KeyModifiers::CONTROL) => {
                self.move_word_right(); true
            }
            (KeyCode::Left, _)                          => { self.move_left(); true }
            (KeyCode::Right, _)                         => { self.move_right(); true }
            (KeyCode::Backspace, _)                     => { self.delete_back(); true }
            (KeyCode::Delete, _)                        => { self.delete_forward(); true }
            (KeyCode::Char(c), m)
                if m == KeyModifiers::NONE || m == KeyModifiers::SHIFT =>
            {
                self.insert_char(c);
                true
            }

            // ── Not consumed — bubble up to TuiApp ───────────────────────
            _ => false,
        }
    }

    fn is_dirty(&self) -> bool {
        true
    }
}
