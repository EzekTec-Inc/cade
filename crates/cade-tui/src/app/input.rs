//! User input loop — read_input and handle_key_input.

use crossterm::event::{
    self, Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers,
};

use crate::Result;

use super::{ServerBootStatus, ToastLevel, TuiApp};
use crate::autocomplete::AutocompleteProvider;

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
            // Redraw when dirty (draw flag, signal system, toast, or slots).
            if self.draw_dirty
                || self.signals.any_dirty()
                || self.toast.is_some()
                || self.slots.requires_tick()
            {
                self.draw()?;
            }

            // 50 ms poll: allows animation ticks without burning CPU.
            if !event::poll(std::time::Duration::from_millis(50))? {
                // Trigger redraw if any MCP server is loading (for spinner animation)
                // or if we are displaying the settled results before hiding the card.
                if let Some(ref progress) = self.mcp_boot_status {
                    let boot_map = progress.lock();
                    let mut show_card = false;
                    let mut all_done = true;
                    for status in boot_map.values() {
                        if matches!(status, ServerBootStatus::Loading) {
                            show_card = true;
                            all_done = false;
                        }
                    }
                    if all_done {
                        if self.mcp_all_settled_at.is_none() {
                            self.mcp_all_settled_at = Some(std::time::Instant::now());
                        }
                    } else {
                        self.mcp_all_settled_at = None;
                        self.mcp_closed = false; // Reset if loading starts again
                    }
                    if let Some(settled) = self.mcp_all_settled_at
                        && settled.elapsed() < std::time::Duration::from_secs(3)
                    {
                        show_card = true;
                    }
                    if show_card && !boot_map.is_empty() && !self.mcp_closed {
                        self.draw_dirty = true;
                    } else if self.mcp_all_settled_at.is_some() && !self.mcp_closed {
                        // The display window has expired! Mark as closed and trigger exactly one redraw to erase the card.
                        self.mcp_closed = true;
                        self.draw_dirty = true;
                    }
                }

                if let Some(ref ready) = self.startup_ready
                    && !self.mcp_processed
                    && ready.load(std::sync::atomic::Ordering::SeqCst)
                {
                    self.mcp_processed = true;
                    return Ok(Some("__MCP_READY__".to_string()));
                }

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
                        if !self.is_pasting {
                            self.draw()?;
                        }
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
                Event::Mouse(m)
                    if self.slots.handle_mouse(m) => {
                        self.draw()?;
                    }
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
        // Track key event velocity for simulated paste flood throttling (TUI-6)
        let now = std::time::Instant::now();
        let delta = now.duration_since(self.last_keypress);
        self.last_keypress = now;

        if delta.as_millis() < 3 {
            self.is_pasting = true;
        } else if delta.as_millis() >= 100 {
            self.is_pasting = false;
        }

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

        // -- UI extension slot focus and input routing (Phase 4)
        {
            use crate::slots::FocusRegion;
            if self.focused_region != FocusRegion::Input {
                if k.code == KeyCode::Esc {
                    self.focused_region = FocusRegion::Input;
                    use crate::slots::UiSlot;
                    for s in [UiSlot::Sidebar, UiSlot::Header, UiSlot::Footer] {
                        if let Some(w) = self.slots.get_mut(s) {
                            w.set_focused(false);
                        }
                    }
                    self.show_toast("Focus: Prompt input active", ToastLevel::Info);
                    self.draw_dirty = true;
                    let _ = self.draw();
                    return Ok(None);
                }

                if k.code == KeyCode::Char('f') && k.modifiers.contains(KeyModifiers::CONTROL) {
                    self.cycle_focus();
                    return Ok(None);
                }

                if let Some(slot) = self.focused_region.to_slot()
                    && let Some(widget) = self.slots.get_mut(slot)
                {
                    widget.handle_input(k);
                }

                // Consume all input when a slot is focused to protect prompt editor
                self.draw_dirty = true;
                let _ = self.draw();
                return Ok(None);
            }
        }

        // -- Lua global keybindings
        if let Some(lua) = &self.lua_engine {
            let mut key_str = String::new();
            if k.modifiers
                .contains(crossterm::event::KeyModifiers::CONTROL)
            {
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
                let has_queued_cmd = !lua
                    .command_queue
                    .lock()
                    .expect("LuaEngine command_queue")
                    .is_empty()
                    || !lua
                        .tool_queue
                        .lock()
                        .expect("LuaEngine tool_queue")
                        .is_empty();
                self.draw_dirty = true;
                let _ = self.draw();
                if has_queued_cmd {
                    return Ok(Some(Some(String::new())));
                }
                return Ok(None); // event consumed
            }
        }

        // Delegate scroll keys (Shift+K, Shift+J, PageUp, PageDown) to unified handler
        if self.handle_scroll_key(k.code, k.modifiers) {
            let _ = self.draw();
            return Ok(None);
        }

        match k.code {
            KeyCode::Char('f') if k.modifiers.contains(KeyModifiers::CONTROL) => {
                self.cycle_focus();
                return Ok(None);
            }
            KeyCode::Char('c') if k.modifiers.contains(KeyModifiers::CONTROL) => {
                return Ok(Some(None));
            }
            KeyCode::Char('d') if k.modifiers.contains(KeyModifiers::CONTROL) => {
                if self.editor.is_empty() {
                    return Ok(Some(None)); // Exit if prompt is empty
                } else {
                    self.editor.expand_pastes();
                    return Ok(Some(Some(self.editor.text()))); // Submit if non-empty
                }
            }

            KeyCode::Char('j') | KeyCode::Char('J')
                if k.modifiers.contains(KeyModifiers::SHIFT) =>
            {
                // "Follow" mode: reset scroll and jump to bottom.
                if self.scroll > 0 {
                    self.scroll = 0;
                    self.draw_dirty = true;
                    // Provide brief visual confirmation
                    self.show_toast("Jumped to bottom", ToastLevel::Info);
                }
                return Ok(None);
            }

            KeyCode::Char('y') if k.modifiers.contains(KeyModifiers::CONTROL) => {
                self.overlays
                    .push(Box::new(crate::app::copy_overlay::CopyOverlay::new(
                        &self.lines,
                    )));
                self.draw_dirty = true;
            }

            KeyCode::Char('v') | KeyCode::Char('V')
                if k.modifiers.contains(KeyModifiers::CONTROL)
                    || k.modifiers.contains(KeyModifiers::ALT) =>
            {
                if let Some((media_type, w, h, b64)) = crate::app::clipboard::read_clipboard_image() {
                    self.handle_image_paste(&media_type, b64, w, h);
                    self.show_toast("Pasted image from clipboard", ToastLevel::Success);
                } else if let Some(text) = crate::app::clipboard::read_clipboard_text() {
                    self.editor.handle_paste(&text);
                    self.draw_dirty = true;
                }
                return Ok(None);
            }

            KeyCode::Char('g') if k.modifiers.contains(KeyModifiers::CONTROL) => {
                self.toggle_last_collapsible_item();
            }

            KeyCode::Char('?')
                if k.modifiers.contains(KeyModifiers::CONTROL)
                    || (k.code == KeyCode::Char('?') && self.editor.is_empty()) =>
            {
                self.overlays
                    .push(Box::new(crate::app::help_overlay::HelpOverlay::new()));
                self.draw_dirty = true;
            }

            KeyCode::Char('p') if k.modifiers.contains(KeyModifiers::CONTROL) => {
                self.overlays.push(Box::new(
                    crate::app::command_palette::CommandPaletteState::new(),
                ));
                self.draw_dirty = true;
            }

            KeyCode::Char('o') if k.modifiers.contains(KeyModifiers::CONTROL) => {
                self.expand_all = !self.expand_all;
                self.draw_dirty = true;
            }

            KeyCode::Char('l') if k.modifiers.contains(KeyModifiers::CONTROL) => {
                self.lines.clear();
                self.content_version += 1;
                self.pending_submit_images.clear();
                self.pending_paste_images.clear();
                self.draw_dirty = true;
            }
            KeyCode::Char('t') if k.modifiers.contains(KeyModifiers::CONTROL) => {
                let msg = if let Some(plan) = &mut self.active_plan {
                    plan.is_visible = !plan.is_visible;
                    self.draw_dirty = true;
                    if plan.is_visible {
                        "Plan panel shown"
                    } else {
                        "Plan panel hidden"
                    }
                } else {
                    ""
                };
                if !msg.is_empty() {
                    self.show_toast(msg, ToastLevel::Info);
                }
            }

            KeyCode::Tab => {
                let input_text = self.editor.text();
                let cursor_pos = self.editor.cursor_pos();

                let word_start = input_text[..cursor_pos]
                    .rfind(|c: char| c.is_whitespace())
                    .map(|i| i + 1)
                    .unwrap_or(0);
                let partial = &input_text[word_start..cursor_pos];

                // Trigger Slash Command completion (Tab on '/')
                if partial.starts_with('/') {
                    let suggestions = self.slash_ac.completions(&input_text, cursor_pos);
                    if !suggestions.is_empty() {
                        self.overlays.push(Box::new(
                            crate::autocomplete::AutocompleteOverlay::new(
                                suggestions,
                                word_start,
                                cursor_pos,
                            ),
                        ));
                        self.draw_dirty = true;
                        return Ok(None);
                    }
                }

                // Trigger Tool/MCP completion (Tab on ':')
                if partial.starts_with(':') {
                    let suggestions = self.tool_ac.completions(&input_text, cursor_pos);
                    if !suggestions.is_empty() {
                        self.overlays.push(Box::new(
                            crate::autocomplete::AutocompleteOverlay::new(
                                suggestions,
                                word_start,
                                cursor_pos,
                            ),
                        ));
                        self.draw_dirty = true;
                        return Ok(None);
                    }
                }

                // Trigger Next Step completion (Tab on '?')
                if partial.starts_with('?') {
                    let suggestions = self.next_step_ac.completions(&input_text, cursor_pos);
                    if !suggestions.is_empty() {
                        self.overlays.push(Box::new(
                            crate::autocomplete::AutocompleteOverlay::new(
                                suggestions,
                                word_start,
                                cursor_pos,
                            ),
                        ));
                        self.draw_dirty = true;
                        return Ok(None);
                    }
                }

                // Trigger agent/model completion (Tab)
                if let Some((new_input, new_cursor)) =
                    self.agent_model_ac.complete_token(&input_text, cursor_pos)
                {
                    self.editor.set_text(new_input);
                    self.editor.set_cursor_pos(new_cursor);
                    self.draw_dirty = true;
                    return Ok(None);
                }

                // Trigger file path completion (Tab)
                if let Some((new_input, new_cursor)) =
                    self.file_ac.complete_path(&input_text, cursor_pos)
                {
                    self.editor.set_text(new_input);
                    self.editor.set_cursor_pos(new_cursor);
                    self.draw_dirty = true;
                    return Ok(None);
                }

                // Trigger history completion (Tab)
                if !input_text.trim().is_empty() {
                    let matches: Vec<String> = history
                        .iter()
                        .filter(|h| h.starts_with(&input_text) && *h != &input_text)
                        .cloned()
                        .collect();
                    if !matches.is_empty() {
                        let suggestion = crate::autocomplete::common_prefix(&matches);
                        let final_suggestion = if suggestion == input_text {
                            // If common prefix is already the input, just take the most recent full match to let them cycle
                            matches.last().unwrap().clone()
                        } else {
                            suggestion
                        };

                        self.editor.set_text(final_suggestion.clone());
                        self.editor.set_cursor_pos(final_suggestion.len());
                        self.draw_dirty = true;
                        return Ok(None);
                    }
                }
            }



            KeyCode::Up if !k.modifiers.contains(KeyModifiers::SHIFT) => {
                let text = self.editor.text();
                let pos = self.editor.cursor_pos();
                let is_first_line = !text[..pos].contains('\n');
                if is_first_line {
                    if !history.is_empty() {
                        let current_idx = hist_idx.unwrap_or(history.len());
                        if current_idx > 0 {
                            *hist_idx = Some(current_idx - 1);
                            let new_content = history[current_idx - 1].clone();
                            self.editor.set_text(new_content);
                            self.draw_dirty = true;
                        }
                    }
                    return Ok(None);
                } else {
                    let _action = self.editor.handle_input(k, self.term_width);
                    self.draw_dirty = true;
                    return Ok(None);
                }
            }
            KeyCode::Down if !k.modifiers.contains(KeyModifiers::SHIFT) => {
                let text = self.editor.text();
                let pos = self.editor.cursor_pos();
                let is_last_line = !text[pos..].contains('\n');
                if is_last_line {
                    if let Some(idx) = *hist_idx {
                        if idx + 1 < history.len() {
                            *hist_idx = Some(idx + 1);
                            let new_content = history[idx + 1].clone();
                            self.editor.set_text(new_content);
                            self.draw_dirty = true;
                        } else {
                            *hist_idx = None;
                            self.editor.clear();
                            self.draw_dirty = true;
                        }
                    }
                    return Ok(None);
                } else {
                    let _action = self.editor.handle_input(k, self.term_width);
                    self.draw_dirty = true;
                    return Ok(None);
                }
            }
            KeyCode::BackTab => {
                return Ok(Some(Some("__BACKTAB__".to_string())));
            }
            KeyCode::Enter if is_newline_shortcut(k.modifiers) => {
                self.editor.insert_newline();
                self.draw_dirty = true;
            }
            _ => {
                use crate::editor_component::EditorAction;
                let action = self.editor.handle_input(k, self.term_width);
                match action {
                    EditorAction::Consumed => {
                        self.draw_dirty = true;

                        if let Some(ac) = self
                            .overlays
                            .last_mut()
                            .and_then(|o| o.as_any_mut())
                            .and_then(|a| {
                                a.downcast_mut::<crate::autocomplete::AutocompleteOverlay>()
                            })
                            && !self.is_pasting
                        {
                            ac.update_suggestions(
                                &self.editor.text(),
                                self.editor.cursor_pos(),
                                &self.slash_ac,
                                &self.tool_ac,
                                &self.next_step_ac,
                            );
                            if ac.suggestions.is_empty() {
                                ac.dismissed = true;
                            }
                        }

                        if !self.is_pasting {
                            if let KeyCode::Char('/') = k.code {
                                let input_text = self.editor.text();
                                let cursor_pos = self.editor.cursor_pos();
                                let suggestions = self.slash_ac.completions(&input_text, cursor_pos);
                                if !suggestions.is_empty() {
                                    self.overlays.push(Box::new(
                                        crate::autocomplete::AutocompleteOverlay::new(
                                            suggestions,
                                            cursor_pos.saturating_sub(1),
                                            cursor_pos,
                                        ),
                                    ));
                                }
                            }
                            if let KeyCode::Char('@') = k.code {
                                let cursor_pos = self.editor.cursor_pos();
                                let at_pos = cursor_pos.saturating_sub(1);
                                self.overlays.push(Box::new(crate::app::PickerState::new(
                                    at_pos,
                                    String::new(),
                                    &self.file_ac,
                                )));
                            }
                        }
                        return Ok(None);
                    }
                    EditorAction::Submit(text) => {
                        if !text.trim().is_empty() {
                            self.dispatch(crate::app::reducer::TuiAction::SendMessage(text.clone()));
                            self.editor.clear();
                            return Ok(Some(Some(text)));
                        } else {
                            self.editor.clear();
                            self.draw_dirty = true;
                            return Ok(None);
                        }
                    }
                    EditorAction::Cancel => {
                        self.editor.clear();
                        self.draw_dirty = true;
                        return Ok(None);
                    }
                    EditorAction::Unhandled(_) => {}
                }
            }
        }

        Ok(None)
    }

    /// Helper to process overlay actions in the input loop.
    ///
    /// Drains any `Option<Box<dyn Any>>` action returned by an overlay and
    /// applies it to the app state.
    fn process_overlay_action(
        &mut self,
        action: Box<dyn std::any::Any>,
    ) -> Result<Option<Option<String>>> {
        let action = match action.downcast::<crate::autocomplete::AutocompleteAction>() {
            Ok(ac_action) => {
                let input = self.editor.text();
                let before = &input[..ac_action.word_start];
                let after = &input[ac_action.cursor_pos..];

                let mut completed = ac_action.text;
                if !completed.ends_with(' ') {
                    completed.push(' ');
                }

                let new_input = format!("{}{}{}", before, completed, after);
                let new_cursor = ac_action.word_start + completed.len();
                self.editor.set_text(new_input);
                self.editor.set_cursor_pos(new_cursor);
                self.draw_dirty = true;
                return Ok(None);
            }
            Err(action) => action,
        };

        let action = match action.downcast::<crate::app::copy_overlay::CopyAction>() {
            Ok(copy_action) => {
                let text = copy_action.0;
                self.dispatch(crate::app::reducer::TuiAction::CopyBlock(text));
                return Ok(None);
            }
            Err(action) => action,
        };

        let action = match action.downcast::<String>() {
            Ok(string_val) => {
                let s = *string_val;
                if s.starts_with('/') {
                    return Ok(Some(Some(s)));
                } else {
                    self.editor.handle_paste(&s);
                    self.draw_dirty = true;
                }
                return Ok(None);
            }
            Err(action) => action,
        };

        let action = match action.downcast::<crate::app::ThemePickerAction>() {
            Ok(tp_action) => {
                match *tp_action {
                    crate::app::ThemePickerAction::Preview(colors) => {
                        self.apply_theme(colors);
                    }
                    crate::app::ThemePickerAction::Submit(cmd) => {
                        return Ok(Some(Some(cmd)));
                    }
                    crate::app::ThemePickerAction::Revert(colors) => {
                        self.apply_theme(colors);
                        self.show_toast("Theme picker cancelled", crate::app::ToastLevel::Info);
                    }
                }
                return Ok(None);
            }
            Err(action) => action,
        };

        let _action = match action.downcast::<crate::app::FilePickerAction>() {
            Ok(fp_action) => {
                match *fp_action {
                    crate::app::FilePickerAction::Select {
                        at_pos,
                        query_len,
                        selected,
                    } => {
                        let mut completed = selected;
                        if !completed.ends_with(' ') {
                            completed.push(' ');
                        }
                        for _ in 0..(1 + query_len) {
                            self.editor.remove_char_at(at_pos);
                        }
                        self.editor.insert_str_at(at_pos, &completed);
                    }
                    crate::app::FilePickerAction::BackspaceChar {
                        at_pos,
                        query_len_before,
                    } => {
                        self.editor.remove_char_at(at_pos + query_len_before);
                    }
                    crate::app::FilePickerAction::DeleteAt { at_pos } => {
                        self.editor.remove_char_at(at_pos);
                    }
                    crate::app::FilePickerAction::InsertChar { position, ch } => {
                        self.editor.insert_char_at(position, ch);
                    }
                }
                self.draw_dirty = true;
                return Ok(None);
            }
            Err(action) => action,
        };

        Ok(None)
    }

    /// Cycle keyboard focus between the main prompt input and active UI slots (Sidebar, Header, Footer).
    pub fn cycle_focus(&mut self) {
        use crate::slots::{FocusRegion, UiSlot};

        let current = self.focused_region;
        let mut next = FocusRegion::Input;

        // Collect list of focusable occupied slots in preferred order
        let mut occupied_slots = Vec::new();
        if self.slots.is_occupied(UiSlot::Sidebar) {
            occupied_slots.push(FocusRegion::Sidebar);
        }
        if self.slots.is_occupied(UiSlot::Header) {
            occupied_slots.push(FocusRegion::Header);
        }
        if self.slots.is_occupied(UiSlot::Footer) {
            occupied_slots.push(FocusRegion::Footer);
        }

        if !occupied_slots.is_empty() {
            match current {
                FocusRegion::Input => {
                    next = occupied_slots[0];
                }
                FocusRegion::Sidebar => {
                    if let Some(pos) = occupied_slots
                        .iter()
                        .position(|&r| r == FocusRegion::Sidebar)
                    {
                        if pos + 1 < occupied_slots.len() {
                            next = occupied_slots[pos + 1];
                        } else {
                            next = FocusRegion::Input;
                        }
                    } else {
                        next = FocusRegion::Input;
                    }
                }
                FocusRegion::Header => {
                    if let Some(pos) = occupied_slots
                        .iter()
                        .position(|&r| r == FocusRegion::Header)
                    {
                        if pos + 1 < occupied_slots.len() {
                            next = occupied_slots[pos + 1];
                        } else {
                            next = FocusRegion::Input;
                        }
                    } else {
                        next = FocusRegion::Input;
                    }
                }
                FocusRegion::Footer => {
                    next = FocusRegion::Input;
                }
            }
        }

        // Inform slots of the change
        for slot in [UiSlot::Sidebar, UiSlot::Header, UiSlot::Footer] {
            if let Some(widget) = self.slots.get_mut(slot) {
                widget.set_focused(next == FocusRegion::from_slot(slot));
            }
        }

        self.focused_region = next;

        // Show a nice toast message
        let toast_msg = match next {
            FocusRegion::Input => "Focus: Prompt input active".to_string(),
            FocusRegion::Sidebar => "Focus: Sidebar active".to_string(),
            FocusRegion::Header => "Focus: Header active".to_string(),
            FocusRegion::Footer => "Focus: Footer active".to_string(),
        };
        self.show_toast(&toast_msg, ToastLevel::Info);
        self.draw_dirty = true;
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
