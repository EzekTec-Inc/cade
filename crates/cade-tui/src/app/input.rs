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
                        if !self.slots.handle_mouse(m) {
                            self.handle_scroll_mouse(m);
                            self.draw()?;
                        }
                    }
                    _ => {
                        if self.slots.handle_mouse(m) {
                            self.draw()?;
                        }
                    }
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
            if !key_str.is_empty() {
                if let Some(mut req) = lua.handle_global_keymap(&key_str) {
                    // LuaAction enum check removed, checking if unhandled directly
                        // event consumed with no action
                        return Ok(None);
                    }
                    if self.handle_lua_request(&mut req)? {
                        return Ok(None); // event consumed
                    }
                }
            }
        }

        match k.code {
            KeyCode::Char('c') if k.modifiers.contains(KeyModifiers::CONTROL) => {
                return Ok(Some(None));
            }
            KeyCode::Char('d') if k.modifiers.contains(KeyModifiers::CONTROL) => {
                if self.editor.is_empty() {
                    return Ok(Some(None)); // Exit if prompt is empty
                } else {
                    return Ok(Some(Some(self.editor.text()))); // Submit if non-empty
                }
            }

            KeyCode::Char('j') | KeyCode::Char('J') if k.modifiers.contains(KeyModifiers::SHIFT) => {
                // "Follow" mode: reset scroll and jump to bottom.
                if self.scroll > 0 {
                    self.scroll = 0;
                    self.draw_dirty = true;
                    // Provide brief visual confirmation
                    self.show_toast("Jumped to bottom", ToastLevel::Info);
                }
                return Ok(None);
            }

            KeyCode::Char('p') if k.modifiers.contains(KeyModifiers::CONTROL) => {
                self.overlays.push(Box::new(
                    crate::app::command_palette::CommandPaletteState::new(),
                ));
                self.draw_dirty = true;
            }

            KeyCode::Char('l') if k.modifiers.contains(KeyModifiers::CONTROL) => {
                self.lines.clear();
                self.clear_images();
                self.draw_dirty = true;
            }
            KeyCode::Char('t') if k.modifiers.contains(KeyModifiers::CONTROL) => {
                if let Some(plan) = &mut self.active_plan {
                    plan.is_visible = !plan.is_visible;
                    self.draw_dirty = true;
                    self.show_toast(
                        if plan.is_visible {
                            "Plan panel shown"
                        } else {
                            "Plan panel hidden"
                        },
                        ToastLevel::Info,
                    );
                }
            }

            // PageUp / PageDown
            KeyCode::PageUp => {
                let viewport = self.viewport_height.saturating_sub(6); // Approx prompt height
                self.scroll_page_up(viewport);
                self.draw_dirty = true;
            }
            KeyCode::PageDown => {
                let viewport = self.viewport_height.saturating_sub(6);
                self.scroll_page_down(viewport);
                self.draw_dirty = true;
            }

            KeyCode::Up if !k.modifiers.contains(KeyModifiers::SHIFT) => {
                if self.editor.is_first_line() {
                    // Navigate history
                    if !history.is_empty() {
                        let current_idx = hist_idx.unwrap_or(history.len());
                        if current_idx > 0 {
                            *hist_idx = Some(current_idx - 1);
                            let new_content = history[current_idx - 1].clone();
                            self.editor.set_text(&new_content);
                            self.draw_dirty = true;
                        }
                    }
                } else {
                    self.editor.move_cursor_up();
                    self.draw_dirty = true;
                }
            }
            KeyCode::Down if !k.modifiers.contains(KeyModifiers::SHIFT) => {
                if self.editor.is_last_line() {
                    // Navigate history
                    if let Some(idx) = *hist_idx {
                        if idx + 1 < history.len() {
                            *hist_idx = Some(idx + 1);
                            let new_content = history[idx + 1].clone();
                            self.editor.set_text(&new_content);
                            self.draw_dirty = true;
                        } else {
                            *hist_idx = None;
                            self.editor.clear();
                            self.draw_dirty = true;
                        }
                    }
                } else {
                    self.editor.move_cursor_down();
                    self.draw_dirty = true;
                }
            }

            KeyCode::Left => {
                self.editor.move_cursor_left();
                self.draw_dirty = true;
            }
            KeyCode::Right => {
                self.editor.move_cursor_right();
                self.draw_dirty = true;
            }
            KeyCode::Backspace => {
                if self.editor.delete_backward() {
                    self.draw_dirty = true;
                }
            }
            KeyCode::Delete => {
                if self.editor.delete_forward() {
                    self.draw_dirty = true;
                }
            }
            KeyCode::Home => {
                self.editor.move_to_start_of_line();
                self.draw_dirty = true;
            }
            KeyCode::End => {
                self.editor.move_to_end_of_line();
                self.draw_dirty = true;
            }

            KeyCode::Enter => {
                if is_newline_shortcut(k) {
                    self.editor.insert_char('\n');
                    self.draw_dirty = true;
                } else if !self.editor.is_empty() {
                    let text = self.editor.text();
                    if !text.trim().is_empty() {
                        self.editor.clear();
                        self.draw_dirty = true;
                        return Ok(Some(Some(text)));
                    } else {
                        // Just empty space
                        self.editor.clear();
                        self.draw_dirty = true;
                    }
                }
            }

            KeyCode::Char(c) => {
                if !k.modifiers.contains(KeyModifiers::CONTROL)
                    && !k.modifiers.contains(KeyModifiers::ALT)
                    // If Shift is pressed with an alphabetic char, insert it; otherwise ignore Shift modifier
                    && (!k.modifiers.contains(KeyModifiers::SHIFT) || c.is_alphabetic() || c.is_ascii_punctuation())
                {
                    self.editor.insert_char(c);
                    self.draw_dirty = true;
                }
            }

            KeyCode::Tab => {
                self.editor.insert_str("    ");
                self.draw_dirty = true;
            }
            KeyCode::BackTab => {
                // If BackTab isn't consumed by autocomplete, return mode toggle sentinel
                return Ok(Some(Some("__BACKTAB__".to_string())));
            }

            _ => {}
        }

        Ok(None)
    }

    /// Helper to process overlay actions in the input loop.
    ///
    /// Drains any `Option<Box<dyn Any>>` action returned by an overlay and
    /// applies it to the app state.
    fn process_overlay_action(&mut self, action: Box<dyn std::any::Any>) -> Result<Option<Option<String>>> {
        if let Ok(string_val) = action.downcast::<String>() {
            let s = *string_val;
            if s.starts_with('/') {
                // Return as immediate command submission
                return Ok(Some(Some(s)));
            } else {
                // Otherwise append to current prompt
                self.editor.handle_paste(&s);
                self.draw_dirty = true;
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
