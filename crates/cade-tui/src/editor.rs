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

use super::component::{Component, RenderedLine};
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use std::collections::VecDeque;

// -- Input mode

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

// -- Paste entry

/// A collapsed text-paste marker stored for later expansion.
#[derive(Debug, Clone)]
pub struct PasteEntry {
    /// 1-based paste ID shown in the marker token.
    pub id: usize,
    /// The full original pasted text.
    pub text: String,
}

// -- Image entry

/// An image pasted by the user (Ctrl+V / Alt+V).
///
/// Kept in memory alongside the input buffer; extracted by `drain_images()`
/// just before submission and forwarded to the LLM as a base64 attachment.
#[derive(Debug, Clone)]
pub struct ImageEntry {
    /// 1-based image ID matching the `[image #N …]` placeholder in `input`.
    pub id: usize,
    /// IANA media type, e.g. `"image/png"`.
    pub media_type: String,
    /// Base64-encoded image bytes.
    pub data: String,
    /// Pixel dimensions (informational; shown in the placeholder).
    pub width: u32,
    pub height: u32,
}

// -- Editor

/// Standalone multi-line text editor component.
pub struct Editor {
    /// The raw text buffer (UTF-8).
    pub input: String,
    /// Byte-offset cursor position within `input`.
    pub cursor_pos: usize,

    // -- Bracketed paste
    /// Monotonically increasing paste counter (for `[paste #N …]` markers).
    paste_counter: usize,
    /// Stored paste buffers keyed by their marker ID.
    pub paste_buffers: Vec<PasteEntry>,

    // -- Image paste
    /// Monotonically increasing image counter (for `[image #N …]` placeholders).
    image_counter: usize,
    /// Stored image data keyed by their placeholder ID.
    pub paste_images: Vec<ImageEntry>,

    // -- Undo / redo
    /// Snapshots of (input, cursor_pos) taken *before* each edit (max 100).
    /// `undo()` pops the top and restores it.
    undo_stack: VecDeque<(String, usize)>,
    /// States saved by `undo()` so `redo()` can reapply them.
    redo_stack: VecDeque<(String, usize)>,
    /// Last action performed, used for undo coalescing.
    pub last_action: EditorAction,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EditorAction {
    TypeWord,
    Other,
}

/// Maximum number of paste lines shown verbatim before collapsing.
const PASTE_COLLAPSE_THRESHOLD: usize = 10;
/// Maximum number of paste characters shown verbatim before collapsing.
const PASTE_CHAR_THRESHOLD: usize = 1000;
/// Maximum entries kept in the undo / redo stacks.
const UNDO_LIMIT: usize = 100;

impl Default for Editor {
    fn default() -> Self {
        Self::new()
    }
}

impl Editor {
    pub fn new() -> Self {
        Self {
            input: String::new(),
            cursor_pos: 0,
            paste_counter: 0,
            paste_buffers: Vec::new(),
            image_counter: 0,
            paste_images: Vec::new(),
            undo_stack: VecDeque::with_capacity(UNDO_LIMIT),
            redo_stack: VecDeque::with_capacity(UNDO_LIMIT),
            last_action: EditorAction::Other,
        }
    }

    // -- Undo / redo

    /// Save current `(input, cursor_pos)` to the undo stack **before** a
    /// destructive edit.  Clears the redo stack (new edit invalidates
    /// any undone future).  Silently caps the stack at `UNDO_LIMIT`.
    pub fn snapshot(&mut self) {
        if self.undo_stack.len() >= UNDO_LIMIT {
            self.undo_stack.pop_front();
        }
        self.undo_stack
            .push_back((self.input.clone(), self.cursor_pos));
        self.redo_stack.clear();
    }

    /// Undo the last edit.  Returns `true` if a state was restored.
    pub fn undo(&mut self) -> bool {
        if let Some((input, pos)) = self.undo_stack.pop_back() {
            // Save current state so redo() can reapply it.
            if self.redo_stack.len() >= UNDO_LIMIT {
                self.redo_stack.pop_front();
            }
            self.redo_stack
                .push_back((self.input.clone(), self.cursor_pos));
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
            self.undo_stack
                .push_back((self.input.clone(), self.cursor_pos));
            self.input = input;
            self.cursor_pos = pos;
            true
        } else {
            false
        }
    }

