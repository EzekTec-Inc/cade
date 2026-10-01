//! User input loop — read_input and handle_key_input.

use crossterm::event::{self, Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers};

use crate::Result;

use super::{ServerBootStatus, ToastLevel, TuiApp};
use crate::autocomplete::AutocompleteProvider;

/// Host work wakes the REPL without impersonating a submitted prompt.
pub enum InputOutcome {
    Submitted(String),
    Exit,
    WorkReady,
}

pub(crate) struct OverlayDispatch {
    pub owned: bool,
    pub dirty: bool,
    pub action: Option<Box<dyn std::any::Any>>,
}

/// Production modal-stack routing shared by idle keys/paste/pointer input and
/// active-turn dispatch. Keeping the stack transition independent of terminal
/// IO allows behavior tests to use the actual editor and ratatui TestBackend.
pub(crate) fn dispatch_overlay_stack(
    overlays: &mut Vec<Box<dyn crate::overlay_component::OverlayComponent>>,
    event: &Event,
) -> OverlayDispatch {
    use crate::overlay_component::OverlayInputResult;
    let count = overlays.len();
    overlays.retain(|overlay| !overlay.is_dismissed());
    let dirty = overlays.len() != count;
    let Some(overlay) = overlays.last_mut() else {
        return OverlayDispatch {
            owned: false,
            dirty,
            action: None,
        };
    };
    let result = overlay.handle_event(event);
    if result == OverlayInputResult::NotHandled {
        return OverlayDispatch {
            owned: matches!(event, Event::Key(_) | Event::Mouse(_) | Event::Paste(_)),
            dirty,
            action: None,
        };
    }
    let action = overlay.take_result();
    let action = if result == OverlayInputResult::Dismiss {
        overlays
            .pop()
            .and_then(|mut overlay| action.or_else(|| overlay.take_result()))
    } else {
        action
    };
    OverlayDispatch {
        owned: true,
        dirty: true,
        action,
    }
}

impl TuiApp {
    // -- Input loop

    /// Returns `true` if any UI animation (spinner, progress bar, toast) is actively running.
    pub fn is_animating(&self) -> bool {
        // 1. Thinking / reasoning spinner active
        if self.thinking.is_some() {
            return true;
        }

        // 2. Active toast displayed
        if self.toast.is_some() {
            return true;
        }

        // 3. UI slot requires active tick execution (animations or interactive controls)
        if self.slots.requires_tick() {
            return true;
        }

        // 4. MCP servers starting up (animate loading card)
        if let Some(ref progress) = self.mcp_boot_status {
            let boot_map = progress.lock();
            let mut show_card = false;
            for status in boot_map.values() {
                if matches!(status, ServerBootStatus::Loading) {
                    show_card = true;
                    break;
                }
            }
            if show_card && !boot_map.is_empty() && !self.mcp_closed {
                return true;
            }
            if let Some(settled) = self.mcp_all_settled_at
                && settled.elapsed() < std::time::Duration::from_secs(3)
                && !self.mcp_closed
            {
                return true;
            }
        }

        false
    }

    /// Synchronous terminal-owner adapter. The REPL uses its asynchronous
    /// reader so the app lock is never retained while awaiting input/work.
    pub fn read_input(
        &mut self,
        history: &mut [String],
        hist_idx: &mut Option<usize>,
        tools_ready: impl Fn() -> bool,
    ) -> Result<InputOutcome> {
        self.draw()?;
        loop {
            if let Some(outcome) = self.tick_idle(true) {
                return Ok(outcome);
            }
            if self.has_lua_host_work(tools_ready()) {
                return Ok(InputOutcome::WorkReady);
            }
            if self.draw_dirty || self.signals.any_dirty() || self.is_animating() {
                self.draw()?;
            }
            let poll_timeout = if self.is_animating() || self.lua_engine.is_some() {
                std::time::Duration::from_millis(50)
            } else {
                std::time::Duration::from_millis(2000)
            };
            if event::poll(poll_timeout)?
                && let Some(outcome) = self.handle_idle_event(event::read()?, history, hist_idx)?
            {
                return Ok(outcome);
            }
        }
    }

