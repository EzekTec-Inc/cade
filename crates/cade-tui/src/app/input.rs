//! User input loop — read_input and handle_key_input.

use crossterm::event::{
    self, Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers,
    MouseEventKind,
};

use crate::Result;

use super::{FIXED_ROWS, MAX_INPUT_ROWS, PickerState, ToastLevel, TuiApp};
use super::layout::cursor::{calc_visual_cursor, find_cursor_at_visual_row_col, input_mode_badge};

impl TuiApp {
    // -- Input loop

    /// Block until the user submits input or presses Ctrl+D.
    /// Returns `None` on Ctrl+D (exit signal).
    pub fn read_input(
        &mut self,
        history: &mut [String],
        hist_idx: &mut Option<usize>,
    ) -> Result<Option<String>> {
        *hist_idx = None;

        self.draw()?;

        loop {
            // Redraw when dirty or when a toast needs expiry check.
            if self.draw_dirty || self.toast.is_some() {
                self.draw()?;
            }

            // 50 ms poll: allows animation ticks without burning CPU.
            if !event::poll(std::time::Duration::from_millis(50))? {
                // Background-subagent completion toast (Option 2).
                // Surface a single toast when pending count changes so the
                // user knows there are results waiting; the actual drain
                // still happens in the outer REPL loop after submit.
                if let Some(getter) = self.bg_pending_count.as_ref() {
                    let pending = getter();
                    let mut taken_toast = self.toast.take();
                    let wrote = super::tick_bg_pending_toast(
                        pending,
                        &mut self.bg_last_announced,
                        &mut taken_toast,
                    );
                    self.toast = taken_toast;
                    if wrote {
                        self.draw_dirty = true;
                    }
                }
                continue;
            }
            match event::read()? {
                Event::Key(k) if k.kind == KeyEventKind::Press => {
                    let was_empty = self.editor.is_empty();
                    if self.active_question.is_some() {
                        self.handle_question_key(k);
                    } else if let Some(result) = self.handle_key_input(k, history, hist_idx)? {
                        return Ok(result);
                    } else {
                        if was_empty && !self.editor.is_empty() {
                            self.last_status = None;
                        }
                        self.draw()?;
                    }
                }
                Event::Paste(text) => {
                    // Bracketed paste: the terminal wrapped the pasted content
                    // in paste-start / paste-end markers so crossterm delivers
                    // it as a single string.
                    //
                    // Drag-onto-terminal: many terminals (Kitty, WezTerm,
                    // iTerm2, Windows Terminal) convert a dragged file into a
                    // bracketed paste of its URI (`file:///path/to/file`) or
                    // plain path.  If the pasted text looks like a single image
                    // file path we try to load it as an image instead of text.
                    let trimmed = text.trim();
                    if self.try_paste_image_file_path(trimmed) {
                        // Image file was loaded — skip normal text paste.
                    } else {
                        self.editor.handle_paste(&text);
                        self.last_status = None;
                    }
                    self.draw()?;
                }
                Event::Resize(_, _) => {
                    self.draw()?;
                }
                Event::Mouse(m) => match m.kind {
                    MouseEventKind::ScrollUp => {
                        self.follow = false;
                        self.scroll_target = self.scroll_target.saturating_add(3);
                        self.draw_dirty = true;
                        self.draw()?;
                    }
                    MouseEventKind::ScrollDown => {
                        self.scroll_target = self.scroll_target.saturating_sub(3);
                        if self.scroll_target == 0 {
                            self.follow = true;
                            self.pending_lines = 0;
                        }
                        self.draw_dirty = true;
                        self.draw()?;
                    }
                    _ => {}
                },
                _ => {}
            }
        }
    }

