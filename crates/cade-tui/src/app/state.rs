use super::*;

impl TuiApp {
    /// Commit any in-progress streaming, push a line, and redraw.
    pub fn push(&mut self, line: RenderLine) -> Result<()> {
        self.commit_streaming_inner();
        self.commit_reasoning_inner();
        let is_tool_result = matches!(line, RenderLine::ToolResult { .. });
        self.lines.push(line);

        if is_tool_result {
            // Scroll to show the associated ToolCall header at the top of the
            // visible area.  When diff-preview lines sit between the ToolCall
            // and ToolResult (e.g. for file edits), a plain scroll=0 would
            // show only the bottom of the diff and clip the ToolCall off-screen.
            // rows_from_last_tool_call() counts visual rows from the most recent
            // ToolCall to the end of lines so the whole tool group scrolls into
            // view as a unit.
            self.scroll = self.rows_from_last_tool_call();
        } else {
            self.scroll = 0;
        }
        if self.follow {
            self.scroll = 0;
        }
        self.pending_lines = 0;
        let scroll_before = self.scroll;
        self.draw()?;
        // V-05: If V-04 clamped self.scroll during draw (rows_from_last_tool_call
        // overshot max_skip in short conversations), redraw immediately so the
        // first visible frame always uses the correct scroll value.
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
    }

    /// Append a streaming chunk and redraw (throttled — max ~60 FPS).
    pub fn push_streaming_chunk(&mut self, text: &str) -> Result<()> {
        self.commit_reasoning_inner();
        if !self.streaming_active {
            // First chunk of a new agent response — always snap to bottom so the
            // analysis is immediately visible.  push(ToolResult) may have scrolled
            // up to show the ToolCall header; that view is correct while the tool
            // was running, but as soon as the agent starts responding the viewport
            // must follow the output.
            if self.follow {
                self.scroll = 0;
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
        });
        self.draw_dirty = true;
    }

    /// Mark a step done/undone from an explicit `UpdatePlan` tool call.
    /// step_id is 1-based.  Returns false if the id is out of range.
    pub fn update_plan_step(&mut self, step_id: usize, done: bool) -> bool {
        if let Some(plan) = &mut self.active_plan
            && let Some(step) = plan.steps.iter_mut().find(|s| s.id == step_id)
        {
            step.is_done = done;
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
        // Snap to bottom when streaming commits — the completed response must
        // be visible.  Only mid-stream chunks (push_streaming_chunk) preserve
        // the user's scroll position; once the response is fully committed here
        // we always show it.
        if self.follow {
            self.scroll = 0;
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
        self.reasoning_text.clear();
        self.reasoning_active = false;
    }

    pub fn has_streaming(&self) -> bool {
        self.streaming_active
    }

    /// Toggle OS text-selection copy mode on/off.
    /// When ON: mouse capture is disabled so the terminal lets the user select text.
    /// When OFF: mouse capture is restored so scroll wheel works normally.
    pub fn toggle_copy_mode(&mut self) {
        self.copy_mode = !self.copy_mode;
        if self.copy_mode {
            let _ = crossterm::execute!(std::io::stdout(), DisableMouseCapture);
            self.show_toast("Copy mode enabled", ToastLevel::Info);
        } else {
            let _ = crossterm::execute!(std::io::stdout(), EnableMouseCapture);
            self.show_toast("Copy mode disabled", ToastLevel::Info);
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
        self.scroll = 0;
        self.follow = true;
        self.draw()
    }
    pub(crate) fn commit_streaming_inner(&mut self) {
        if self.streaming_active {
            let text = std::mem::take(&mut self.streaming_text);
            let clean = crate::app::strip_orchestrator_prompts(&text);
            if !clean.trim().is_empty() {
                self.lines.push(RenderLine::AssistantText(clean.into_owned()));
            }
            self.streaming_active = false;
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
        self.lines.len() - 1
    }

    /// Append one output line to the `LiveOutput` at `idx` and redraw
    /// (throttled — max ~60 FPS).  No-op if `idx` is not a `LiveOutput`.
    pub fn append_live_output_line(&mut self, idx: usize, line: String) -> Result<()> {
        if let Some(RenderLine::LiveOutput { lines, .. }) = self.lines.get_mut(idx) {
            lines.push(line);
        }
        if self.follow {
            self.scroll = 0;
        }
        self.draw_throttled()
    }

    /// Mark the `LiveOutput` at `idx` as finished (subprocess has exited).
    /// Redraws so the final state is shown before the caller returns.
    pub fn finish_live_output(&mut self, idx: usize) -> Result<()> {
        if let Some(RenderLine::LiveOutput { done, .. }) = self.lines.get_mut(idx) {
            *done = true;
        }
        if self.follow {
            self.scroll = 0;
        }
        self.draw()
    }

    /// Temporarily suspends the TUI, runs the provided closure, and then restores it.
    pub fn suspend_for<F>(&mut self, f: F) -> Result<()>
    where
        F: FnOnce(),
    {
        crossterm::terminal::disable_raw_mode().map_err(|e| crate::Error::Custom(e.to_string()))?;
        crossterm::execute!(
            self.terminal.backend_mut(),
            crossterm::terminal::LeaveAlternateScreen
        )
        .map_err(|e| crate::Error::Custom(e.to_string()))?;

        f();

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

    pub fn update_model(&mut self, model: String) {
        self.model = model;
    }

    pub fn update_mode(&mut self, mode: PermissionMode) {
        self.mode = mode;
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
        self.scroll = 0; // snap to bottom at the start of every agent turn
        let arc = Arc::new(Mutex::new(text.into()));
        self.thinking = Some(ThinkingState {
            text: arc.clone(),
            started: Instant::now(),
        });
        arc
    }

    /// Update the thinking text from the animation/assessing timer.
    pub fn update_thinking_text(&mut self, text: String) {
        if let Some(ts) = &self.thinking
        {
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
        secs
    }

    pub fn open_theme_picker(
        &mut self,
        themes: Vec<cade_core::resources::themes::Theme>,
        original_theme: crate::colors::ThemeColors,
    ) {
        let tp = ThemePickerState {
            query: String::new(),
            filtered_indices: (0..themes.len()).collect(),
            themes,
            cursor: 0,
            original_theme,
        };
        self.theme_picker = Some(tp);
        self.apply_theme_from_picker();
    }
    pub(crate) fn apply_theme_from_picker(&mut self) {
        if let Some(tp) = &self.theme_picker
            && !tp.filtered_indices.is_empty()
        {
            let idx = tp.filtered_indices[tp.cursor];
            let t = &tp.themes[idx];
            let colors = if t.name == "dark" {
                crate::colors::ThemeColors::dark()
            } else if t.name == "light" {
                crate::colors::ThemeColors::light()
            } else {
                crate::colors::ThemeColors::from_theme(t)
            };
            self.apply_theme(colors);
        }
    }
    pub(crate) fn update_theme_picker_filter(&mut self) {
        if let Some(tp) = &mut self.theme_picker {
            tp.cursor = 0;
            tp.filtered_indices = tp
                .themes
                .iter()
                .enumerate()
                .filter(|(_, t)| {
                    tp.query.is_empty() || t.name.to_lowercase().contains(&tp.query.to_lowercase())
                })
                .map(|(i, _)| i)
                .collect();
        }
        self.apply_theme_from_picker();
    }

    pub fn set_context_pct(&mut self, pct: u8) {
        self.context_pct = Some(pct.min(99));
    }

}