    pub fn handle_bracketed_paste_text(&mut self, text: &str) {
        if self
            .dispatch_overlay_event(&Event::Paste(text.to_owned()))
            .map(|r| r.0)
            .unwrap_or(true)
        {
            return;
        }
        // Bracketed paste: the terminal wrapped the pasted content in
        // paste-start / paste-end markers so crossterm delivers it as one string.
        // Drag-onto-terminal often appears as a file URI/path; load image files
        // as attachments and normalize non-image file paths for @mentions.
        let trimmed = text.trim();
        if self.try_paste_image_file_path(trimmed) {
            return;
        }

        if let Some(normalized_path) = self.try_normalize_pasted_file_path(trimmed) {
            self.editor.handle_paste(&normalized_path);
        } else {
            self.editor.handle_paste(text);
        }
        self.last_status = None;
        self.draw_dirty = true;
    }

    /// The asynchronous REPL reader calls this with the app lock held only for
    /// dispatch, then releases it before waiting for the next terminal event.
    pub fn handle_idle_event(
        &mut self,
        event: Event,
        history: &mut [String],
        hist_idx: &mut Option<usize>,
    ) -> Result<Option<InputOutcome>> {
        match event {
            Event::Key(key) if key.kind == KeyEventKind::Press => {
                let was_empty = self.editor.is_empty();
                if let Some(result) = self.handle_key_input(key, history, hist_idx)? {
                    return Ok(Some(match result {
                        Some(text) => InputOutcome::Submitted(text),
                        None => InputOutcome::Exit,
                    }));
                }
                if was_empty && !self.editor.is_empty() {
                    self.last_status = None;
                }
                self.draw_dirty = true;
            }
            Event::Paste(text) => self.handle_bracketed_paste_text(&text),
            Event::Mouse(mouse) => {
                self.handle_message_area_mouse_event(mouse)?;
            }
            Event::Resize(_, _) => self.handle_resize()?,
            Event::FocusGained => self.has_focus = true,
            Event::FocusLost => self.has_focus = false,
            _ => {}
        }
        Ok(None)
    }

    pub fn tick_idle(&mut self, allow_host_actions: bool) -> Option<InputOutcome> {
        self.pump_lua_ui_events();
        if allow_host_actions && self.has_lua_host_work(false) {
            return Some(InputOutcome::WorkReady);
        }
        if allow_host_actions
            && let Some(ready) = &self.startup_ready
            && !self.mcp_processed
            && ready.load(std::sync::atomic::Ordering::SeqCst)
        {
            self.mcp_processed = true;
            return Some(InputOutcome::Submitted("__MCP_READY__".into()));
        }
        if let Some(progress) = &self.mcp_boot_status {
            let map = progress.lock();
            let loading = map
                .values()
                .any(|status| matches!(status, ServerBootStatus::Loading));
            if loading {
                self.mcp_all_settled_at = None;
                self.mcp_closed = false;
            } else if self.mcp_all_settled_at.is_none() {
                self.mcp_all_settled_at = Some(std::time::Instant::now());
            }
            let visible = loading
                || self
                    .mcp_all_settled_at
                    .is_some_and(|at| at.elapsed() < std::time::Duration::from_secs(3));
            if !visible && !self.mcp_closed {
                self.mcp_closed = true;
                self.draw_dirty = true;
            }
            if visible && !map.is_empty() && !self.mcp_closed {
                self.draw_dirty = true;
            }
        }
        if let Some(getter) = &self.bg_pending_count {
            let pending = getter();
            let mut toast = self.toast.take();
            if super::tick_bg_pending_toast(pending, &mut self.bg_last_announced, &mut toast) {
                self.draw_dirty = true;
            }
            self.toast = toast;
            self.prune_completed_subagents();
        }
        None
    }

