use tui_textarea::{CursorMove, TextArea, WrapMode};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InputMode {
    Regular,
    BashCommand { silent: bool },
    SlashCommand,
}

#[derive(Debug, Clone)]
pub struct PasteEntry {
    pub id: usize,
    pub text: String,
}

#[derive(Debug, Clone)]
pub struct ImageEntry {
    pub id: usize,
    pub media_type: String,
    pub data: String,
    pub width: u32,
    pub height: u32,
}

pub struct Editor {
    lines: Vec<String>,
    cursor: (usize, usize),
    wrap_mode: WrapMode,
    paste_counter: usize,
    pub paste_buffers: Vec<PasteEntry>,
    image_counter: usize,
    pub paste_images: Vec<ImageEntry>,
    /// Last terminal area the editor was rendered into, captured by
    /// the `EditorComponent::render` impl.  Used by `cursor_position()`
    /// to compute absolute screen coordinates for the IME hardware-
    /// cursor sync path.  `None` until the first render.
    last_render_area: Option<ratatui::layout::Rect>,
}

const PASTE_COLLAPSE_THRESHOLD: usize = 10;
const PASTE_CHAR_THRESHOLD: usize = 1000;

impl Default for Editor {
    fn default() -> Self {
        Self::new()
    }
}

impl Editor {
    pub fn new() -> Self {
        Self {
            lines: vec![String::new()],
            cursor: (0, 0),
            wrap_mode: WrapMode::WordOrGlyph,
            paste_counter: 0,
            paste_buffers: Vec::new(),
            image_counter: 0,
            paste_images: Vec::new(),
            last_render_area: None,
        }
    }

    pub fn cursor(&self) -> (usize, usize) {
        self.cursor
    }

    pub fn text(&self) -> String {
        self.lines.join("\n")
    }

    pub fn is_empty(&self) -> bool {
        self.lines.is_empty() || (self.lines.len() == 1 && self.lines[0].is_empty())
    }

    pub fn set_text(&mut self, text: String) {
        self.lines = text.lines().map(|s| s.to_string()).collect();
        if self.lines.is_empty() {
            self.lines.push(String::new());
        }
        self.cursor = (0, 0);
        self.wrap_mode = WrapMode::WordOrGlyph;
    }

    pub fn cursor_pos(&self) -> usize {
        let (row, col) = self.cursor;
        let mut pos = 0;
        for i in 0..row {
            pos += self.lines[i].len() + 1; // +1 for newline
        }
        pos + col
    }

    pub fn set_cursor_pos(&mut self, pos: usize) {
        let mut current_pos = 0;
        for (row, line) in self.lines.iter().enumerate() {
            let next_pos = current_pos + line.len() + 1;
            if pos < next_pos {
                self.cursor = (row, (pos - current_pos).min(line.len()));
                return;
            }
            current_pos = next_pos;
        }
        if let Some(last_row) = self.lines.len().checked_sub(1) {
            self.cursor = (last_row, self.lines[last_row].len());
        }
    }

    pub fn insert_str_at(&mut self, pos: usize, s: &str) {
        let mut text = self.text();
        text.insert_str(pos, s);
        self.set_text(text);
        self.set_cursor_pos(pos + s.len());
    }

    pub fn remove_char_at(&mut self, pos: usize) {
        let mut text = self.text();
        if pos < text.len() {
            text.remove(pos);
            self.set_text(text);
            self.set_cursor_pos(pos);
        }
    }

    pub fn insert_char_at(&mut self, pos: usize, c: char) {
        let mut text = self.text();
        text.insert(pos, c);
        self.set_text(text);
        self.set_cursor_pos(pos + c.len_utf8());
    }

    pub fn clear(&mut self) {
        self.lines = vec![String::new()];
        self.cursor = (0, 0);
        self.wrap_mode = WrapMode::WordOrGlyph;
    }

    pub fn snapshot(&mut self) {
        // TextArea does its own undo/redo tracking
    }

