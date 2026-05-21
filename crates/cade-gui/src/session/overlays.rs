//! Miscellaneous overlays (context, MCP, model picker, palette, menu, question, settings) for [`super::SessionState`].

use super::*;

impl SessionState {
    /// Present a question from an `ask_user_question` tool call.
    ///
    /// Initialises cursor to 0 and checked-vec to all-false.  If a
    /// question is already active it is replaced (the server serialises
    /// tool calls so this shouldn't happen in practice).
    pub fn set_active_question(&mut self, q: crate::api::Question) {
        if let Self::Connected(session) = self {
            let crate::session::ConnectedSession { 
            active_question,
            question_cursor,
            question_checked,
            ..
         } = &mut **session;
            let n = q.options.len();
            *question_cursor = 0;
            *question_checked = vec![false; n];
            *active_question = Some(q);
        }
    }

    /// Clear the active question (after the user answers or cancels).
    pub fn clear_active_question(&mut self) {
        if let Self::Connected(session) = self {
            let crate::session::ConnectedSession { 
            active_question,
            question_cursor,
            question_checked,
            ..
         } = &mut **session;
            *active_question = None;
            *question_cursor = 0;
            question_checked.clear();
        }
    }

    /// Whether a question is currently awaiting an answer.
    pub fn has_active_question(&self) -> bool {
        matches!(self, Self::Connected(session) if session.active_question.is_some())
    }

    /// Immutable reference to the active question, if any.
    pub fn active_question(&self) -> Option<&crate::api::Question> {
        if let Self::Connected(session) = self {
            session.active_question.as_ref()
        } else {
            None
        }
    }

    /// Move the question cursor up or down (wraps).  `delta` is -1 or +1.
    pub fn move_question_cursor(&mut self, delta: i32) {
        if let Self::Connected(session) = self {
            if let Some(q) = &session.active_question {
                let n = q.options.len();
                if n == 0 {
                    return;
                }
                let cur = session.question_cursor as i32;
                session.question_cursor = ((cur + delta).rem_euclid(n as i32)) as usize;
            }
        }
    }

    /// Toggle the checked state for the option at the current cursor
    /// (multi-select mode only).
    pub fn toggle_question_checked(&mut self) {
        if let Self::Connected(session) = self {
            if let Some(q) = &session.active_question {
                if q.multi_select {
                    let idx = session.question_cursor;
                    if let Some(v) = session.question_checked.get_mut(idx) {
                        *v = !*v;
                    }
                }
            }
        }
    }

    /// Build the answer string to send back to the server.
    ///
    /// Single-select: the label of the selected option.
    /// Multi-select: comma-joined labels of all checked options.
    /// Returns `None` when no question is active or nothing is selected.
    pub fn commit_question_answer(&mut self) -> Option<String> {
        if let Self::Connected(session) = self {
            let q = session.active_question.as_ref()?;
            let answer = if q.multi_select {
                let labels: Vec<&str> = session.question_checked
                    .iter()
                    .enumerate()
                    .filter(|(_, c)| **c)
                    .filter_map(|(i, _)| q.options.get(i).map(|o| o.label.as_str()))
                    .collect();
                if labels.is_empty() {
                    return None;
                }
                labels.join(", ")
            } else {
                q.options.get(session.question_cursor).map(|o| o.label.clone())?
            };
            Some(answer)
        } else {
            None
        }
    }

    /// Open the context-stats overlay.  Caller spawns the fetch.
    pub fn open_context_overlay(&mut self) {
        if let Self::Connected(session) = self {
            let crate::session::ConnectedSession { 
            context_open,
            context_loading,
            context_error,
            context_breakdown_loading,
            ..
         } = &mut **session;
            *context_open = true;
            *context_loading = true;
            *context_error = None;
            *context_breakdown_loading = true;
        }
    }

    /// Close the context-stats overlay.
    pub fn close_context_overlay(&mut self) {
        if let Self::Connected(session) = self {
            let crate::session::ConnectedSession { 
            context_open,
            context_error,
            ..
         } = &mut **session;
            *context_open = false;
            *context_error = None;
        }
    }

    /// Whether the context overlay is open.
    pub fn is_context_open(&self) -> bool {
        matches!(self, Self::Connected(session) if matches!(&**session, crate::session::ConnectedSession { 
                context_open: true,
                ..
             }))
    }

