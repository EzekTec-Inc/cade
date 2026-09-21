use super::*;
use crate::colors::ThemeColorsExt;

impl TuiApp {
    /// Commit any in-progress streaming, push a line, and redraw.
    pub fn push(&mut self, line: RenderLine) -> Result<()> {
        self.commit_streaming_inner();
        self.commit_reasoning_inner();
        if matches!(line, RenderLine::UserMessage(_)) {
            self.turn_start_time = Some(std::time::Instant::now());
        }
        self.lines.push(line);
        self.content_version += 1;

        if self.follow {
            // User is following — auto-scroll to show new content.
            self.scroll_instant(0);
            self.pending_lines = 0;
        } else {
            // User scrolled up — don't steal their position.
            // Increment pending_lines so the "↓ N new" badge appears.
            self.pending_lines += 1;
        }
        self.signals.content_changed.write(true);
        self.draw()?;
        Ok(())
    }

    /// Snap scroll position to the bottom of the viewport and re-enable follow mode.
    pub fn scroll_to_bottom(&mut self) {
        self.follow = true;
        self.scroll_instant(0);
        self.pending_lines = 0;
        self.draw_dirty = true;
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
        let now = std::time::Instant::now();
        if !self.streaming_active {
            self.signals.streaming.write(true);
            self.streaming_start_time = Some(now);
            if let Some(turn_start) = self.turn_start_time {
                self.ttft_secs = Some(now.duration_since(turn_start).as_secs_f64());
            }
            self.streaming_tokens = 0;
            self.streaming_revealed_len = 0;
            if self.follow {
                self.scroll_instant(0);
                self.pending_lines = 0;
            }
        }
        self.streaming_active = true;
        let delta_tokens = (text.split_whitespace().count() * 4 / 3).max(1);
        self.streaming_tokens += delta_tokens;
        self.streaming_text.push_str(text);
        // Refresh the prompt-stripped display copy once per chunk — draw frames
        // reuse it instead of re-running the strip regex over the whole stream.
        self.streaming_display =
            crate::app::strip_orchestrator_prompts(&self.streaming_text).into_owned();
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
            if let Some(re) = done_regex() {
                for caps in re.captures_iter(&self.streaming_text) {
                    if let Ok(id) = caps[1].parse::<usize>()
                        && let Some(step) = plan.steps.iter_mut().find(|s| s.id == id)
                        && !step.is_done
                    {
                        step.is_done = true;
                        changed = true;
                    }
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

    /// Toggle sidebar visibility override and show a brief toast notification.
    pub fn toggle_sidebar(&mut self) -> bool {
        self.sidebar_hidden = !self.sidebar_hidden;
        self.draw_dirty = true;
        let (msg, level) = if self.sidebar_hidden {
            ("Sidebar hidden", ToastLevel::Info)
        } else {
            ("Sidebar visible", ToastLevel::Info)
        };
        self.show_toast(msg, level);
        !self.sidebar_hidden
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

    /// Append a reasoning chunk.  The thinking text is streamed live into the
    /// viewport (via `reasoning_active` + the layout engine's active-reasoning
    /// entry) and collapsed into a `RenderLine::Reasoning` header on commit.
    pub fn push_reasoning_chunk(&mut self, text: &str) {
        self.reasoning_active = true;
        self.reasoning_text.push_str(text);
        self.reasoning_display =
            crate::app::strip_orchestrator_prompts(&self.reasoning_text).into_owned();
        self.draw_dirty = true;
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
        self.streaming_display.clear();
        self.reasoning_text.clear();
        self.reasoning_active = false;
        self.reasoning_display.clear();
    }

    pub fn has_streaming(&self) -> bool {
        self.streaming_active
    }

    pub fn show_toast(&mut self, message: impl Into<String>, level: ToastLevel) {
        if self.is_processing() {
            return;
        }
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
            if let Some(metrics) = self.current_streaming_metrics() {
                self.last_turn_metrics = Some(metrics);
            }
            let text = std::mem::take(&mut self.streaming_text);
            let clean = crate::app::strip_orchestrator_prompts(&text);
            self.streaming_display.clear();
            if !clean.trim().is_empty() {
                self.lines
                    .push(RenderLine::AssistantText(clean.into_owned()));
                self.content_version += 1;
            }
            self.streaming_revealed_len = 0;
            self.streaming_active = false;
            self.streaming_start_time = None;
        }
    }

    /// Commit reasoning state without drawing.  Public so callers that
    /// batch multiple mutations (e.g. commit reasoning + push streaming chunk)
    /// can avoid redundant intermediate draws.
    pub fn commit_reasoning_inner(&mut self) {
        if self.reasoning_active {
            let text = std::mem::take(&mut self.reasoning_text);
            let clean = crate::app::strip_orchestrator_prompts(&text);
            self.reasoning_display.clear();
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
        if !self.mouse_capture_disabled {
            let _ = crossterm::execute!(
                self.terminal.backend_mut(),
                crossterm::event::DisableMouseCapture
            );
        }
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
        if !self.mouse_capture_disabled {
            let _ = crossterm::execute!(
                self.terminal.backend_mut(),
                crossterm::event::EnableMouseCapture
            );
        }
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
        self.toast = None; // Dismiss any active toast immediately when processing starts
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

    /// Returns `true` if CADE is actively processing or working on a task.
    ///
    /// This includes LLM reasoning/inference (`thinking`), streaming output (`streaming_active`),
    /// or active background subagent tasks.
    pub fn is_processing(&self) -> bool {
        self.thinking.is_some()
            || self.streaming_active
            || self
                .subagent_trackers
                .iter()
                .any(|t| matches!(t.status, crate::subagent_tracker::SubagentStatus::Running))
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
        modifiers: crossterm::event::KeyModifiers,
    ) -> bool {
        use crossterm::event::KeyCode;
        use crossterm::event::KeyModifiers;

        // Block all keyboard scrolling while an active drag-selection is in-progress (ADR 9)
        if self.selection_active {
            return false;
        }

        match code {
            // Alt+K or Alt+Shift+K — scroll up 10 lines
            KeyCode::Char('k') | KeyCode::Char('K') if modifiers.contains(KeyModifiers::ALT) => {
                self.follow = false;
                self.scroll_target = self.scroll_target.saturating_add(10);
                self.draw_dirty = true;
                true
            }
            // Alt+J, Alt+Shift+J, End, or Ctrl+Alt+G — snap to bottom ("Follow" mode)
            KeyCode::Char('j') | KeyCode::Char('J') if modifiers.contains(KeyModifiers::ALT) => {
                if self.scroll > 0 || self.scroll_target > 0 {
                    self.show_toast("Jumped to bottom", ToastLevel::Info);
                }
                self.scroll_target = 0;
                self.scroll = 0;
                self.follow = true;
                self.pending_lines = 0;
                self.draw_dirty = true;
                true
            }
            KeyCode::End => {
                if self.scroll > 0 || self.scroll_target > 0 {
                    self.show_toast("Jumped to bottom", ToastLevel::Info);
                }
                self.scroll_target = 0;
                self.scroll = 0;
                self.follow = true;
                self.pending_lines = 0;
                self.draw_dirty = true;
                true
            }
            KeyCode::Char('g') | KeyCode::Char('G')
                if modifiers.contains(KeyModifiers::CONTROL | KeyModifiers::ALT) =>
            {
                if self.scroll > 0 || self.scroll_target > 0 {
                    self.show_toast("Jumped to bottom", ToastLevel::Info);
                }
                self.scroll_target = 0;
                self.scroll = 0;
                self.follow = true;
                self.pending_lines = 0;
                self.draw_dirty = true;
                true
            }
            // Home or Ctrl+G — jump to top of transcript (first message)
            KeyCode::Home => {
                self.follow = false;
                let vh = crossterm::terminal::size()
                    .map(|(_, h)| h.saturating_sub(super::FIXED_ROWS + super::MAX_INPUT_ROWS))
                    .unwrap_or(20) as usize;
                let prepared = self.build_prepared_entries();
                let total_visual: usize = prepared.iter().map(|p| p.rows as usize).sum();
                let max_skip = total_visual.saturating_sub(vh);
                self.scroll_target = max_skip;
                self.scroll = max_skip;
                self.draw_dirty = true;
                true
            }
            KeyCode::Char('g') | KeyCode::Char('G')
                if modifiers.contains(KeyModifiers::CONTROL)
                    && !modifiers.contains(KeyModifiers::ALT) =>
            {
                self.follow = false;
                let vh = crossterm::terminal::size()
                    .map(|(_, h)| h.saturating_sub(super::FIXED_ROWS + super::MAX_INPUT_ROWS))
                    .unwrap_or(20) as usize;
                let prepared = self.build_prepared_entries();
                let total_visual: usize = prepared.iter().map(|p| p.rows as usize).sum();
                let max_skip = total_visual.saturating_sub(vh);
                self.scroll_target = max_skip;
                self.scroll = max_skip;
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
            // Half-page up: Ctrl+Alt+U
            KeyCode::Char('u') | KeyCode::Char('U')
                if modifiers.contains(KeyModifiers::CONTROL | KeyModifiers::ALT) =>
            {
                self.follow = false;
                let vh = crossterm::terminal::size()
                    .map(|(_, h)| h.saturating_sub(super::FIXED_ROWS + super::MAX_INPUT_ROWS))
                    .unwrap_or(20);
                self.scroll_target = crate::app::input::scroll_half_page_up(self.scroll_target, vh);
                self.draw_dirty = true;
                true
            }
            // Half-page down: Ctrl+Alt+D
            KeyCode::Char('d') | KeyCode::Char('D')
                if modifiers.contains(KeyModifiers::CONTROL | KeyModifiers::ALT) =>
            {
                let vh = crossterm::terminal::size()
                    .map(|(_, h)| h.saturating_sub(super::FIXED_ROWS + super::MAX_INPUT_ROWS))
                    .unwrap_or(20);
                let (new_target, should_follow) =
                    crate::app::input::scroll_half_page_down(self.scroll_target, vh);
                self.scroll_target = new_target;
                if should_follow {
                    self.follow = true;
                    self.pending_lines = 0;
                }
                self.draw_dirty = true;
                true
            }
            // Shift+Up or Ctrl+Up — scroll up 3 lines
            KeyCode::Up
                if modifiers.contains(KeyModifiers::SHIFT)
                    || modifiers.contains(KeyModifiers::CONTROL) =>
            {
                self.follow = false;
                self.scroll_target = self.scroll_target.saturating_add(3);
                self.draw_dirty = true;
                true
            }
            // Shift+Down or Ctrl+Down — scroll down 3 lines
            KeyCode::Down
                if modifiers.contains(KeyModifiers::SHIFT)
                    || modifiers.contains(KeyModifiers::CONTROL) =>
            {
                self.scroll_target = self.scroll_target.saturating_sub(3);
                if self.scroll_target == 0 {
                    self.follow = true;
                    self.pending_lines = 0;
                }
                self.draw_dirty = true;
                true
            }
            // Ctrl+u — scroll up 10 lines
            KeyCode::Char('u')
                if modifiers.contains(KeyModifiers::CONTROL)
                    && !modifiers.contains(KeyModifiers::ALT) =>
            {
                self.follow = false;
                self.scroll_target = self.scroll_target.saturating_add(10);
                self.draw_dirty = true;
                true
            }
            _ => false,
        }
    }

    /// Handle a mouse scroll event.  Returns `true` if the event was consumed.
    pub fn handle_scroll_mouse(&mut self, kind: crossterm::event::MouseEventKind) -> bool {
        use crossterm::event::MouseEventKind;

        // Block all mouse scrolling while an active drag-selection is in-progress (ADR 9)
        if self.selection_active {
            return false;
        }

        let vh = crossterm::terminal::size()
            .map(|(_, h)| h.saturating_sub(super::FIXED_ROWS + super::MAX_INPUT_ROWS))
            .unwrap_or(20) as usize;
        let max_buffer = (vh * 4).max(100);

        // Calculate velocity acceleration (Section F)
        let now = std::time::Instant::now();
        let is_rapid = self
            .last_scroll_at
            .map(|t| t.elapsed() < std::time::Duration::from_millis(180))
            .unwrap_or(false);
        if is_rapid {
            self.scroll_streak = (self.scroll_streak + 1).min(10);
        } else {
            self.scroll_streak = 0;
        }
        self.last_scroll_at = Some(now);

        let delta = crate::app::input::compute_accelerated_scroll(
            self.tui_settings.scrolling.scroll_speed,
            self.tui_settings.scrolling.scroll_acceleration,
            self.scroll_streak,
        );

        match kind {
            MouseEventKind::ScrollUp => {
                self.follow = false;

                let diff = self.scroll_target.saturating_sub(self.scroll);
                if diff < max_buffer {
                    let increment = if diff < max_buffer / 2 {
                        delta
                    } else {
                        (delta / 2).max(1)
                    };
                    self.scroll_target = self.scroll_target.saturating_add(increment);

                    if self.scroll_target.saturating_sub(self.scroll) > max_buffer {
                        self.scroll_target = self.scroll.saturating_add(max_buffer);
                    }
                }

                self.draw_dirty = true;
                true
            }
            MouseEventKind::ScrollDown => {
                let diff = self.scroll.saturating_sub(self.scroll_target);
                if diff < max_buffer {
                    let increment = if diff < max_buffer / 2 {
                        delta
                    } else {
                        (delta / 2).max(1)
                    };
                    self.scroll_target = self.scroll_target.saturating_sub(increment);

                    if self.scroll.saturating_sub(self.scroll_target) > max_buffer {
                        self.scroll_target = self.scroll.saturating_sub(max_buffer);
                    }
                }

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
                    | TimelineItemKind::Assistant
                    | TimelineItemKind::StreamingAssistant
                    | TimelineItemKind::ToolCall
                    | TimelineItemKind::ContextBar
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