    pub fn handle_key_event(&mut self, event: crossterm::event::KeyEvent, _max_width: u16) -> bool {
        let mut ta = TextArea::from(self.lines.clone());
        ta.set_wrap_mode(self.wrap_mode);
        let (row, col) = self.cursor;
        ta.move_cursor(CursorMove::Jump(row as u16, col as u16));
        let modified = ta.input(event);
        self.lines = ta.lines().to_vec();
        self.cursor = ta.cursor();
        modified
    }

    pub fn insert_char(&mut self, c: char) {
        let (row, col) = self.cursor;
        self.lines[row].insert(col, c);
        self.cursor = (row, col + c.len_utf8());
    }

    pub fn insert_str(&mut self, s: &str) {
        let (row, col) = self.cursor;
        self.lines[row].insert_str(col, s);
        self.cursor = (row, col + s.len());
    }

    pub fn insert_newline(&mut self) {
        let (row, col) = self.cursor;
        let rest = self.lines[row][col..].to_string();
        self.lines[row].truncate(col);
        self.lines.insert(row + 1, rest);
        self.cursor = (row + 1, 0);
    }

    pub fn handle_paste(&mut self, text: &str) {
        let lines: Vec<&str> = text.lines().collect();
        if lines.len() > PASTE_COLLAPSE_THRESHOLD || text.len() > PASTE_CHAR_THRESHOLD {
            self.paste_counter += 1;
            let id = self.paste_counter;
            self.paste_buffers.push(PasteEntry {
                id,
                text: text.to_string(),
            });
            let marker = format!("[paste #{id}: {} lines]", lines.len());
            self.insert_str(&marker);
            self.insert_newline();
        } else {
            self.insert_str(text);
        }
    }

    pub fn expand_pastes(&mut self) {
        let mut text = self.text();
        for paste in &self.paste_buffers {
            let marker_prefix = format!("[paste #{}:", paste.id);
            if let Some(start) = text.find(&marker_prefix)
                && let Some(end_offset) = text[start..].find(']')
            {
                let end = start + end_offset + 1;
                text.replace_range(start..end, &paste.text);
            }
        }
        self.set_text(text);
        self.paste_buffers.clear();
        self.paste_counter = 0;
    }

    pub fn handle_image_paste(&mut self, media_type: &str, data: String, width: u32, height: u32) {
        self.image_counter += 1;
        let id = self.image_counter;
        self.paste_images.push(ImageEntry {
            id,
            media_type: media_type.to_string(),
            data,
            width,
            height,
        });
        let marker = format!("[image #{id}: {width}x{height}]");
        self.insert_str(&marker);
        self.insert_newline();
    }

    pub fn drain_images(&mut self) -> Vec<ImageEntry> {
        let mut extracted = Vec::new();
        let mut text = self.text();
        let current_images = std::mem::take(&mut self.paste_images);

        for img in current_images {
            let marker_prefix = format!("[image #{}:", img.id);
            if text.contains(&marker_prefix) {
                if let Some(start) = text.find(&marker_prefix)
                    && let Some(end_offset) = text[start..].find(']')
                {
                    let end = start + end_offset + 1;
                    text.replace_range(start..end, "");
                }
                extracted.push(img);
            }
        }
        self.set_text(text);
        self.image_counter = 0;
        extracted
    }

    pub fn detect_mode(&self) -> InputMode {
        let text = self.text();
        if text.starts_with("!!") {
            InputMode::BashCommand { silent: true }
        } else if text.starts_with('!') {
            InputMode::BashCommand { silent: false }
        } else if text.starts_with('/') {
            InputMode::SlashCommand
        } else {
            InputMode::Regular
        }
    }
}

// region:    --- EditorComponent impl
//
// Adapter that exposes [`Editor`] through the host-agnostic
// [`crate::editor_component::EditorComponent`] trait so the TUI event
// loop, render path, and IME cursor sync can target the trait rather
// than the concrete textarea-backed editor.