    /// Feed a successful context-stats response.
    pub fn on_context_loaded(&mut self, stats: crate::api::ContextStats) {
        if let Self::Connected(session) = self {
            let crate::session::ConnectedSession { 
            context_stats,
            context_loading,
            context_error,
            ..
         } = &mut **session;
            *context_loading = false;
            *context_error = None;
            *context_stats = Some(stats);
        }
    }

    /// Feed an error from the context fetch.
    pub fn on_context_error(&mut self, err: &str) {
        if let Self::Connected(session) = self {
            let crate::session::ConnectedSession { 
            context_loading,
            context_error,
            ..
         } = &mut **session;
            *context_loading = false;
            *context_error = Some(err.to_string());
        }
    }

    /// Read-only access to last-fetched context stats.
    pub fn context_stats(&self) -> Option<&crate::api::ContextStats> {
        if let Self::Connected(session) = self {
            let crate::session::ConnectedSession {  context_stats, ..  } = &**session;
            context_stats.as_ref()
        } else {
            None
        }
    }

    /// Open the agents list overlay.
    pub fn open_agents_overlay(&mut self) {
        if let Self::Connected(session) = self {
            let crate::session::ConnectedSession {  agents_open, ..  } = &mut **session;
            *agents_open = true;
        }
    }

    /// Close the agents list overlay.
    pub fn close_agents_overlay(&mut self) {
        if let Self::Connected(session) = self {
            let crate::session::ConnectedSession {  agents_open, ..  } = &mut **session;
            *agents_open = false;
        }
    }

    /// Whether the agents overlay is open.
    pub fn is_agents_open(&self) -> bool {
        matches!(self, Self::Connected(session) if matches!(&**session, crate::session::ConnectedSession { 
                agents_open: true,
                ..
             }))
    }

    /// Open the stats overlay.
    pub fn open_stats_overlay(&mut self) {
        if let Self::Connected(session) = self {
            let crate::session::ConnectedSession {  stats_open, ..  } = &mut **session;
            *stats_open = true;
        }
    }

    /// Close the stats overlay.
    pub fn close_stats_overlay(&mut self) {
        if let Self::Connected(session) = self {
            let crate::session::ConnectedSession {  stats_open, ..  } = &mut **session;
            *stats_open = false;
        }
    }

    /// Whether the stats overlay is open.
    pub fn is_stats_open(&self) -> bool {
        matches!(self, Self::Connected(session) if matches!(&**session, crate::session::ConnectedSession { 
                stats_open: true,
                ..
             }))
    }

    /// Open the MCP servers overlay and mark loading state.
    pub fn open_mcp_overlay(&mut self) {
        if let Self::Connected(session) = self {
            let crate::session::ConnectedSession { 
            mcp_open,
            mcp_loading,
            mcp_error,
            ..
         } = &mut **session;
            *mcp_open = true;
            *mcp_loading = true;
            *mcp_error = None;
        }
    }

    /// Close the MCP servers overlay and clear any error.
    pub fn close_mcp_overlay(&mut self) {
        if let Self::Connected(session) = self {
            let crate::session::ConnectedSession { 
            mcp_open,
            mcp_error,
            ..
         } = &mut **session;
            *mcp_open = false;
            *mcp_error = None;
        }
    }

    /// Returns `true` when the MCP overlay is visible.
    pub fn is_mcp_open(&self) -> bool {
        matches!(self, Self::Connected(session) if matches!(&**session, crate::session::ConnectedSession {  mcp_open: true, ..  }))
    }

    /// Store freshly-fetched MCP server list and clear loading state.
    pub fn on_mcp_loaded(&mut self, servers: Vec<crate::api::McpServerInfo>) {
        if let Self::Connected(session) = self {
            let crate::session::ConnectedSession { 
            mcp_servers,
            mcp_loading,
            mcp_error,
            ..
         } = &mut **session;
            *mcp_servers = servers;
            *mcp_loading = false;
            *mcp_error = None;
        }
    }

    /// Record a fetch error and clear the loading flag.
    pub fn on_mcp_error(&mut self, err: String) {
        if let Self::Connected(session) = self {
            let crate::session::ConnectedSession { 
            mcp_loading,
            mcp_error,
            ..
         } = &mut **session;
            *mcp_loading = false;
            *mcp_error = Some(err);
        }
    }