    // -- Insert / delete

    /// Insert a character at the current cursor position.
    pub fn insert_char(&mut self, c: char) {
        if c.is_whitespace() || self.last_action != EditorAction::TypeWord {
            self.snapshot();
        }
        self.last_action = EditorAction::TypeWord;
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
        self.last_action = EditorAction::Other;
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
        self.snapshot();
        self.last_action = EditorAction::Other;
        let char_len = self.input[..self.cursor_pos]
            .chars()
            .last()
            .map(|c| c.len_utf8())
            .unwrap_or(1);
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
        self.last_action = EditorAction::Other;
        self.input.remove(self.cursor_pos);
        true
    }

    /// Delete from cursor to start of buffer (Ctrl+U).
    pub fn delete_to_start(&mut self) {
        if self.cursor_pos == 0 {
            return;
        }
        self.snapshot();
        self.last_action = EditorAction::Other;
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
        self.last_action = EditorAction::Other;
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
        self.last_action = EditorAction::Other;
        self.input.drain(start..end);
        self.cursor_pos = start;
    }

    // -- Cursor movement
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
            before.rfind('\n').map(|i| i + 1).unwrap_or(0)
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

    // -- Bulk operations

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

    // -- Bracketed paste

    /// Handle a bracketed-paste event.
    ///
    /// If the pasted text is ≤ `PASTE_COLLAPSE_THRESHOLD` lines it is
    /// inserted verbatim (via `insert_str`, which snapshots).  Otherwise
    /// the full text is stored in `paste_buffers` and a compact marker
    /// `[paste #N +M lines]` is inserted instead.
    pub fn handle_paste(&mut self, text: &str) {
        let line_count = text.lines().count();
        let char_count = text.chars().count();
        if line_count <= PASTE_COLLAPSE_THRESHOLD && char_count <= PASTE_CHAR_THRESHOLD {
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
            let marker = if line_count > PASTE_COLLAPSE_THRESHOLD {
                format!("[paste #{id} +{line_count} lines]")
            } else {
                format!("[paste #{id} {char_count} chars]")
            };
            self.insert_str(&marker); // snapshot happens inside insert_str
        }
    }

    /// Expand all paste markers in the input buffer, replacing each
    /// `[paste #N …]` token with the original pasted text.
    /// Called just before submission — does NOT snapshot.
    pub fn expand_pastes(&mut self) {
        for entry in &self.paste_buffers {
            let marker = format!("[paste #{} +", entry.id);
            if let Some(start) = self.input.find(&marker)
                && let Some(end) = self.input[start..].find(']') {
                    self.input
                        .replace_range(start..start + end + 1, &entry.text);
                }
        }
        self.paste_buffers.clear();
        self.cursor_pos = self.cursor_pos.min(self.input.len());
    }

    // -- Image paste

    /// Record a pasted image and insert a `[image #N: WxH]` placeholder at
    /// the cursor.  The full image data is kept in `paste_images` and
    /// extracted by `drain_images()` just before the message is submitted.
    pub fn handle_image_paste(&mut self, media_type: &str, data: String, width: u32, height: u32) {
        self.image_counter += 1;
        let id = self.image_counter;
        let placeholder = format!("[image #{id}: {width}×{height}]");
        self.paste_images.push(ImageEntry {
            id,
            media_type: media_type.to_string(),
            data,
            width,
            height,
        });
        self.insert_str(&placeholder);
    }

    /// Remove all stored images from the buffer and return them.
    ///
    /// Also strips the placeholder tokens from `input` so the LLM sees clean
    /// text alongside the separate image attachments.
    /// Called just before submission — does NOT snapshot.
    pub fn drain_images(&mut self) -> Vec<ImageEntry> {
        for entry in &self.paste_images {
            let placeholder = format!("[image #{}:", entry.id);
            if let Some(start) = self.input.find(&placeholder)
                && let Some(end) = self.input[start..].find(']') {
                    self.input.drain(start..start + end + 1);
                    self.cursor_pos = self.cursor_pos.min(self.input.len());
                }
        }
        std::mem::take(&mut self.paste_images)
    }