    fn handle_key_input(
        &mut self,
        k: KeyEvent,
        history: &mut [String],
        hist_idx: &mut Option<usize>,
    ) -> Result<Option<Option<String>>> {
        // Some(None)        = Ctrl+D (exit)
        // Some(Some(s))     = line submitted
        // None              = continue reading

        // -- Summary overlay routing
        if self.summary_overlay.is_some() {
            match (k.code, k.modifiers) {
                (KeyCode::Esc, _) | (KeyCode::Enter, _) | (KeyCode::Char('c'), KeyModifiers::CONTROL) => {
                    self.summary_overlay = None;
                }
                (KeyCode::Up, _) | (KeyCode::Char('k'), _) => {
                    if let Some(su) = &mut self.summary_overlay {
                        su.scroll_y = su.scroll_y.saturating_sub(1);
                    }
                }
                (KeyCode::Down, _) | (KeyCode::Char('j'), _) => {
                    if let Some(su) = &mut self.summary_overlay {
                        su.scroll_y = su.scroll_y.saturating_add(1);
                    }
                }
                (KeyCode::PageUp, _) => {
                    if let Some(su) = &mut self.summary_overlay {
                        su.scroll_y = su.scroll_y.saturating_sub(20);
                    }
                }
                (KeyCode::PageDown, _) => {
                    if let Some(su) = &mut self.summary_overlay {
                        su.scroll_y = su.scroll_y.saturating_add(20);
                    }
                }
                _ => {}
            }
            let _ = self.draw();
            return Ok(None);
        }

        // -- Command palette routing (Ctrl+P overlay)
        if self.command_palette.is_some() {
            match (k.code, k.modifiers) {
                (KeyCode::Esc, _)
                | (KeyCode::Char('c'), KeyModifiers::CONTROL) => {
                    self.command_palette = None;
                }
                (KeyCode::Up, _) | (KeyCode::BackTab, _) => {
                    if let Some(cp) = &mut self.command_palette {
                        cp.cursor_up();
                    }
                }
                (KeyCode::Down, _) | (KeyCode::Tab, _) => {
                    if let Some(cp) = &mut self.command_palette {
                        cp.cursor_down();
                    }
                }
                (KeyCode::Enter, _) => {
                    if let Some(cp) = self.command_palette.take()
                        && let Some(cmd) = cp.selected_command() {
                            let cmd = format!("/{}", cmd);
                            return Ok(Some(Some(cmd)));
                        }
                }
                (KeyCode::Backspace, _) => {
                    if let Some(cp) = &mut self.command_palette {
                        if cp.query.is_empty() {
                            self.command_palette = None;
                        } else {
                            cp.pop_char();
                        }
                    }
                }
                (KeyCode::Char(c), m) if m == KeyModifiers::NONE || m == KeyModifiers::SHIFT => {
                    if let Some(cp) = &mut self.command_palette {
                        cp.push_char(c);
                    }
                }
                _ => {}
            }
            let _ = self.draw();
            return Ok(None);
        }

        // -- A-01b: theme picker routing
        if self.theme_picker.is_some() {
            match (k.code, k.modifiers) {
                (KeyCode::Esc, _)
                | (KeyCode::Char('q'), KeyModifiers::NONE)
                | (KeyCode::Char('c'), KeyModifiers::CONTROL) => {
                    if let Some(tp) = self.theme_picker.take() {
                        self.apply_theme(tp.original_theme);
                    }
                }
                (KeyCode::Up, _) | (KeyCode::BackTab, _) => {
                    if let Some(tp) = &mut self.theme_picker {
                        tp.cursor = tp.cursor.saturating_sub(1);
                        if !tp.filtered_indices.is_empty() {
                            let idx = tp.filtered_indices[tp.cursor];
                            let t = &tp.themes[idx];
                            let colors = crate::colors::ThemeColors::builtin_by_name(&t.name)
                                .unwrap_or_else(|| crate::colors::ThemeColors::from_theme(t));
                            self.apply_theme(colors);
                        }
                    }
                }
                (KeyCode::Down, _) | (KeyCode::Tab, _) => {
                    if let Some(tp) = &mut self.theme_picker {
                        if !tp.filtered_indices.is_empty()
                            && tp.cursor + 1 < tp.filtered_indices.len()
                        {
                            tp.cursor += 1;
                        }
                        if !tp.filtered_indices.is_empty() {
                            let idx = tp.filtered_indices[tp.cursor];
                            let t = &tp.themes[idx];
                            let colors = crate::colors::ThemeColors::builtin_by_name(&t.name)
                                .unwrap_or_else(|| crate::colors::ThemeColors::from_theme(t));
                            self.apply_theme(colors);
                        }
                    }
                }
                (KeyCode::Enter, _) => {
                    if let Some(tp) = self.theme_picker.take()
                        && !tp.filtered_indices.is_empty()
                    {
                        let t = &tp.themes[tp.filtered_indices[tp.cursor]];
                        return Ok(Some(Some(format!("/theme {}", t.name))));
                    }
                }
                (KeyCode::Backspace, _) => {
                    if self.theme_picker.is_some() {
                        self.theme_picker.as_mut().unwrap().query.pop();
                        self.update_theme_picker_filter();
                    }
                }
                (KeyCode::Char(c), m) if m == KeyModifiers::NONE || m == KeyModifiers::SHIFT => {
                    if self.theme_picker.is_some() {
                        self.theme_picker.as_mut().unwrap().query.push(c);
                        self.update_theme_picker_filter();
                    }
                }
                _ => {}
            }
            let _ = self.draw();
            return Ok(None);
        }

        // -- A-01: file picker routing
        if self.picker.is_some() {
            match (k.code, k.modifiers) {
                (KeyCode::Esc, _) => {
                    self.picker = None;
                }
                (KeyCode::Up, _) => {
                    if let Some(pk) = &mut self.picker
                        && pk.cursor > 0
                    {
                        pk.cursor -= 1;
                    }
                }
                (KeyCode::Down, _) => {
                    if let Some(pk) = &mut self.picker
                        && !pk.matches.is_empty()
                        && pk.cursor + 1 < pk.matches.len()
                    {
                        pk.cursor += 1;
                    }
                }
                (KeyCode::Enter, m) if m == KeyModifiers::NONE => {
                    if let Some(pk) = self.picker.take()
                        && let Some(selected) = pk.matches.get(pk.cursor).cloned()
                    {
                        self.editor.snapshot();
                        let query_end = pk.at_pos + 1 + pk.query.len();
                        let drain_end = query_end.min(self.editor.text().len());
                        let mut text = self.editor.text();
                        text.drain(pk.at_pos..drain_end);
                        self.editor.set_text(text);
                        self.editor.insert_str_at(pk.at_pos, &selected);
                        self.editor.set_cursor_pos(pk.at_pos + selected.len());
                    }
                    // dismiss whether or not a match was selected
                }
                (KeyCode::Backspace, _) => {
                    if let Some(pk) = &mut self.picker {
                        if pk.query.is_empty() {
                            // Delete the @ and dismiss
                            if pk.at_pos < self.editor.text().len() {
                                self.editor.remove_char_at(pk.at_pos);
                                self.editor.set_cursor_pos(pk.at_pos);
                            }
                            self.picker = None;
                        } else {
                            // Remove last query char from both query and input
                            let query_end = pk.at_pos + 1 + pk.query.len();
                            let remove_at = query_end.saturating_sub(1);
                            if remove_at < self.editor.text().len() {
                                self.editor.remove_char_at(remove_at);
                            }
                            pk.query.pop();
                            pk.cursor = 0;
                            pk.matches = self.file_ac.collect_files(&pk.query);
                        }
                    }
                }
                (KeyCode::Char(c), m) if m == KeyModifiers::NONE || m == KeyModifiers::SHIFT => {
                    // Append char to both input and picker query
                    if let Some(pk) = &mut self.picker {
                        let query_end = pk.at_pos + 1 + pk.query.len();
                        self.editor.insert_char_at(query_end, c);
                        self.editor.set_cursor_pos(query_end + c.len_utf8());
                        pk.query.push(c);
                        pk.cursor = 0;
                        pk.matches = self.file_ac.collect_files(&pk.query);
                    }
                }
                _ => {}
            }
            let _ = self.draw();
            return Ok(None);
        }

        match (k.code, k.modifiers) {
            // -- Submit
            // Alt+Enter  — universal cross-terminal newline.
            // Shift+Enter — kitty keyboard protocol terminals (Kitty, WezTerm, Ghostty).
            // Ctrl+Enter  — Windows Terminal (which reports this as CONTROL+Enter).
            (KeyCode::Enter, m) if is_newline_shortcut(m) =>
            {
                self.editor.insert_newline();
            }
            (KeyCode::Enter, _) => {
                // Expand any collapsed paste markers back to full text,
                // then drain any pasted images (stripping their placeholders)
                // into pending_submit_images for repl.rs to pick up.
                self.editor.expand_pastes();
                self.pending_submit_images = self.editor.drain_images();
                let line = self.editor.text();
                self.editor.clear();
                self.scroll_instant(0); // snap to bottom on submit
                self.pending_lines = 0; // user is following the conversation
                return Ok(Some(Some(line)));
            }

            // -- Exit
            (KeyCode::Char('d'), KeyModifiers::CONTROL) if self.editor.is_empty() => {
                return Ok(Some(None));
            }

            // -- Cancel / clear
            // Ctrl+C at the idle prompt: clear the input line if not empty.
            // If empty, exit cleanly (acts like Ctrl+D).
            (KeyCode::Char('c'), KeyModifiers::CONTROL) => {
                if self.editor.is_empty() {
                    return Ok(Some(None));
                } else {
                    self.editor.clear();
                    return Ok(None);
                }
            }
            (KeyCode::Esc, _) => {
                self.editor.clear();
            }

            // -- Edit shortcuts
            (KeyCode::Char('l'), KeyModifiers::CONTROL) => {
                self.scroll_instant(0);
                self.follow = true;
                self.pending_lines = 0;
                let _ = self.draw();
            }

            // -- History / cursor-up
            // When the cursor is NOT on the first visual row of the input,
            // Up/Down move the cursor one visual row up/down within the
            // multiline buffer.  Only when already on the first/last visual
            // row do we switch to history navigation.
            (KeyCode::Up, _) => {
                let available_w = self.term_width.saturating_sub(2).max(1);
                let (badge_text, _) = input_mode_badge(self.editor.detect_mode(), &self.colors);
                let input_prefix_w = badge_text.chars().count() as u16 + 1 + 2;
                let before = &self.editor.text()[..self.editor.cursor_pos()];
                let (cur_row, cur_col) = calc_visual_cursor(before, available_w, input_prefix_w);

                if cur_row == 0 {
                    // Already on the first visual row → history navigation
                    if !history.is_empty() {
                        let new_idx = match *hist_idx {
                            None => history.len() - 1,
                            Some(i) if i > 0 => i - 1,
                            Some(i) => i,
                        };
                        *hist_idx = Some(new_idx);
                        self.editor.set_text(history[new_idx].clone());
                        self.editor.set_cursor_pos(self.editor.text().len());
                    }
                } else {
                    // Move cursor up one visual row: target column = cur_col
                    // Walk backwards through the byte string to find the char
                    // at (cur_row-1, cur_col).
                    let target_row = cur_row - 1;
                    // Rebuild visual-row byte-offset map
                    let new_pos = find_cursor_at_visual_row_col(
                        &self.editor.text(),
                        available_w,
                        input_prefix_w,
                        target_row,
                        cur_col,
                    );
                    self.editor.set_cursor_pos(new_pos);
                }
            }
            (KeyCode::Down, _) => {
                let available_w = self.term_width.saturating_sub(2).max(1);
                let (badge_text, _) = input_mode_badge(self.editor.detect_mode(), &self.colors);
                let input_prefix_w = badge_text.chars().count() as u16 + 1 + 2;
                let total_rows = {
                    let (tr, _) =
                        calc_visual_cursor(&self.editor.text(), available_w, input_prefix_w);
                    tr
                };
                let before = &self.editor.text()[..self.editor.cursor_pos()];
                let (cur_row, cur_col) = calc_visual_cursor(before, available_w, input_prefix_w);

                if cur_row >= total_rows {
                    // Already on the last visual row → history navigation
                    if let Some(i) = *hist_idx {
                        if i + 1 < history.len() {
                            *hist_idx = Some(i + 1);
                            self.editor.set_text(history[i + 1].clone());
                            self.editor.set_cursor_pos(self.editor.text().len());
                        } else {
                            *hist_idx = None;
                            self.editor.clear();
                            self.editor.set_cursor_pos(0);
                        }
                    }
                } else {
                    let target_row = cur_row + 1;
                    let new_pos = find_cursor_at_visual_row_col(
                        &self.editor.text(),
                        available_w,
                        input_prefix_w,
                        target_row,
                        cur_col,
                    );
                    self.editor.set_cursor_pos(new_pos);
                }
            }

            // -- Timeline navigation / content scroll
            (KeyCode::Char('K'), _) => {
                self.follow = false;
                self.scroll_target = self.scroll_target.saturating_add(10);
                self.draw_dirty = true;
            }
            (KeyCode::Char('J'), _) => {
                self.scroll_target = 0;
                self.follow = true;
                self.pending_lines = 0;
                self.draw_dirty = true;
            }
            (KeyCode::PageUp, _) => {
                self.follow = false;
                let vh = crossterm::terminal::size()
                    .map(|(_, h)| h.saturating_sub(FIXED_ROWS + MAX_INPUT_ROWS))
                    .unwrap_or(20);
                self.scroll_target = scroll_page_up(self.scroll_target, vh);
                self.draw_dirty = true;
            }
            (KeyCode::PageDown, _) => {
                let vh = crossterm::terminal::size()
                    .map(|(_, h)| h.saturating_sub(FIXED_ROWS + MAX_INPUT_ROWS))
                    .unwrap_or(20);
                let (new_target, should_follow) = scroll_page_down(self.scroll_target, vh);
                self.scroll_target = new_target;
                if should_follow {
                    self.follow = true;
                    self.pending_lines = 0;
                }
                self.draw_dirty = true;
            }

            // -- Mode cycle / path completion
            (KeyCode::Tab, _) => {
                // I-02: if cursor is on a path token, complete it; otherwise
                // fall through to the mode-cycle sentinel.
                if let Some((new_input, new_cursor)) = self
                    .file_ac
                    .complete_path(&self.editor.text(), self.editor.cursor_pos())
                {
                    self.editor.snapshot();
                    self.editor.set_text(new_input);
                    self.editor.set_cursor_pos(new_cursor);
                } else {
                    self.scroll_instant(0);
                    return Ok(Some(Some("__TAB__".to_string())));
                }
            }
            (KeyCode::BackTab, _) => {
                self.scroll_instant(0);
                return Ok(Some(Some("__BACKTAB__".to_string())));
            }

            // -- Expand/Collapse Tool Outputs
            (KeyCode::Char('o'), KeyModifiers::CONTROL) => {
                self.expand_all = !self.expand_all;
                self.show_toast(
                    if self.expand_all {
                        "Expanded all blocks"
                    } else {
                        "Collapsed all blocks"
                    },
                    ToastLevel::Info,
                );
            }

            // -- Command Palette (Ctrl+P)
            (KeyCode::Char('p'), KeyModifiers::CONTROL) => {
                self.command_palette = Some(super::command_palette::CommandPaletteState::new());
            }

            // -- Image / clipboard paste
            // Ctrl+V (universal) or Alt+V (Windows Terminal fallback):
            // query the OS clipboard for image data; fall through silently if
            // no image is present (text pastes arrive via Event::Paste).
            (KeyCode::Char('v'), m) if m == KeyModifiers::CONTROL || m == KeyModifiers::ALT => {
                self.try_paste_clipboard_image();
                // don't consume — if no image was found the keypress is silently ignored
            }

            _ => {
                self.editor.handle_key_event(k, self.last_input_width);
                if let KeyCode::Char('@') = k.code
                    && self.picker.is_none()
                {
                    let at_pos = self.editor.cursor_pos().saturating_sub(1);
                    let matches = self.file_ac.collect_files("");
                    self.picker = Some(PickerState {
                        at_pos,
                        query: String::new(),
                        matches,
                        cursor: 0,
                    });
                }
            }
        }
        Ok(None)
    }
}