    /// Open the model picker overlay and mark loading state.
    pub fn open_model_picker(&mut self) {
        if let Self::Connected(session) = self {
            let crate::session::ConnectedSession { 
            model_picker_open,
            model_picker_query,
            model_picker_selection,
            model_picker_loading,
            model_picker_error,
            ..
         } = &mut **session;
            *model_picker_open = true;
            model_picker_query.clear();
            *model_picker_selection = 0;
            *model_picker_loading = true;
            *model_picker_error = None;
        }
    }

    /// Close the model picker overlay.
    pub fn close_model_picker(&mut self) {
        if let Self::Connected(session) = self {
            let crate::session::ConnectedSession { 
            model_picker_open, ..
         } = &mut **session;
            *model_picker_open = false;
        }
    }

    /// Whether the model picker overlay is open.
    pub fn is_model_picker_open(&self) -> bool {
        matches!(self, Self::Connected(session) if matches!(&**session, crate::session::ConnectedSession { 
                model_picker_open: true,
                ..
             }))
    }

    /// Called when models are successfully fetched.
    pub fn on_models_loaded(
        &mut self,
        models: Vec<crate::api::ModelInfo>,
        custom_providers: Vec<String>,
    ) {
        if let Self::Connected(session) = self {
            let crate::session::ConnectedSession { 
            model_picker_models,
            model_picker_custom_providers,
            model_picker_loading,
            model_picker_error,
            ..
         } = &mut **session;
            *model_picker_models = models;
            *model_picker_custom_providers = custom_providers;
            *model_picker_loading = false;
            *model_picker_error = None;
        }
    }

    /// Called when model fetch fails.
    pub fn on_models_error(&mut self, err: String) {
        if let Self::Connected(session) = self {
            let crate::session::ConnectedSession { 
            model_picker_loading,
            model_picker_error,
            ..
         } = &mut **session;
            *model_picker_loading = false;
            *model_picker_error = Some(err);
        }
    }

    /// Set the model picker search query and reset selection to 0.
    pub fn set_model_picker_query(&mut self, q: String) {
        if let Self::Connected(session) = self {
            let crate::session::ConnectedSession { 
            model_picker_query,
            model_picker_selection,
            ..
         } = &mut **session;
            *model_picker_query = q;
            *model_picker_selection = 0;
        }
    }

    /// Set the model picker selection index.
    pub fn set_model_picker_selection(&mut self, idx: usize) {
        if let Self::Connected(session) = self {
            let crate::session::ConnectedSession { 
            model_picker_selection,
            ..
         } = &mut **session;
            *model_picker_selection = idx;
        }
    }

    /// Get the currently selected model ID from the filtered list.
    ///
    /// Returns `None` if no models or selection is out of range.
    pub fn selected_model_id(&self) -> Option<String> {
        if let Self::Connected(session) = self {
            let crate::session::ConnectedSession { 
            model_picker_models,
            model_picker_query,
            model_picker_selection,
            ..
         } = &**session;
            let filtered = filter_models(model_picker_models, model_picker_query);
            filtered.get(*model_picker_selection).map(|m| m.id.clone())
        } else {
            None
        }
    }

    /// Open the slash-command palette overlay.
    ///
    /// Resets query + selection.  Optional `initial_input` pre-fills the
    /// filter text (used when the user typed `/foo` in the input bar).
    pub fn open_palette(&mut self, initial_input: &str) {
        if let Self::Connected(session) = self {
            let crate::session::ConnectedSession { 
            palette_open,
            palette_input,
            palette_selection,
            ..
         } = &mut **session;
            *palette_open = true;
            *palette_input = initial_input.to_string();
            *palette_selection = 0;
        }
    }

    /// Close the palette.  Clears query + selection.
    pub fn close_palette(&mut self) {
        if let Self::Connected(session) = self {
            let crate::session::ConnectedSession { 
            palette_open,
            palette_input,
            palette_selection,
            ..
         } = &mut **session;
            *palette_open = false;
            palette_input.clear();
            *palette_selection = 0;
        }
    }

