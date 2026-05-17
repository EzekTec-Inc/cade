//! User input loop — read_input and handle_key_input.

use crossterm::event::{
    self, Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers, MouseEventKind,
};

use crate::Result;

use super::layout::cursor::{calc_visual_cursor, find_cursor_at_visual_row_col, input_mode_badge};
use super::{FilePickerAction, PickerState, ThemePickerAction, ToastLevel, TuiApp};

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
            if self.draw_dirty || self.toast.is_some() || self.slots.requires_tick() {
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
                    if let Some(result) = self.handle_key_input(k, history, hist_idx)? {
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
                    MouseEventKind::ScrollUp | MouseEventKind::ScrollDown => {
                        self.handle_scroll_mouse(m.kind);
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

        // -- Dynamic overlay stack (Phase 3: highest priority)
        if let Some(overlay) = self.overlays.last_mut() {
            use crate::overlay_component::OverlayInputResult;
            let result = overlay.handle_input(k);

            // Drain side effects (preview, insert, etc.) on every dispatch.
            let action = overlay.take_result();

            match result {
                OverlayInputResult::Dismiss => {
                    // Pop the overlay, then process its final action.
                    if let Some(mut popped) = self.overlays.pop() {
                        // Drain final result if handle_input didn't already produce one.
                        let final_action = action.or_else(|| popped.take_result());
                        if let Some(any_val) = final_action {
                            return self.process_overlay_action(any_val);
                        }
                    }
                }
                OverlayInputResult::Consumed => {
                    if let Some(any_val) = action {
                        let _ = self.process_overlay_action(any_val);
                    }
                }
                OverlayInputResult::NotHandled => {
                    // Fall through to legacy handlers below.
                }
            }

            if !matches!(result, OverlayInputResult::NotHandled) {
                self.draw_dirty = true;
                let _ = self.draw();
                return Ok(None);
            }
        }

        // Legacy overlay dispatch blocks removed — all four overlays
        // (summary, command palette, theme picker, file picker) are now
        // handled by the dynamic overlay stack above (Phase 3).

        // -- UI extension slot input (Phase 4)
        // Give installed slot widgets a chance to consume the key event.
        // Slots are passive by default (handle_input returns false).
        {
            use crate::slots::UiSlot;
            for slot in [UiSlot::Sidebar, UiSlot::Header, UiSlot::Footer] {
                if let Some(widget) = self.slots.get_mut(slot)
                    && widget.handle_input(k)
                {
                    self.draw_dirty = true;
                    let _ = self.draw();
                    return Ok(None);
                }
            }
        }

        // -- Lua global keybindings
        if let Some(lua) = &self.lua_engine {
            let mut key_str = String::new();
            if k.modifiers.contains(crossterm::event::KeyModifiers::CONTROL) {
                key_str.push_str("C-");
            }
            if k.modifiers.contains(crossterm::event::KeyModifiers::ALT) {
                key_str.push_str("A-");
            }
            if k.modifiers.contains(crossterm::event::KeyModifiers::SHIFT) {
                key_str.push_str("S-");
            }
            match k.code {
                crossterm::event::KeyCode::Char(c) => key_str.push(c),
                crossterm::event::KeyCode::Enter => key_str.push_str("Enter"),
                crossterm::event::KeyCode::Esc => key_str.push_str("Esc"),
                crossterm::event::KeyCode::Tab => key_str.push_str("Tab"),
                crossterm::event::KeyCode::Backspace => key_str.push_str("Backspace"),
                crossterm::event::KeyCode::Delete => key_str.push_str("Delete"),
                crossterm::event::KeyCode::Up => key_str.push_str("Up"),
                crossterm::event::KeyCode::Down => key_str.push_str("Down"),
                crossterm::event::KeyCode::Left => key_str.push_str("Left"),
                crossterm::event::KeyCode::Right => key_str.push_str("Right"),
                _ => {}
            }
            
            if !key_str.is_empty() && lua.handle_keybinding(&key_str) {
                let has_queued_cmd = !lua.command_queue.lock().unwrap().is_empty() || !lua.tool_queue.lock().unwrap().is_empty();
                self.draw_dirty = true;
                let _ = self.draw();
                if has_queued_cmd {
                    return Ok(Some(Some(String::new())));
                }
                return Ok(None);
            }
        }

        match (k.code, k.modifiers) {
            // -- Submit
            // Alt+Enter  — universal cross-terminal newline.
            // Shift+Enter — kitty keyboard protocol terminals (Kitty, WezTerm, Ghostty).
            // Ctrl+Enter  — Windows Terminal (which reports this as CONTROL+Enter).
            (KeyCode::Enter, m) if is_newline_shortcut(m) => {
                self.editor.insert_newline();
            }
            (KeyCode::Enter, _) => {
                // Expand any collapsed paste markers back to full text,
                // then drain any pasted images (stripping their placeholders)
                // into pending_submit_images for repl.rs to pick up.
                self.editor.expand_pastes();
                self.pending_submit_images = self.drain_images();
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
                let (badge_text, _) = input_mode_badge(self.editor_input_mode(), &self.colors);
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
                let (badge_text, _) = input_mode_badge(self.editor_input_mode(), &self.colors);
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
            (KeyCode::Char('K'), _)
            | (KeyCode::Char('J'), _)
            | (KeyCode::PageUp, _)
            | (KeyCode::PageDown, _) => {
                self.handle_scroll_key(k.code, k.modifiers);
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
                self.overlays
                    .push(Box::new(super::command_palette::CommandPaletteState::new()));
            }

            // -- Toggle Plan Panel (Ctrl+T)
            // Match both the Kitty-protocol form (Char('t') + CONTROL) and
            // the legacy VT form (raw control character \x14).
            (KeyCode::Char('t'), KeyModifiers::CONTROL) | (KeyCode::Char('\x14'), _) => {
                if let Some(plan) = &mut self.active_plan {
                    plan.is_visible = !plan.is_visible;
                    let msg = if plan.is_visible {
                        "Plan panel shown"
                    } else {
                        "Plan panel hidden"
                    };
                    self.show_toast(msg, ToastLevel::Info);
                } else {
                    self.show_toast("No active plan", ToastLevel::Info);
                }
                self.draw_dirty = true;
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
                self.editor.handle_input(k, self.last_input_width);
                if let KeyCode::Char('@') = k.code
                    && !self.overlays.iter().any(|o| o.id() == "file_picker")
                {
                    let at_pos = self.editor.cursor_pos().saturating_sub(1);
                    self.overlays.push(Box::new(PickerState::new(
                        at_pos,
                        String::new(),
                        &self.file_ac,
                    )));
                }
            }
        }
        Ok(None)
    }

    /// Interpret a boxed `dyn Any` action produced by an overlay's
    /// [`OverlayComponent::take_result`].
    ///
    /// Returns `Ok(Some(Some(cmd)))` when the overlay wants to submit
    /// a REPL command, `Ok(None)` for side-effect-only actions.
    fn process_overlay_action(
        &mut self,
        action: Box<dyn std::any::Any>,
    ) -> Result<Option<Option<String>>> {
        // -- Command palette → submit a slash command
        let action = match action.downcast::<String>() {
            Ok(cmd) => return Ok(Some(Some(*cmd))),
            Err(a) => a,
        };

        // -- Theme picker actions (preview / submit / revert)
        let action = match action.downcast::<ThemePickerAction>() {
            Ok(tp_action) => {
                match *tp_action {
                    ThemePickerAction::Preview(colors) => {
                        self.apply_theme(colors);
                    }
                    ThemePickerAction::Submit(cmd) => {
                        return Ok(Some(Some(cmd)));
                    }
                    ThemePickerAction::Revert(colors) => {
                        self.apply_theme(colors);
                        self.show_toast("Theme reverted", ToastLevel::Info);
                    }
                }
                return Ok(None);
            }
            Err(a) => a,
        };

        // -- File picker actions
        let _action = match action.downcast::<FilePickerAction>() {
            Ok(fp_action) => {
                match *fp_action {
                    FilePickerAction::Select {
                        at_pos,
                        query_len,
                        selected,
                    } => {
                        self.editor.snapshot();
                        let query_end = at_pos + 1 + query_len;
                        let drain_end = query_end.min(self.editor.text().len());
                        let mut text = self.editor.text();
                        text.drain(at_pos..drain_end);
                        self.editor.set_text(text);
                        self.editor.insert_str_at(at_pos, &selected);
                        self.editor.set_cursor_pos(at_pos + selected.len());
                    }
                    FilePickerAction::BackspaceChar {
                        at_pos,
                        query_len_before,
                    } => {
                        let remove_at = at_pos + 1 + query_len_before - 1;
                        if remove_at < self.editor.text().len() {
                            self.editor.remove_char_at(remove_at);
                        }
                    }
                    FilePickerAction::DeleteAt { at_pos } => {
                        if at_pos < self.editor.text().len() {
                            self.editor.remove_char_at(at_pos);
                            self.editor.set_cursor_pos(at_pos);
                        }
                    }
                    FilePickerAction::InsertChar { position, ch } => {
                        self.editor.insert_char_at(position, ch);
                        self.editor.set_cursor_pos(position + ch.len_utf8());
                    }
                }
                return Ok(None);
            }
            Err(a) => a,
        };

        // Unknown action type — ignore.
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
        assert!(
            is_newline_shortcut(KeyModifiers::SHIFT),
            "Shift+Enter should be recognized as a newline shortcut"
        );
        assert!(
            is_newline_shortcut(KeyModifiers::ALT),
            "Alt+Enter should be recognized as a newline shortcut"
        );
        assert!(
            is_newline_shortcut(KeyModifiers::CONTROL),
            "Ctrl+Enter should be recognized as a newline shortcut"
        );
        assert!(
            is_newline_shortcut(KeyModifiers::SHIFT | KeyModifiers::CONTROL),
            "Ctrl+Shift+Enter should be recognized"
        );
        assert!(
            is_newline_shortcut(KeyModifiers::SHIFT | KeyModifiers::ALT),
            "Alt+Shift+Enter should be recognized"
        );
        assert!(
            !is_newline_shortcut(KeyModifiers::NONE),
            "Plain Enter should not be recognized as a newline shortcut"
        );
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