pub fn is_newline_shortcut(m: KeyModifiers) -> bool {
    m == KeyModifiers::ALT
        || m == KeyModifiers::SHIFT
        || m == KeyModifiers::CONTROL
        || m == (KeyModifiers::SHIFT | KeyModifiers::ALT)
        || m == (KeyModifiers::CONTROL | KeyModifiers::SHIFT)
}

/// Compute the new scroll position after a PageUp keypress.
/// `viewport_h` is the visible content height in terminal rows.
pub(crate) fn scroll_page_up(current: usize, viewport_h: u16) -> usize {
    let step = (viewport_h as usize).max(1);
    current.saturating_add(step)
}

/// Compute the new scroll position after a PageDown keypress.
/// Returns `(new_scroll, should_follow)`.
pub(crate) fn scroll_page_down(current: usize, viewport_h: u16) -> (usize, bool) {
    let step = (viewport_h as usize).max(1);
    let new = current.saturating_sub(step);
    (new, new == 0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crossterm::event::KeyModifiers;

    #[test]
    fn test_is_newline_shortcut() {
        assert!(is_newline_shortcut(KeyModifiers::SHIFT), "Shift+Enter should be recognized as a newline shortcut");
        assert!(is_newline_shortcut(KeyModifiers::ALT), "Alt+Enter should be recognized as a newline shortcut");
        assert!(is_newline_shortcut(KeyModifiers::CONTROL), "Ctrl+Enter should be recognized as a newline shortcut");
        assert!(is_newline_shortcut(KeyModifiers::SHIFT | KeyModifiers::CONTROL), "Ctrl+Shift+Enter should be recognized");
        assert!(is_newline_shortcut(KeyModifiers::SHIFT | KeyModifiers::ALT), "Alt+Shift+Enter should be recognized");
        assert!(!is_newline_shortcut(KeyModifiers::NONE), "Plain Enter should not be recognized as a newline shortcut");
    }

    #[test]
    fn test_scroll_page_up_from_bottom() {
        // At bottom (scroll=0), PageUp should jump up by viewport height.
        assert_eq!(scroll_page_up(0, 40), 40);
    }

    #[test]
    fn test_scroll_page_up_already_scrolled() {
        // Already scrolled 20 lines up, viewport=40 → should be at 60.
        assert_eq!(scroll_page_up(20, 40), 60);
    }

    #[test]
    fn test_scroll_page_up_zero_viewport() {
        // Edge case: viewport_h=0 → step should be at least 1.
        assert_eq!(scroll_page_up(5, 0), 6);
    }

    #[test]
    fn test_scroll_page_down_to_bottom() {
        // Scrolled up 30, viewport=40 → should snap to 0 (bottom), follow=true.
        let (new, follow) = scroll_page_down(30, 40);
        assert_eq!(new, 0);
        assert!(follow);
    }

    #[test]
    fn test_scroll_page_down_partial() {
        // Scrolled up 60, viewport=40 → should be at 20, follow=false.
        let (new, follow) = scroll_page_down(60, 40);
        assert_eq!(new, 20);
        assert!(!follow);
    }

    #[test]
    fn test_scroll_page_down_already_at_bottom() {
        // Already at bottom → stays at 0, follow=true.
        let (new, follow) = scroll_page_down(0, 40);
        assert_eq!(new, 0);
        assert!(follow);
    }

    #[test]
    fn test_scroll_page_down_zero_viewport() {
        // Edge case: viewport_h=0 → step=1, scroll 5→4.
        let (new, follow) = scroll_page_down(5, 0);
        assert_eq!(new, 4);
        assert!(!follow);
    }
}