    // -- Input-mode detection

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

// -- Component impl

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
        let at_cursor = self.input[self.cursor_pos..].chars().next().unwrap_or(' ');
        let after_start = self.cursor_pos
            + at_cursor
                .len_utf8()
                .min(self.input.len().saturating_sub(self.cursor_pos));
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
            // -- Text editing (consumed)
            (KeyCode::Char('u'), KeyModifiers::CONTROL) => {
                self.delete_to_start();
                true
            }
            (KeyCode::Char('k'), KeyModifiers::CONTROL) => {
                self.delete_to_end();
                true
            }
            (KeyCode::Char('w'), KeyModifiers::CONTROL) => {
                self.delete_word_back();
                true
            }
            (KeyCode::Char('z'), KeyModifiers::CONTROL) => {
                self.undo();
                true
            }
            (KeyCode::Char('y'), KeyModifiers::CONTROL) => {
                self.redo();
                true
            }
            (KeyCode::Char('a'), KeyModifiers::CONTROL) | (KeyCode::Home, _) => {
                self.move_home();
                true
            }
            (KeyCode::Char('e'), KeyModifiers::CONTROL) | (KeyCode::End, _) => {
                self.move_end();
                true
            }
            // Word navigation (Alt+Arrow or Ctrl+Arrow)
            (KeyCode::Left, m) if m.intersects(KeyModifiers::ALT | KeyModifiers::CONTROL) => {
                self.move_word_left();
                true
            }
            (KeyCode::Right, m) if m.intersects(KeyModifiers::ALT | KeyModifiers::CONTROL) => {
                self.move_word_right();
                true
            }
            (KeyCode::Left, _) => {
                self.move_left();
                true
            }
            (KeyCode::Right, _) => {
                self.move_right();
                true
            }
            (KeyCode::Backspace, _) => {
                self.delete_back();
                true
            }
            (KeyCode::Delete, _) => {
                self.delete_forward();
                true
            }
            (KeyCode::Char(c), m) if m == KeyModifiers::NONE || m == KeyModifiers::SHIFT => {
                self.insert_char(c);
                true
            }

            // -- Not consumed — bubble up to TuiApp
            _ => false,
        }
    }

    fn is_dirty(&self) -> bool {
        true
    }
}

// region:    --- Tests

#[cfg(test)]
mod tests {
    #[allow(unused)]
    type Result<T> = core::result::Result<T, Box<dyn std::error::Error>>; // For tests.

    use super::*;

    #[test]
    fn test_editor_insert_and_delete() {
        // -- Setup & Fixtures
        let mut editor = Editor::new();

        // -- Exec & Check
        editor.insert_str("hello");
        assert_eq!(editor.input, "hello");
        assert_eq!(editor.cursor_pos, 5);

        editor.delete_back();
        assert_eq!(editor.input, "hell");
        assert_eq!(editor.cursor_pos, 4);

        editor.move_left();
        editor.delete_forward();
        assert_eq!(editor.input, "hel");
        assert_eq!(editor.cursor_pos, 3);
    }

    #[test]
    fn test_editor_undo_redo() {
        // -- Setup & Fixtures
        let mut editor = Editor::new();
        editor.insert_str("hello");
        assert_eq!(editor.input, "hello");

        // -- Exec & Check
        editor.undo();
        assert_eq!(editor.input, "");

        editor.redo();
        assert_eq!(editor.input, "hello");
    }

    #[test]
    fn test_editor_word_movement() {
        // -- Setup & Fixtures
        let mut editor = Editor::new();
        editor.insert_str("one two three");

        // -- Exec & Check
        editor.move_word_left();
        assert_eq!(editor.cursor_pos, 8);

        editor.move_word_left();
        assert_eq!(editor.cursor_pos, 4);

        editor.move_word_right();
        assert_eq!(editor.cursor_pos, 8);
    }

    #[test]
    fn test_editor_delete_to_end() {
        // -- Setup & Fixtures
        let mut editor = Editor::new();
        editor.insert_str("hello world\nnewline");
        editor.cursor_pos = 5;

        // -- Exec
        editor.delete_to_end();

        // -- Check
        assert_eq!(editor.input, "hello\nnewline");

        editor.undo();
        assert_eq!(editor.input, "hello world\nnewline");
    }
}

// endregion: --- Tests