    pub fn paste_from_clipboard(&mut self) -> bool {
        // 1. Try OS clipboard text first
        if let Some(text) = crate::app::clipboard::read_clipboard_text()
            && !text.is_empty()
        {
            self.handle_bracketed_paste_text(&text);
            return true;
        }

        // 2. Try CADE internal clipboard buffer (from viewport selection / copy)
        if let Some(ref text) = self.retained_selected_text
            && !text.is_empty()
        {
            let t = text.clone();
            self.handle_bracketed_paste_text(&t);
            return true;
        }

        // 3. Try OS clipboard image
        if let Some((media_type, w, h, b64)) = crate::app::clipboard::read_clipboard_image() {
            self.handle_image_paste(&media_type, b64, w, h);
            self.show_toast("Pasted image from clipboard", ToastLevel::Success);
            self.draw_dirty = true;
            return true;
        }

        false
    }

    pub fn handle_message_area_mouse_event(
        &mut self,
        m: crossterm::event::MouseEvent,
    ) -> Result<bool> {
        if self.dispatch_overlay_event(&Event::Mouse(m))?.0 {
            self.draw()?;
            return Ok(true);
        }
        if let Some(slot) = self.slots.route_mouse(m) {
            if matches!(m.kind, crossterm::event::MouseEventKind::Down(_)) {
                self.focused_region = crate::slots::FocusRegion::from_slot(slot);
                for region in [
                    crate::slots::UiSlot::Sidebar,
                    crate::slots::UiSlot::Header,
                    crate::slots::UiSlot::Footer,
                ] {
                    if let Some(widget) = self.slots.get_mut(region) {
                        widget.set_focused(region == slot);
                    }
                }
            }
            self.draw()?;
            return Ok(true);
        }

        if self.subagent_tray.is_visible {
            let trackers = self.subagent_trackers.clone();
            if self.subagent_tray.handle_mouse(m, &trackers) {
                self.draw_dirty = true;
                self.draw()?;
                return Ok(true);
            }
        }

        let is_inside_messages = m.column >= self.messages_area.x
            && m.column < self.messages_area.x + self.messages_area.width
            && m.row >= self.messages_area.y
            && m.row < self.messages_area.y + self.messages_area.height;

        match m.kind {
            crossterm::event::MouseEventKind::Down(crossterm::event::MouseButton::Left) => {
                if is_inside_messages {
                    self.selection_active = true;
                    self.selection_start = Some((m.column, m.row));
                    self.selection_current = Some((m.column, m.row));
                    self.selection_retained = false;
                    self.retained_selected_text = None;
                    self.draw()?;
                    return Ok(true);
                } else if self.selection_retained || self.selection_active {
                    self.clear_selection();
                    self.draw()?;
                }
            }
            crossterm::event::MouseEventKind::Drag(crossterm::event::MouseButton::Left) => {
                if self.selection_active {
                    let new_pos = Some((m.column, m.row));
                    if self.selection_current != new_pos {
                        self.selection_current = new_pos;
                        self.draw_throttled()?;
                    }
                    return Ok(true);
                }
            }
            crossterm::event::MouseEventKind::Up(crossterm::event::MouseButton::Left) => {
                if self.selection_active {
                    let is_single_click = self.selection_start == Some((m.column, m.row));
                    self.selection_current = Some((m.column, m.row));
                    if is_single_click {
                        self.toggle_last_collapsible_item();
                        self.clear_selection();
                    } else {
                        self.copy_selected_text();
                    }
                    self.draw()?;
                    return Ok(true);
                }
            }
            _ => {
                if is_inside_messages && self.handle_scroll_mouse(m.kind) {
                    self.draw()?;
                    return Ok(true);
                }
            }
        }

        Ok(false)
    }

