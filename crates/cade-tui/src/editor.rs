use tui_textarea::{TextArea, Input, Key};

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

pub struct Editor<'a> {
    pub textarea: TextArea<'a>,
    paste_counter: usize,
    pub paste_buffers: Vec<PasteEntry>,
    image_counter: usize,
    pub paste_images: Vec<ImageEntry>,
}

const PASTE_COLLAPSE_THRESHOLD: usize = 10;
const PASTE_CHAR_THRESHOLD: usize = 1000;

impl Default for Editor<'static> {
    fn default() -> Self {
        Self::new()
    }
}

impl<'a> Editor<'a> {
    pub fn new() -> Self {
        Self {
            textarea: TextArea::default(),
            paste_counter: 0,
            paste_buffers: Vec::new(),
            image_counter: 0,
            paste_images: Vec::new(),
        }
    }

    pub fn text(&self) -> String {
        self.textarea.lines().join("\n")
    }

    pub fn is_empty(&self) -> bool {
        let lines = self.textarea.lines();
        lines.is_empty() || (lines.len() == 1 && lines[0].is_empty())
    }

    pub fn set_text(&mut self, text: String) {
        self.textarea = TextArea::from(text.lines().map(|s| s.to_string()));
    }

    pub fn cursor_pos(&self) -> usize {
        let (row, col) = self.textarea.cursor();
        let lines = self.textarea.lines();
        let mut pos = 0;
        for i in 0..row {
            pos += lines[i].len() + 1; // +1 for newline
        }
        pos + col
    }

    pub fn set_cursor_pos(&mut self, pos: usize) {
        let lines = self.textarea.lines();
        let mut current_pos = 0;
        for (row, line) in lines.iter().enumerate() {
            let next_pos = current_pos + line.len() + 1;
            if pos < next_pos {
                self.textarea.move_cursor(tui_textarea::CursorMove::Jump(row as u16, (pos - current_pos) as u16));
                return;
            }
            current_pos = next_pos;
        }
        if let Some(last_row) = lines.len().checked_sub(1) {
            self.textarea.move_cursor(tui_textarea::CursorMove::Jump(last_row as u16, lines[last_row].len() as u16));
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
        self.textarea = TextArea::default();
    }

    pub fn snapshot(&mut self) {
        // TextArea does its own undo/redo tracking
    }

    pub fn handle_key_event(&mut self, event: crossterm::event::KeyEvent, max_width: u16) -> bool {
        let modified = self.textarea.input(event);
        if modified && max_width > 0 {
            let (row, col) = self.textarea.cursor();
            if col as u16 >= max_width.saturating_sub(2) {
                let lines = self.textarea.lines();
                if let Some(line) = lines.get(row) {
                    let chars: Vec<char> = line.chars().collect();
                    if let Some(last_space_idx) = chars[..col].iter().rposition(|&c| c == ' ') {
                        let go_back = col - last_space_idx;
                        for _ in 0..go_back {
                            self.textarea.move_cursor(tui_textarea::CursorMove::Back);
                        }
                        self.textarea.delete_next_char();
                        self.textarea.insert_newline();
                        for _ in 0..(go_back - 1) {
                            self.textarea.move_cursor(tui_textarea::CursorMove::Forward);
                        }
                    } else {
                        self.textarea.insert_newline();
                    }
                }
            }
        }
        modified
    }

    pub fn insert_char(&mut self, c: char) {
        self.textarea.input(Input { key: Key::Char(c), ctrl: false, alt: false, shift: false });
    }

    pub fn insert_str(&mut self, s: &str) {
        self.textarea.insert_str(s);
    }

    pub fn insert_newline(&mut self) {
        self.textarea.insert_newline();
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
            if let Some(start) = text.find(&marker_prefix) {
                if let Some(end_offset) = text[start..].find(']') {
                    let end = start + end_offset + 1;
                    text.replace_range(start..end, &paste.text);
                }
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
                if let Some(start) = text.find(&marker_prefix) {
                    if let Some(end_offset) = text[start..].find(']') {
                        let end = start + end_offset + 1;
                        text.replace_range(start..end, "");
                    }
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