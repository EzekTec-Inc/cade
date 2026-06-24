use super::*;
use crate::colors::ThemeColorsExt;

impl TuiApp {
    /// Apply a new theme dynamically from the backend and force a redraw.
    /// Commit any in-progress streaming, push a line, and redraw.
    pub fn push(&mut self, line: RenderLine) -> Result<()> {
        self.commit_streaming_inner();
        self.commit_reasoning_inner();
        let is_tool_result = matches!(line, RenderLine::ToolResult { .. });
        self.lines.push(line);
        self.content_version += 1;

        if self.follow {
            // User is following — auto-scroll to show new content.
            if is_tool_result {
                self.scroll_instant(self.rows_from_last_tool_call());
            } else {
                self.scroll_instant(0);
            }
            self.pending_lines = 0;
        } else {
            // User scrolled up — don't steal their position.
            // Increment pending_lines so the "↓ N new" badge appears.
            self.pending_lines += 1;
        }
        self.signals.content_changed.write(true);
        let scroll_before = self.scroll;
        self.draw()?;
        if is_tool_result && self.scroll != scroll_before {
            return self.draw();
        }
        Ok(())
    }

    /// Count visual rows from the most recent `ToolCall` entry (inclusive) to
    /// the end of `self.lines`.  The result is used as the scroll offset so
    /// that the ToolCall header appears at the top of the viewport when the
    /// corresponding ToolResult is pushed.
    pub(crate) fn rows_from_last_tool_call(&self) -> usize {
        let main_w = if self.term_width >= crate::app::SIDEBAR_BREAKPOINT {
            let sidebar_w = crate::app::SIDEBAR_WIDTH.min(self.term_width.saturating_sub(24));
            self.term_width.saturating_sub(sidebar_w)
        } else {
            self.term_width
        };
        let cw = main_w.saturating_sub(4).max(1);

        let mut total: u16 = 0;
        for entry in build_timeline_entries(&self.lines).into_iter().rev() {
            total = total.saturating_add(entry.visual_rows_with_state(
                cw,
                self.expand_all,
                &self.expanded_items,
                &self.colors,
                self.use_nerd_fonts,
            ));
            if entry.is_tool_call() {
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
        self.content_version += 1;
    }

    /// Append a streaming chunk and redraw (throttled — max ~60 FPS).
    pub fn push_streaming_chunk(&mut self, text: &str) -> Result<()> {
        self.commit_reasoning_inner();
        if !self.streaming_active {
            self.signals.streaming.write(true);
            // First chunk of a new agent response — always snap to bottom so the
            // analysis is immediately visible.  push(ToolResult) may have scrolled
            // up to show the ToolCall header; that view is correct while the tool
            // was running, but as soon as the agent starts responding the viewport
            // must follow the output.
            if self.follow {
                self.scroll_instant(0);
                self.pending_lines = 0;
            }
        }
        // Subsequent chunks of the same response preserve scroll (V-01):
        // if the user scrolled up mid-stream to read history, leave them there.
        self.streaming_active = true;
        self.streaming_text.push_str(text);
        self.update_plan_state();
        self.draw_throttled()
    }
    pub(crate) fn update_plan_state(&mut self) {
        // Legacy streaming-regex plan detection removed.
        // Plans are now set explicitly via the set_plan() / update_plan_step() methods,
        // driven by the SetPlan and UpdatePlan tool calls.
        //
        // [DONE:N] markers in streaming text are still honoured for backward
        // compatibility with any in-flight conversations.
        if let Some(plan) = &mut self.active_plan {
            let mut changed = false;
            for caps in done_regex().captures_iter(&self.streaming_text) {
                if let Ok(id) = caps[1].parse::<usize>()
                    && let Some(step) = plan.steps.iter_mut().find(|s| s.id == id)
                    && !step.is_done
                {
                    step.is_done = true;
                    changed = true;
                }
            }
            if changed {
                self.draw_dirty = true;
            }
        }
    }

    /// Set the plan panel steps from an explicit `set_plan` tool call.
    /// Replaces any existing plan and makes the panel visible.
    pub fn set_plan(&mut self, steps: Vec<String>) {
        if steps.is_empty() {
            self.active_plan = None;
            return;
        }
        self.active_plan = Some(PlanState {
            steps: steps
                .into_iter()
                .enumerate()
                .map(|(i, desc)| PlanStep {
                    id: i + 1,
                    description: desc,
                    is_done: false,
                })
                .collect(),
            is_visible: true,
            scroll_offset: 0,
        });
        self.signals.plan_changed.write(true);
        self.draw_dirty = true;
    }

    /// Mark a step done/undone from an explicit `UpdatePlan` tool call.
    /// step_id is 1-based.  Returns false if the id is out of range.
    pub fn update_plan_step(&mut self, step_id: usize, done: bool) -> bool {
        if let Some(plan) = &mut self.active_plan
            && let Some(step) = plan.steps.iter_mut().find(|s| s.id == step_id)
        {
            step.is_done = done;
            self.signals.plan_changed.write(true);
            self.draw_dirty = true;
            return true;
        }
        false
    }

    /// Read `.cade-todo.md` from the current directory and return its contents,
    /// or a message explaining it doesn't exist yet.
    pub fn read_todo_file() -> String {
        let path = match std::env::current_dir() {
            Ok(d) => d.join(".cade-todo.md"),
            Err(_) => return "Could not determine current directory.".to_string(),
        };
        match std::fs::read_to_string(&path) {
            Ok(content) if content.trim().is_empty() => {
                format!("{} exists but is empty.", path.display())
            }
            Ok(content) => content,
            Err(_) => format!(
                "No todo file found at {}.\nAsk the agent to create one with the TodoWrite tool.",
                path.display()
            ),
        }
    }

    /// Append a reasoning chunk (accumulated; committed as header on done).
    pub fn push_reasoning_chunk(&mut self, text: &str) {
        self.reasoning_active = true;
        self.reasoning_text.push_str(text);
    }

    /// Commit any in-progress assistant streaming to `lines`.
    pub fn commit_streaming(&mut self) -> Result<()> {
        self.commit_streaming_inner();
        self.signals.streaming.write(false);
        // Snap to bottom when streaming commits — the completed response must
        // be visible.  Only mid-stream chunks (push_streaming_chunk) preserve
        // the user's scroll position; once the response is fully committed here
        // we always show it.
        if self.follow {
            self.scroll_instant(0);
            self.pending_lines = 0;
        }
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
        self.streaming_reveal_len = 0;
        self.reasoning_text.clear();
        self.reasoning_active = false;
    }

    pub fn has_streaming(&self) -> bool {
        self.streaming_active
    }

    /// Toggle scroll-wheel capture on/off.
    /// When disabled: scroll capture is off, terminal handles all mouse events natively.
    /// When enabled: CADE captures scroll-wheel events (text selection always works).
    pub fn toggle_mouse_capture(&mut self) {
        self.mouse_capture_disabled = !self.mouse_capture_disabled;
        if self.mouse_capture_disabled {
            let _ = crossterm::execute!(std::io::stdout(), crossterm::event::DisableMouseCapture);
            self.show_toast(
                "Mouse capture disabled — native text selection restored",
                ToastLevel::Info,
            );
        } else {
            let _ = crossterm::execute!(std::io::stdout(), crossterm::event::EnableMouseCapture);
            self.show_toast("Mouse capture enabled", ToastLevel::Info);
        }
    }

    pub fn show_toast(&mut self, message: impl Into<String>, level: ToastLevel) {
        self.toast = Some(Toast {
            message: message.into(),
            level,
            created_at: Instant::now(),
            ttl: std::time::Duration::from_secs(3),
        });
    }

    /// Clear all content (e.g. /clear).
    pub fn clear_content(&mut self) -> Result<()> {
        self.lines.clear();
        self.expanded_items.clear();
        self.discard_streaming();
        self.content_version += 1;
        self.scroll_instant(0);
        self.follow = true;
        self.draw()
    }
    pub(crate) fn commit_streaming_inner(&mut self) {
        if self.streaming_active {
            let text = std::mem::take(&mut self.streaming_text);
            let clean = crate::app::strip_orchestrator_prompts(&text);
            if !clean.trim().is_empty() {
                self.lines
                    .push(RenderLine::AssistantText(clean.into_owned()));
                self.content_version += 1;
            }
            self.streaming_active = false;
            self.streaming_reveal_len = 0;
        }
    }

    /// Commit reasoning state without drawing.  Public so callers that
    /// batch multiple mutations (e.g. commit reasoning + push streaming chunk)
    /// can avoid redundant intermediate draws.
    pub fn commit_reasoning_inner(&mut self) {
        if self.reasoning_active {
            let text = std::mem::take(&mut self.reasoning_text);
            let clean = crate::app::strip_orchestrator_prompts(&text);
            let words = clean.split_whitespace().count();
            if words > 0 {
                self.lines.push(RenderLine::Reasoning {
                    words,
                    content: clean.into_owned(),
                });
                self.content_version += 1;
            }
            self.reasoning_active = false;
        }
    }

    /// Push an empty `LiveOutput` entry and return its index in `self.lines`.
    /// Call this once before streaming begins; pass the returned index to
    /// `append_live_output_line` and `finish_live_output`.
    pub fn begin_live_output(&mut self, max_visible: usize) -> usize {
        self.commit_streaming_inner();
        self.commit_reasoning_inner();
        self.lines.push(RenderLine::LiveOutput {
            lines: Vec::new(),
            max_visible,
            done: false,
        });
        self.content_version += 1;
        self.lines.len() - 1
    }

    /// Append one output line to the `LiveOutput` at `idx` and redraw
    /// (throttled — max ~60 FPS).  No-op if `idx` is not a `LiveOutput`.
    pub fn append_live_output_line(&mut self, idx: usize, line: String) -> Result<()> {
        if let Some(RenderLine::LiveOutput { lines, .. }) = self.lines.get_mut(idx) {
            lines.push(line);
            self.content_version += 1;
        }
        if self.follow {
            self.scroll_instant(0);
        }
        self.draw_throttled()
    }

    /// Mark the `LiveOutput` at `idx` as finished (subprocess has exited).
    /// Redraws so the final state is shown before the caller returns.
    pub fn finish_live_output(&mut self, idx: usize) -> Result<()> {
        if let Some(RenderLine::LiveOutput { done, .. }) = self.lines.get_mut(idx) {
            *done = true;
            self.content_version += 1;
        }
        if self.follow {
            self.scroll_instant(0);
        }
        self.draw()
    }

    pub fn suspend(&mut self) -> Result<()> {
        crossterm::terminal::disable_raw_mode().map_err(|e| crate::Error::Custom(e.to_string()))?;
        crossterm::execute!(
            self.terminal.backend_mut(),
            crossterm::terminal::LeaveAlternateScreen
        )
        .map_err(|e| crate::Error::Custom(e.to_string()))?;
        Ok(())
    }

    pub fn resume(&mut self) -> Result<()> {
        crossterm::terminal::enable_raw_mode().map_err(|e| crate::Error::Custom(e.to_string()))?;
        crossterm::execute!(
            self.terminal.backend_mut(),
            crossterm::terminal::EnterAlternateScreen
        )
        .map_err(|e| crate::Error::Custom(e.to_string()))?;
        self.terminal
            .clear()
            .map_err(|e| crate::Error::Custom(e.to_string()))?;
        self.draw()?;
        Ok(())
    }

    /// Temporarily suspends the TUI, runs the provided closure, and then restores it.
    pub fn suspend_for<F>(&mut self, f: F) -> Result<()>
    where
        F: FnOnce(),
    {
        self.suspend()?;
        f();
        self.resume()?;
        Ok(())
    }

    pub fn update_model(&mut self, model: String) {
        self.model = model;
    }

    pub fn update_mode(&mut self, mode: PermissionMode) {
        self.mode = mode;
        self.signals.mode_changed.write(true);
    }

    pub fn update_agent_name(&mut self, name: String) {
        self.agent_name = name;
    }

    pub fn set_last_status(&mut self, s: Option<String>) {
        self.last_status = s;
    }

    /// Start the thinking animation.  Returns the shared text Arc so callers
    /// can update the status text (e.g. assessing timer, tool name updates).
    pub fn start_thinking(&mut self, text: impl Into<String>) -> Arc<Mutex<String>> {
        self.scroll_instant(0); // snap to bottom at the start of every agent turn
        let arc = Arc::new(Mutex::new(text.into()));
        self.thinking = Some(ThinkingState {
            text: arc.clone(),
            started: Instant::now(),
        });
        self.signals.thinking.write(true);
        arc
    }

    /// Update the thinking text from the animation/assessing timer.
    pub fn update_thinking_text(&mut self, text: String) {
        if let Some(ts) = &self.thinking {
            let mut guard = ts.text.lock();
            *guard = text;
        }
    }

    /// Stop the thinking animation.  Returns elapsed seconds (for summary line).
    pub fn stop_thinking(&mut self) -> u64 {
        let secs = self
            .thinking
            .as_ref()
            .map(|ts| ts.started.elapsed().as_secs())
            .unwrap_or(0);
        self.thinking = None;
        self.signals.thinking.write(false);
        secs
    }

    pub fn open_theme_picker(
        &mut self,
        themes: Vec<cade_core::resources::themes::Theme>,
        original_theme: crate::colors::ThemeColors,
    ) {
        // U5: init cursor at the position of the currently active theme
        let initial_cursor = themes
            .iter()
            .enumerate()
            .position(|(_, t)| {
                let tc = t;
                tc.c_primary() == original_theme.c_primary()
                    && tc.c_bg_base() == original_theme.c_bg_base()
            })
            .unwrap_or(0);
        let tp = ThemePickerState {
            query: String::new(),
            filtered_indices: (0..themes.len()).collect(),
            themes,
            cursor: initial_cursor,
            original_theme,
            pending_action: None,
        };
        self.overlays.push(Box::new(tp));
        self.draw_dirty = true;
    }

    pub fn set_context_pct(&mut self, pct: u8) {
        let p = pct.min(99);
        self.context_pct = Some(p);
        // Record in history for sparkline (keep last 50 entries).
        self.token_history.push(p);
        if self.token_history.len() > 50 {
            self.token_history.remove(0);
        }
        self.refresh_lua_ui();
    }

    /// Increment the turn counter (called when a user message is submitted).
    pub fn increment_turn(&mut self) {
        self.turn_count = self.turn_count.saturating_add(1);
    }

    // -- ImageChannel (side-channel for image pastes) --

    pub fn handle_image_paste(&mut self, media_type: &str, data: String, width: u32, height: u32) {
        self.image_counter += 1;
        let id = self.image_counter;
        self.pending_paste_images.push(crate::editor::ImageEntry {
            id,
            media_type: media_type.to_string(),
            data,
            width,
            height,
        });
        let marker = format!("[image #{id}: {width}x{height}]");
        self.editor.insert_str(&marker);
        self.editor.insert_newline();
    }

    pub fn drain_images(&mut self) -> Vec<crate::editor::ImageEntry> {
        let mut extracted = Vec::new();
        let mut text = self.editor.text();
        let current_images = std::mem::take(&mut self.pending_paste_images);
        for img in current_images {
            let marker_prefix = format!("[image #{}:", img.id);
            if text.contains(&marker_prefix)
                && let Some(start) = text.find(&marker_prefix)
                && let Some(end_offset) = text[start..].find(']')
            {
                let end = start + end_offset + 1;
                text.replace_range(start..end, "");
                extracted.push(img);
            }
        }
        if !extracted.is_empty() {
            self.editor.set_text(text);
        }
        self.image_counter = 0;
        extracted
    }

    // -- Mode hint parsing --

    /// Parse the editor's `mode_hint()` into the concrete `InputMode` enum.
    pub fn editor_input_mode(&self) -> InputMode {
        match self.editor.mode_hint().as_deref() {
            Some("slash") => InputMode::SlashCommand,
            Some("bash") => InputMode::BashCommand { silent: false },
            Some("bash:silent") => InputMode::BashCommand { silent: true },
            _ => InputMode::Regular,
        }
    }

    // -- Shared scroll handler (Fix E) --
    //
    // Unified scroll logic used by both the idle input loop (input.rs) and the
    // tick task during agent processing (turn_loop/agent.rs).  All scroll
    // mutations go through scroll_target for smooth animation.

    /// Handle a keyboard scroll event.  Returns `true` if the key was consumed.
    pub fn handle_scroll_key(
        &mut self,
        code: crossterm::event::KeyCode,
        _modifiers: crossterm::event::KeyModifiers,
    ) -> bool {
        use crossterm::event::KeyCode;

        match code {
            // Shift+K — scroll up 10 lines
            KeyCode::Char('K') => {
                self.follow = false;
                self.scroll_target = self.scroll_target.saturating_add(10);
                self.draw_dirty = true;
                true
            }
            // Shift+J — snap to bottom
            KeyCode::Char('J') => {
                self.scroll_target = 0;
                self.follow = true;
                self.pending_lines = 0;
                self.draw_dirty = true;
                true
            }
            // PageUp — scroll up by viewport height
            KeyCode::PageUp => {
                self.follow = false;
                let vh = crossterm::terminal::size()
                    .map(|(_, h)| h.saturating_sub(super::FIXED_ROWS + super::MAX_INPUT_ROWS))
                    .unwrap_or(20);
                self.scroll_target = crate::app::input::scroll_page_up(self.scroll_target, vh);
                self.draw_dirty = true;
                true
            }
            // PageDown — scroll down by viewport height
            KeyCode::PageDown => {
                let vh = crossterm::terminal::size()
                    .map(|(_, h)| h.saturating_sub(super::FIXED_ROWS + super::MAX_INPUT_ROWS))
                    .unwrap_or(20);
                let (new_target, should_follow) =
                    crate::app::input::scroll_page_down(self.scroll_target, vh);
                self.scroll_target = new_target;
                if should_follow {
                    self.follow = true;
                    self.pending_lines = 0;
                }
                self.draw_dirty = true;
                true
            }
            _ => false,
        }
    }

    /// Handle a mouse scroll event.  Returns `true` if the event was consumed.
    pub fn handle_scroll_mouse(&mut self, kind: crossterm::event::MouseEventKind) -> bool {
        use crossterm::event::MouseEventKind;

        match kind {
            MouseEventKind::ScrollUp => {
                self.follow = false;
                self.scroll_target = self.scroll_target.saturating_add(3);
                self.draw_dirty = true;
                true
            }
            MouseEventKind::ScrollDown => {
                self.scroll_target = self.scroll_target.saturating_sub(3);
                if self.scroll_target == 0 {
                    self.follow = true;
                    self.pending_lines = 0;
                }
                self.draw_dirty = true;
                true
            }
            _ => false,
        }
    }

    /// Build the prepared-timeline-entry list the same way `render_frame` does.
    /// Uses a content-version cache to avoid re-parsing markdown/ANSI on every mouse click.
    fn build_prepared_entries(&mut self) -> Vec<super::timeline::PreparedTimelineEntry> {
        use std::collections::hash_map::DefaultHasher;
        use std::hash::{Hash, Hasher};

        let timeline_w = self.messages_area.width.saturating_sub(4).max(1) as usize;

        // Derive a stable hash of expanded_items for cache invalidation.
        let expanded_hash = {
            let mut h = DefaultHasher::new();
            let mut items: Vec<_> = self.expanded_items.iter().collect();
            items.sort();
            for k in &items {
                k.hash(&mut h);
            }
            h.finish()
        };

        // Try cache for the non-streaming portion.
        let mut prepared = if let Some(ref cache) = self.prepared_cache {
            if cache.version == self.content_version
                && cache.timeline_w == timeline_w
                && cache.expand_all == self.expand_all
                && cache.expanded_hash == expanded_hash
            {
                // Cache hit — avoid full rebuild.
                cache.entries.clone()
            } else {
                // Cache miss — rebuild non-streaming entries.
                let entries = super::timeline::build_timeline_entries(&self.lines);
                let p = super::timeline::prepare_timeline_entries(
                    &entries,
                    timeline_w,
                    self.expand_all,
                    &self.expanded_items,
                    &self.colors,
                    self.use_nerd_fonts,
                );
                self.prepared_cache = Some(super::timeline::PreparedCache {
                    entries: p.clone(),
                    version: self.content_version,
                    timeline_w,
                    expand_all: self.expand_all,
                    expanded_hash,
                });
                p
            }
        } else {
            let entries = super::timeline::build_timeline_entries(&self.lines);
            let p = super::timeline::prepare_timeline_entries(
                &entries,
                timeline_w,
                self.expand_all,
                &self.expanded_items,
                &self.colors,
                self.use_nerd_fonts,
            );
            self.prepared_cache = Some(super::timeline::PreparedCache {
                entries: p.clone(),
                version: self.content_version,
                timeline_w,
                expand_all: self.expand_all,
                expanded_hash,
            });
            p
        };

        // Streaming entry (always rebuilt — changes every tick, not cached).
        if self.streaming_active {
            let full =
                crate::app::strip_orchestrator_prompts(&self.streaming_text).into_owned();
            let reveal = self.streaming_reveal_len.min(full.len());
            let visible_streaming = &full[..reveal];
            let next_index = self.lines.len();
            let streaming_entry =
                super::timeline::TimelineEntry::streaming(next_index, visible_streaming);
            let mut stream_lines = Vec::new();
            let effective_w = timeline_w.saturating_sub(2);
            streaming_entry.render_with_state(
                effective_w,
                self.expand_all,
                &self.expanded_items,
                &mut stream_lines,
                &self.colors,
                self.use_nerd_fonts,
            );
            let stream_rows: u16 = stream_lines
                .iter()
                .map(|l| crate::app::render::count_wrapped_rows(l, effective_w as u16))
                .sum();
            prepared.push(super::timeline::PreparedTimelineEntry {
                lines: stream_lines,
                rows: stream_rows,
                card_style: super::timeline::CardStyle::Assistant,
            });
        }

        prepared
    }

    /// Given a mouse position, return the index into the prepared-entries list
    /// of the entry the cursor is over, or `None`.
    fn find_entry_idx_at_mouse(
        &self,
        _mx: u16,
        my: u16,
        inner: ratatui::layout::Rect,
        prepared: &[super::timeline::PreparedTimelineEntry],
    ) -> Option<usize> {
        let total_visual: u16 = prepared
            .iter()
            .map(|p| p.rows as u32)
            .sum::<u32>()
            .min(u16::MAX as u32) as u16;
        let visible = inner.height;
        let max_skip = total_visual.saturating_sub(visible);
        let effective_up = (self.scroll as u16).min(max_skip);
        let visible_start = max_skip.saturating_sub(effective_up);

        let clicked_visual_row = my.saturating_sub(inner.y) + visible_start;

        let mut item_start: u16 = 0;
        prepared.iter().position(|item| {
            let item_end = item_start.saturating_add(item.rows);
            let hit = clicked_visual_row >= item_start && clicked_visual_row < item_end;
            item_start = item_end;
            hit
        })
    }

    /// Called on MouseDown(Left) to store which entry is under the cursor
    /// so the renderer can highlight it (visual "press to select" feedback).
    /// Returns `true` if the click was inside the messages area.
    pub fn set_mouse_selection(&mut self, mouse: &crossterm::event::MouseEvent) -> bool {
        use ratatui::layout::Rect;

        let inner = Rect {
            x: self.messages_area.x + 2,
            y: self.messages_area.y + 1,
            width: self.messages_area.width.saturating_sub(4),
            height: self.messages_area.height.saturating_sub(2),
        };

        if inner.width == 0 || inner.height == 0 {
            return false;
        }

        let mx = mouse.column;
        let my = mouse.row;
        if mx < inner.x
            || mx >= inner.x + inner.width
            || my < inner.y
            || my >= inner.y + inner.height
        {
            self.mouse_selection = None;
            return false;
        }

        let prepared = self.build_prepared_entries();
        let idx = self.find_entry_idx_at_mouse(mx, my, inner, &prepared);
        self.mouse_selection = idx;
        self.draw_dirty = true;
        idx.is_some()
    }

    /// Handle a left-mouse-up click on the messages viewport.
    /// Maps the terminal coordinate to a RenderLine and copies its text to clipboard.
    /// Returns `true` if the click was consumed.
    pub fn click_to_copy(&mut self, mouse: &crossterm::event::MouseEvent) -> bool {
        use ratatui::layout::Rect;

        let inner = Rect {
            x: self.messages_area.x + 2,
            y: self.messages_area.y + 1,
            width: self.messages_area.width.saturating_sub(4),
            height: self.messages_area.height.saturating_sub(2),
        };

        if inner.width == 0 || inner.height == 0 {
            return false;
        }

        let mx = mouse.column;
        let my = mouse.row;
        if mx < inner.x
            || mx >= inner.x + inner.width
            || my < inner.y
            || my >= inner.y + inner.height
        {
            return false;
        }

        let prepared = self.build_prepared_entries();
        let found_index = self.find_entry_idx_at_mouse(mx, my, inner, &prepared);

        let Some(clicked_line_index) = found_index else {
            self.show_toast(
                "No content at click position",
                crate::app::ToastLevel::Warning,
            );
            self.draw_dirty = true;
            return false;
        };

        let Some(line) = self.lines.get(clicked_line_index) else {
            self.show_toast(
                "No content at click position",
                crate::app::ToastLevel::Warning,
            );
            self.draw_dirty = true;
            return false;
        };
        let text = crate::app::copy_overlay::render_line_plain_text(line);
        if text.is_empty() {
            self.show_toast(
                "Nothing to copy on this line",
                crate::app::ToastLevel::Warning,
            );
            self.draw_dirty = true;
            return false;
        }

        // Copy to clipboard (OSC 52 + arboard fallback).
        crate::app::clipboard::write_to_clipboard(&text);

        // Store the clicked line index (in the prepared/timeline-entries list)
        // so the next render flash-highlights it.
        self.copy_highlight = Some((clicked_line_index, std::time::Instant::now()));

        let label = crate::app::copy_overlay::render_line_label(line).unwrap_or("Content");
        self.show_toast(
            format!("Copied {label} (click to copy)"),
            crate::app::ToastLevel::Success,
        );
        self.draw_dirty = true;

        true
    }

    /// Toggle the folding/expansion of the most recent collapsible timeline item (Ctrl+G).
    pub fn toggle_last_collapsible_item(&mut self) {
        use crate::app::timeline::{TimelineItemKind, build_timeline_entries};

        let entries = build_timeline_entries(&self.lines);
        for entry in entries.into_iter().rev() {
            if matches!(
                entry.key.kind,
                TimelineItemKind::ToolResult
                    | TimelineItemKind::Reasoning
                    | TimelineItemKind::LiveOutput
            ) {
                let key = entry.key;
                if self.expanded_items.contains(&key) {
                    self.expanded_items.remove(&key);
                    self.show_toast("Collapsed item details", ToastLevel::Info);
                } else {
                    self.expanded_items.insert(key);
                    self.show_toast("Expanded item details", ToastLevel::Info);
                }
                self.draw_dirty = true;
                break;
            }
        }
    }
}