impl crate::editor_component::EditorComponent for Editor {
    fn render(
        &mut self,
        frame: &mut ratatui::Frame,
        area: ratatui::layout::Rect,
        _colors: &crate::colors::ThemeColors,
    ) {
        self.last_render_area = Some(area);
        let mut ta = TextArea::from(self.lines.clone());
        ta.set_wrap_mode(self.wrap_mode);
        let (row, col) = self.cursor;
        ta.move_cursor(CursorMove::Jump(row as u16, col as u16));
        frame.render_widget(&ta, area);
    }

    fn handle_input(
        &mut self,
        key: crossterm::event::KeyEvent,
        max_width: u16,
    ) -> crate::editor_component::EditorAction {
        use crate::editor_component::EditorAction;
        use crossterm::event::{KeyCode, KeyModifiers};

        match (key.code, key.modifiers) {
            // Plain Enter submits.  Shift+Enter / Alt+Enter still reach
            // the textarea below to insert a newline.
            (KeyCode::Enter, m) if m == KeyModifiers::NONE => {
                self.expand_pastes();
                EditorAction::Submit(self.text())
            }
            (KeyCode::Esc, _) => EditorAction::Cancel,
            _ => {
                let modified = self.handle_key_event(key, max_width);
                if modified {
                    EditorAction::Consumed
                } else {
                    // Let the host route this to global shortcuts /
                    // overlays (Ctrl+P palette, Ctrl+L clear, …).
                    EditorAction::Unhandled(key)
                }
            }
        }
    }

    fn text(&self) -> String {
        Editor::text(self)
    }

    fn set_text(&mut self, text: String) {
        Editor::set_text(self, text);
    }

    fn cursor_position(&self) -> Option<(u16, u16)> {
        let area = self.last_render_area?;
        let (row, col) = self.cursor;
        // Clamp to the rendered area to avoid emitting a MoveTo
        // outside the input region (which terminals interpret
        // unpredictably).
        let x = area
            .x
            .saturating_add(col as u16)
            .min(area.x + area.width.saturating_sub(1));
        let y = area
            .y
            .saturating_add(row as u16)
            .min(area.y + area.height.saturating_sub(1));
        Some((x, y))
    }

    fn is_empty(&self) -> bool {
        Editor::is_empty(self)
    }

    fn clear(&mut self) {
        Editor::clear(self)
    }

    fn cursor_pos(&self) -> usize {
        Editor::cursor_pos(self)
    }

    fn set_cursor_pos(&mut self, pos: usize) {
        Editor::set_cursor_pos(self, pos)
    }

    fn insert_str_at(&mut self, pos: usize, s: &str) {
        Editor::insert_str_at(self, pos, s)
    }

    fn remove_char_at(&mut self, pos: usize) {
        Editor::remove_char_at(self, pos)
    }

    fn insert_char_at(&mut self, pos: usize, c: char) {
        Editor::insert_char_at(self, pos, c)
    }

    fn insert_str(&mut self, s: &str) {
        Editor::insert_str(self, s)
    }

    fn insert_char(&mut self, c: char) {
        Editor::insert_char(self, c)
    }

    fn insert_newline(&mut self) {
        Editor::insert_newline(self)
    }

    fn snapshot(&mut self) {
        Editor::snapshot(self)
    }

    fn handle_paste(&mut self, text: &str) {
        Editor::handle_paste(self, text)
    }

    fn expand_pastes(&mut self) {
        Editor::expand_pastes(self)
    }

    fn mode_hint(&self) -> Option<String> {
        Some(match self.detect_mode() {
            InputMode::Regular => "regular".to_string(),
            InputMode::SlashCommand => "slash".to_string(),
            InputMode::BashCommand { silent } => {
                if silent {
                    "bash:silent".to_string()
                } else {
                    "bash".to_string()
                }
            }
        })
    }
}

// endregion: --- EditorComponent impl

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_textarea_newline() {
        let mut e = Editor::new();
        e.insert_newline();
        assert_eq!(e.text(), "\n");
        assert_eq!(e.cursor(), (1, 0));
    }
}