    /// Replace the palette filter input. Resets selection to 0 so the top
    /// result stays highlighted as the user types.
    pub fn set_palette_input(&mut self, query: &str) {
        if let Self::Connected(session) = self {
            let crate::session::ConnectedSession { 
            palette_input,
            palette_selection,
            ..
         } = &mut **session;
            *palette_input = query.to_string();
            *palette_selection = 0;
        }
    }

    /// Move the palette selection up (-1) or down (+1), clamped to the
    /// number of filtered entries for the current query.
    pub fn move_palette_selection(&mut self, delta: i32) {
        if let Self::Connected(session) = self {
            let crate::session::ConnectedSession { 
            palette_input,
            palette_selection,
            ..
         } = &mut **session;
            let count = crate::palette::fuzzy_filter(palette_input).len();
            if count == 0 {
                *palette_selection = 0;
                return;
            }
            let max_idx = count - 1;
            let new_idx = (*palette_selection as i32) + delta;
            *palette_selection = new_idx.clamp(0, max_idx as i32) as usize;
        }
    }

    /// Whether the palette overlay is currently open.
    pub fn is_palette_open(&self) -> bool {
        matches!(self, Self::Connected(session) if session.palette_open)
    }

    /// Parse the currently-selected palette entry into a concrete
    /// [`crate::palette::PaletteCmd`].  Returns `None` if the palette is
    /// closed or there are no matching entries for the query.
    pub fn selected_palette_cmd(&self) -> Option<crate::palette::PaletteCmd> {
        if let Self::Connected(session) = self {
            if !session.palette_open { return None; }
            let filtered = crate::palette::fuzzy_filter(&session.palette_input);
            if filtered.is_empty() {
                return None;
            }
            let idx = session.palette_selection.min(filtered.len() - 1);
            Some(crate::palette::parse_palette_input(
                filtered[idx].def.trigger,
            ))
        } else {
            None
        }
    }

    /// Open the full-screen command menu overlay.
    pub fn open_menu(&mut self, initial_input: &str) {
        if let Self::Connected(session) = self {
            let crate::session::ConnectedSession { 
            menu_open,
            menu_input,
            menu_selection,
            ..
         } = &mut **session;
            *menu_open = true;
            *menu_input = initial_input.to_string();
            *menu_selection = 0;
        }
    }

    /// Close the command menu.
    pub fn close_menu(&mut self) {
        if let Self::Connected(session) = self {
            let crate::session::ConnectedSession { 
            menu_open,
            menu_input,
            menu_selection,
            ..
         } = &mut **session;
            *menu_open = false;
            menu_input.clear();
            *menu_selection = 0;
        }
    }

    /// Replace the menu filter input and reset selection.
    pub fn set_menu_input(&mut self, query: &str) {
        if let Self::Connected(session) = self {
            let crate::session::ConnectedSession { 
            menu_input,
            menu_selection,
            ..
         } = &mut **session;
            *menu_input = query.to_string();
            *menu_selection = 0;
        }
    }

    /// Move the menu selection up (-1) or down (+1).
    pub fn move_menu_selection(&mut self, delta: i32) {
        if let Self::Connected(session) = self {
            let crate::session::ConnectedSession { 
            menu_input,
            menu_selection,
            ..
         } = &mut **session;
            let count = crate::palette::fuzzy_filter(menu_input).len();
            if count == 0 {
                *menu_selection = 0;
                return;
            }
            let max_idx = count - 1;
            let new_idx = (*menu_selection as i32) + delta;
            *menu_selection = new_idx.clamp(0, max_idx as i32) as usize;
        }
    }

    /// Whether the menu overlay is currently open.
    pub fn is_menu_open(&self) -> bool {
        matches!(self, Self::Connected(session) if matches!(&**session, crate::session::ConnectedSession { 
                menu_open: true,
                ..
             }))
    }

    /// Parse the currently-selected menu entry.
    pub fn selected_menu_cmd(&self) -> Option<crate::palette::PaletteCmd> {
        if let Self::Connected(session) = self {
            if !session.menu_open { return None; }
            let filtered = crate::palette::fuzzy_filter(&session.menu_input);
            if filtered.is_empty() {
                return None;
            }
            let idx = session.menu_selection.min(filtered.len() - 1);
            Some(crate::palette::parse_palette_input(
                filtered[idx].def.trigger,
            ))
        } else {
            None
        }
    }
}