    pub(crate) fn handle_key_input(
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
        let (owned, action) = self.dispatch_overlay_event(&Event::Key(k))?;
        if owned {
            return Ok(action);
        }

        // -- Subagent Control Tray hotkeys & input routing
        if matches!(k.code, KeyCode::F(5)) {
            self.toggle_subagent_tray();
            let _ = self.draw();
            return Ok(None);
        }

        if matches!(k.code, KeyCode::Char('w') | KeyCode::Char('W'))
            && k.modifiers.contains(KeyModifiers::CONTROL)
            && self.subagent_tray.is_visible
        {
            self.toggle_subagent_tray_focus();
            let _ = self.draw();
            return Ok(None);
        }

        if self.subagent_tray.is_visible && self.subagent_tray.is_focused {
            let trackers = self.subagent_trackers.clone();
            if self.subagent_tray.handle_key(k, &trackers) {
                let action = self.subagent_tray.take_pending_action();
                self.draw_dirty = true;
                let _ = self.draw();
                if action != crate::app::subagent_tray::SubagentTrayAction::None
                    && let Ok(json) = serde_json::to_string(&action)
                {
                    return Ok(Some(Some(format!("__SUBAGENT_TRAY_ACTION__{json}"))));
                }
                return Ok(None);
            }
            return Ok(None); // The focused tray owns unrecognized input too.
        }

        // Legacy overlay dispatch blocks removed — all four overlays
        // (summary, command palette, theme picker, file picker) are now
        // handled by the dynamic overlay stack above (Phase 3).

        // -- UI extension slot focus and input routing (Phase 4)
        if self.handle_focused_slot_key(k) {
            return Ok(None);
        }

        // -- Lua global keybindings
        if self.handle_lua_key(k) {
            return Ok(None);
        }

        // Delegate scroll keys (Alt+K/J, PageUp, PageDown, Ctrl+End) to unified handler
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
            KeyCode::Char('l') | KeyCode::Char('L')
                if k.modifiers.contains(KeyModifiers::CONTROL) =>
            {
                let _ = self.terminal.clear();
                self.draw_dirty = true;
                self.draw()?;
                return Ok(None);
            }
            KeyCode::Char('d') if k.modifiers.contains(KeyModifiers::CONTROL) => {
                if self.editor.is_empty() {
                    return Ok(Some(None)); // Exit if prompt is empty
                } else {
                    self.editor.expand_pastes();
                    return Ok(Some(Some(self.editor.text()))); // Submit if non-empty
                }
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
                self.paste_from_clipboard();
                return Ok(None);
            }

            KeyCode::Insert if k.modifiers.contains(KeyModifiers::SHIFT) => {
                self.paste_from_clipboard();
                return Ok(None);
            }

            KeyCode::Char('g') if k.modifiers.contains(KeyModifiers::CONTROL) => {
                self.toggle_last_collapsible_item();
            }

            // Ctrl+Shift+C: Quote active/retained selection to prompt (B.3)
            KeyCode::Char('C') | KeyCode::Char('c')
                if k.modifiers
                    .contains(KeyModifiers::CONTROL | KeyModifiers::SHIFT) =>
            {
                self.quote_selection_to_prompt();
                return Ok(None);
            }

            // Esc without overlays: Drop retained selection (B.1)
            KeyCode::Esc if self.selection_retained || self.selection_active => {
                self.clear_selection();
                return Ok(None);
            }

            // Esc: clear tool card selection if active (Phase 9)
            KeyCode::Esc if self.selected_tool_card_index.is_some() => {
                self.selected_tool_card_index = None;
                self.copy_highlight = None;
                self.draw_dirty = true;
                return Ok(None);
            }

            // Alt+Up / Alt+k: Select previous tool card (Phase 9)
            KeyCode::Up | KeyCode::Char('k') if k.modifiers.contains(KeyModifiers::ALT) => {
                self.select_prev_tool_card();
                return Ok(None);
            }

            // Alt+Down / Alt+j: Select next tool card (Phase 9)
            KeyCode::Down | KeyCode::Char('j') if k.modifiers.contains(KeyModifiers::ALT) => {
                self.select_next_tool_card();
                return Ok(None);
            }

            // Alt+Enter or Enter with empty prompt and card selected: Open tool pager (Phase 9)
            KeyCode::Enter
                if k.modifiers.contains(KeyModifiers::ALT)
                    || (self.selected_tool_card_index.is_some() && self.editor.is_empty()) =>
            {
                self.open_selected_or_latest_tool_pager();
                return Ok(None);
            }

            _ if self.leader_engine.is_active
                || (k.modifiers.contains(KeyModifiers::CONTROL)
                    && k.code == KeyCode::Char('x')) =>
            {
                let outcome = self.leader_engine.handle_key(k);
                self.draw_dirty = true;
                match outcome {
                    crate::app::leader::LeaderOutcome::Pending => return Ok(None),
                    crate::app::leader::LeaderOutcome::Dismissed => return Ok(None),
                    crate::app::leader::LeaderOutcome::Action(action) => match action {
                        crate::app::leader::LeaderAction::QuoteSelection => {
                            self.quote_selection_to_prompt();
                            return Ok(None);
                        }
                        crate::app::leader::LeaderAction::CopyMessage => {
                            if !self.copy_selected_text() {
                                self.copy_last_message();
                            }
                            return Ok(None);
                        }
                        crate::app::leader::LeaderAction::SidebarToggle => {
                            self.toggle_sidebar();
                            return Ok(None);
                        }
                        crate::app::leader::LeaderAction::NewSession => {
                            return Ok(Some(Some("/session new".to_string())));
                        }
                        crate::app::leader::LeaderAction::ListSessions => {
                            return Ok(Some(Some("/session".to_string())));
                        }
                        crate::app::leader::LeaderAction::CompactSession => {
                            return Ok(Some(Some("/compact".to_string())));
                        }
                        crate::app::leader::LeaderAction::SessionTimeline => {
                            return Ok(Some(Some("/timeline".to_string())));
                        }
                        crate::app::leader::LeaderAction::ModelPicker => {
                            return Ok(Some(Some("/model".to_string())));
                        }
                        crate::app::leader::LeaderAction::SessionPicker => {
                            return Ok(Some(Some("/session".to_string())));
                        }
                        crate::app::leader::LeaderAction::ThemePicker => {
                            return Ok(Some(Some("/theme".to_string())));
                        }
                        crate::app::leader::LeaderAction::UndoCheckpoint => {
                            return Ok(Some(Some("/undo".to_string())));
                        }
                        crate::app::leader::LeaderAction::RedoCheckpoint => {
                            return Ok(Some(Some("/redo".to_string())));
                        }
                        crate::app::leader::LeaderAction::TogglePermissions => {
                            return Ok(Some(Some("/permissions".to_string())));
                        }
                        crate::app::leader::LeaderAction::HelpOverlay => {
                            return Ok(Some(Some("/help".to_string())));
                        }
                        crate::app::leader::LeaderAction::StashPrompt => {
                            self.stash_prompt();
                            return Ok(None);
                        }
                        crate::app::leader::LeaderAction::ToggleConceal => {
                            self.toggle_conceal();
                            return Ok(None);
                        }
                        crate::app::leader::LeaderAction::ToolPager => {
                            self.open_tool_pager_overlay();
                            return Ok(None);
                        }
                    },
                }
            }

            // Ctrl+Alt+V: Paste as plain-text (Section J: skips collapse/attachments)
            KeyCode::Char('v') | KeyCode::Char('V')
                if k.modifiers
                    .contains(KeyModifiers::CONTROL | KeyModifiers::ALT) =>
            {
                let text = crate::app::clipboard::read_clipboard_text_with_mode(
                    self.tui_settings.linux_clipboard_selection,
                );
                if let Some(text) = text {
                    self.editor.insert_str(&text);
                    self.draw_dirty = true;
                }
                return Ok(None);
            }

            // Ctrl+E: Open prompt in external editor ($VISUAL / $EDITOR) (Section J)
            KeyCode::Char('e') | KeyCode::Char('E')
                if k.modifiers.contains(KeyModifiers::CONTROL)
                    && !k.modifiers.contains(KeyModifiers::ALT) =>
            {
                let _ = self.edit_in_external_editor();
                return Ok(None);
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

            KeyCode::F(5) => {
                self.toggle_subagent_tray();
                let _ = self.draw();
                return Ok(None);
            }

            KeyCode::Char('w') if k.modifiers.contains(KeyModifiers::CONTROL) => {
                if self.subagent_tray.is_visible {
                    self.toggle_subagent_tray_focus();
                    let _ = self.draw();
                    return Ok(None);
                }
            }

            KeyCode::Char('o' | 'O') if k.modifiers.contains(KeyModifiers::CONTROL) => {
                self.expand_all = !self.expand_all;
                self.content_version += 1;
                self.draw_dirty = true;
                let msg = if self.expand_all {
                    "All blocks expanded"
                } else {
                    "All blocks collapsed"
                };
                self.show_toast(msg, ToastLevel::Info);
                let _ = self.draw();
                return Ok(None);
            }
            KeyCode::Char('\x0f') => {
                self.expand_all = !self.expand_all;
                self.content_version += 1;
                self.draw_dirty = true;
                let msg = if self.expand_all {
                    "All blocks expanded"
                } else {
                    "All blocks collapsed"
                };
                self.show_toast(msg, ToastLevel::Info);
                let _ = self.draw();
                return Ok(None);
            }

            KeyCode::Char('l') if k.modifiers.contains(KeyModifiers::CONTROL) => {
                self.lines.clear();
                self.content_version += 1;
                self.pending_submit_images.clear();
                self.pending_paste_images.clear();
                self.draw_dirty = true;
            }
            KeyCode::Char('b') if k.modifiers.contains(KeyModifiers::CONTROL) => {
                self.toggle_sidebar();
                let _ = self.draw();
                return Ok(None);
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
                        crate::autocomplete::AutocompleteOverlay::upsert_on_stack(
                            &mut self.overlays,
                            suggestions,
                            word_start,
                            cursor_pos,
                        );
                        self.draw_dirty = true;
                        return Ok(None);
                    }
                }

                // Trigger Tool/MCP completion (Tab on ':')
                if partial.starts_with(':') {
                    let suggestions = self.tool_ac.completions(&input_text, cursor_pos);
                    if !suggestions.is_empty() {
                        crate::autocomplete::AutocompleteOverlay::upsert_on_stack(
                            &mut self.overlays,
                            suggestions,
                            word_start,
                            cursor_pos,
                        );
                        self.draw_dirty = true;
                        return Ok(None);
                    }
                }

                // Trigger Next Step completion (Tab on '?')
                if partial.starts_with('?') {
                    let suggestions = self.next_step_ac.completions(&input_text, cursor_pos);
                    if !suggestions.is_empty() {
                        crate::autocomplete::AutocompleteOverlay::upsert_on_stack(
                            &mut self.overlays,
                            suggestions,
                            word_start,
                            cursor_pos,
                        );
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
                            matches
                                .last()
                                .cloned()
                                .unwrap_or_else(|| suggestion.clone())
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
                        if self.selection_retained {
                            self.clear_selection();
                        }

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
                                let suggestions =
                                    self.slash_ac.completions(&input_text, cursor_pos);
                                if !suggestions.is_empty() {
                                    crate::autocomplete::AutocompleteOverlay::upsert_on_stack(
                                        &mut self.overlays,
                                        suggestions,
                                        cursor_pos.saturating_sub(1),
                                        cursor_pos,
                                    );
                                }
                            }
                            if let KeyCode::Char('@') = k.code {
                                let input_text = self.editor.text();
                                let cursor_pos = self.editor.cursor_pos();
                                let at_pos = cursor_pos.saturating_sub(1);
                                let before_at = &input_text[..at_pos];
                                let is_start_or_after_space = before_at.is_empty()
                                    || before_at.ends_with(|c: char| c.is_whitespace());

                                if is_start_or_after_space {
                                    if self.editor_input_mode()
                                        != crate::editor::InputMode::SlashCommand
                                    {
                                        self.overlays.push(Box::new(crate::app::PickerState::new(
                                            at_pos,
                                            String::new(),
                                            &self.file_ac,
                                        )));
                                    } else {
                                        let suggestions = self.slash_ac.at_completions("");
                                        if !suggestions.is_empty() {
                                            crate::autocomplete::AutocompleteOverlay::upsert_on_stack(
                                                &mut self.overlays,
                                                suggestions,
                                                at_pos,
                                                cursor_pos,
                                            );
                                        }
                                    }
                                }
                            }
                        }
                        return Ok(None);
                    }
                    EditorAction::Submit(text) => {
                        if !text.trim().is_empty() {
                            self.dispatch(crate::app::reducer::TuiAction::SendMessage(
                                text.clone(),
                            ));
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
                    EditorAction::Unhandled(_) => {
                        if self.selection_retained {
                            self.clear_selection();
                        }
                    }
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
                if s == "/quote" || s == "quote" {
                    self.quote_selection_to_prompt();
                    return Ok(None);
                } else if s == "/conceal" || s == "conceal" {
                    self.toggle_conceal();
                    return Ok(None);
                } else if s == "/timestamps" || s == "timestamps" {
                    self.toggle_timestamps();
                    return Ok(None);
                } else if s == "/stash" || s == "stash" {
                    self.stash_prompt();
                    return Ok(None);
                } else if s == "/save" || s == "save" {
                    self.save_settings();
                    return Ok(None);
                } else if s.starts_with('/') {
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

    /// One overlay dispatch seam for idle, active and synchronous questions.
    pub fn dispatch_overlay_event(
        &mut self,
        event: &Event,
    ) -> Result<(bool, Option<Option<String>>)> {
        let dispatch = dispatch_overlay_stack(&mut self.overlays, event);
        self.draw_dirty |= dispatch.dirty;
        let submission = match dispatch.action {
            Some(action) => self.process_overlay_action(action)?,
            None => None,
        };
        Ok((dispatch.owned, submission))
    }

    pub fn handle_lua_key(&mut self, key: KeyEvent) -> bool {
        let mut name = String::new();
        for (modifier, prefix) in [
            (KeyModifiers::CONTROL, "C-"),
            (KeyModifiers::ALT, "A-"),
            (KeyModifiers::SHIFT, "S-"),
        ] {
            if key.modifiers.contains(modifier) {
                name.push_str(prefix);
            }
        }
        match key.code {
            KeyCode::Char(c) => name.push(c),
            KeyCode::Enter => name.push_str("Enter"),
            KeyCode::Esc => name.push_str("Esc"),
            KeyCode::Tab => name.push_str("Tab"),
            KeyCode::BackTab => name.push_str("BackTab"),
            KeyCode::Backspace => name.push_str("Backspace"),
            KeyCode::Delete => name.push_str("Delete"),
            KeyCode::Up => name.push_str("Up"),
            KeyCode::Down => name.push_str("Down"),
            KeyCode::Left => name.push_str("Left"),
            KeyCode::Right => name.push_str("Right"),
            _ => return false,
        }
        let handled = self
            .lua_engine
            .as_ref()
            .is_some_and(|lua| lua.handle_keybinding(&name));
        if handled {
            self.refresh_lua_ui();
            self.draw_dirty = true;
        }
        handled
    }

    pub fn handle_focused_slot_key(&mut self, key: KeyEvent) -> bool {
        use crate::slots::{FocusRegion, UiSlot};
        if key.code == KeyCode::Char('f') && key.modifiers.contains(KeyModifiers::CONTROL) {
            self.cycle_focus();
            return true;
        }
        if self.focused_region == FocusRegion::Input {
            return false;
        }
        if key.code == KeyCode::Esc {
            self.focused_region = FocusRegion::Input;
            for slot in [UiSlot::Sidebar, UiSlot::Header, UiSlot::Footer] {
                if let Some(widget) = self.slots.get_mut(slot) {
                    widget.set_focused(false);
                }
            }
            self.draw_dirty = true;
            return true;
        }
        let consumed = self
            .focused_region
            .to_slot()
            .and_then(|slot| self.slots.get_mut(slot))
            .is_some_and(|widget| widget.handle_input(key));
        if !consumed {
            self.handle_scroll_key(key.code, key.modifiers);
        }
        self.draw_dirty = true;
        true // A focused slot owns even unrecognized keys.
    }

    pub fn has_lua_host_work(&self, tools_ready: bool) -> bool {
        self.lua_engine.as_ref().is_some_and(|lua| {
            !lua.command_queue
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .is_empty()
                || (tools_ready
                    && !lua
                        .tool_queue
                        .lock()
                        .unwrap_or_else(|e| e.into_inner())
                        .is_empty())
        })
    }

    /// Bound callback work per tick and refresh extension slots once per batch.
    pub fn pump_lua_ui_events(&mut self) {
        let Some(lua) = &self.lua_engine else {
            return;
        };
        if lua.pump_ui_events() {
            self.refresh_lua_ui();
            self.draw_dirty = true;
        }
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

/// Compute the new scroll position after a half-page up keypress (ctrl+alt+u).
pub(crate) fn scroll_half_page_up(current: usize, viewport_h: u16) -> usize {
    let step = ((viewport_h as usize) / 2).max(1);
    current.saturating_add(step)
}

/// Compute the new scroll position after a half-page down keypress (ctrl+alt+d).
/// Returns `(new_scroll, should_follow)`.
pub(crate) fn scroll_half_page_down(current: usize, viewport_h: u16) -> (usize, bool) {
    let step = ((viewport_h as usize) / 2).max(1);
    let new = current.saturating_sub(step);
    (new, new == 0)
}

/// Compute accelerated scroll delta (macOS-style scroll ramp).
pub(crate) fn compute_accelerated_scroll(
    base_speed: u16,
    acceleration_enabled: bool,
    streak: u16,
) -> usize {
    let base = (base_speed as usize).max(1);
    if !acceleration_enabled || streak <= 1 {
        base
    } else {
        let multiplier = 1 + ((streak as usize) / 2).min(4);
        base * multiplier
    }
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

    #[test]
    fn test_scroll_half_page() {
        assert_eq!(scroll_half_page_up(0, 40), 20);
        assert_eq!(scroll_half_page_up(10, 40), 30);

        let (new, follow) = scroll_half_page_down(30, 40);
        assert_eq!(new, 10);
        assert!(!follow);

        let (new, follow) = scroll_half_page_down(10, 40);
        assert_eq!(new, 0);
        assert!(follow);
    }

    #[test]
    fn test_compute_accelerated_scroll() {
        assert_eq!(compute_accelerated_scroll(3, false, 5), 3);
        assert_eq!(compute_accelerated_scroll(3, true, 1), 3);
        assert_eq!(compute_accelerated_scroll(3, true, 2), 6);
        assert_eq!(compute_accelerated_scroll(3, true, 4), 9);
        assert_eq!(compute_accelerated_scroll(3, true, 10), 15);
    }
}
