//! Agent-list and metrics state for [`super::SessionState`].

use super::*;

impl SessionState {
    /// Feed a successful agent-list result.
    ///
    /// Only transitions from `HealthOk` → `Connected`.
    /// No-op in any other state.
    pub fn on_agents(&mut self, agents: Vec<AgentInfo>) {
        if let Self::HealthOk {
            server_url,
            token,
            health,
            ..
        } = self
        {
            *self = Self::Connected {
                server_url: std::mem::take(server_url),
                token: std::mem::take(token),
                health: health.clone(),
                agents,
                selected_agent: None,
                messages: Vec::new(),
                input_buffer: String::new(),
                streaming: false,
                auto_scroll: true,
                error_toast: None,
                conversation_id: None,
                last_usage: None,
                last_finish_reason: None,
                conversations: Vec::new(),
                selected_conversation: None,
                has_more_messages: false,
                palette_open: false,
                palette_input: String::new(),
                palette_selection: 0,
                menu_open: false,
                menu_input: String::new(),
                menu_selection: 0,
                memory_open: false,
                memory_blocks: Vec::new(),
                memory_selection: 0,
                memory_edit_buffer: String::new(),
                memory_loading: false,
                memory_saving: false,
                memory_error: None,
                memory_save_notice: None,

                checkpoints_open: false,
                checkpoints: Vec::new(),
                checkpoints_loading: false,
                checkpoints_busy: false,
                checkpoints_error: None,
                checkpoints_notice: None,

                artifacts_open: false,
                artifacts: Vec::new(),
                artifact_selection: None,
                artifact_detail: None,
                artifacts_loading: false,
                artifacts_busy: false,
                artifacts_error: None,

                tools_open: false,
                tools: Vec::new(),
                tools_loading: false,
                tools_error: None,

                active_question: None,
                question_cursor: 0,
                question_checked: Vec::new(),

                agent_metrics: None,
                total_input_tokens: 0,
                total_output_tokens: 0,

                context_open: false,
                context_stats: None,
                context_loading: false,
                context_error: None,

                agents_open: false,
                stats_open: false,

                mcp_open: false,
                mcp_servers: Vec::new(),
                mcp_loading: false,
                mcp_error: None,
                theme_update: None,

                model_picker_open: false,
                model_picker_models: Vec::new(),
                model_picker_custom_providers: Vec::new(),
                model_picker_query: String::new(),
                model_picker_selection: 0,
                model_picker_loading: false,
                model_picker_error: None,

                active_plan: None,
                live_outputs: Vec::new(),

                context_breakdown: None,
                context_breakdown_loading: false,

                providers_open: false,
                providers: Vec::new(),
                providers_loading: false,
                permissions_open: false,
                current_permission_mode: "default".to_string(),
                theme_picker_open: false,
                available_themes: vec![
                    "tokyo-night".into(),
                    "catppuccin-mocha".into(),
                    "catppuccin-latte".into(),
                    "dark".into(),
                    "light".into(),
                ],
                current_theme_name: "tokyo-night".to_string(),
                hooks_open: false,
                hooks: Vec::new(),
                hooks_loading: false,
                toolset_open: false,
                current_toolset: "default".to_string(),
                pricing_open: false,
                pricing_info: String::new(),
                backend_open: false,
                current_backend: "local".to_string(),
                reasoning_open: false,
                current_reasoning_effort: "none".to_string(),
                skills_overlay_open: false,
                all_skills_list: Vec::new(),
                loaded_skill_ids: Vec::new(),
                skills_loading: false,
                skills_filter: String::new(),
                subagent_cards: Vec::new(),
            };
        }
    }

    /// Select an agent by index.  Clears messages so the UI can show a
    /// loading state while the fetch is in flight.
    ///
    /// Returns `true` if the selection changed (caller should spawn a
    /// message fetch), `false` if it was a no-op (already selected, or
    /// index out of bounds, or not in `Connected` state).
    pub fn on_select_agent(&mut self, idx: usize) -> bool {
        if let Self::Connected {
            agents,
            selected_agent,
            messages,
            conversation_id,
            conversations,
            selected_conversation,
            ..
        } = self
        {
            if idx >= agents.len() {
                return false;
            }
            if *selected_agent == Some(idx) {
                return false;
            }
            *selected_agent = Some(idx);
            messages.clear();
            *conversation_id = None;
            conversations.clear();
            *selected_conversation = None;
            true
        } else {
            false
        }
    }

    /// The currently selected agent's ID, if any.
    pub fn selected_agent_id(&self) -> Option<&str> {
        if let Self::Connected {
            agents,
            selected_agent: Some(idx),
            ..
        } = self
        {
            agents.get(*idx).map(|a| a.id.as_str())
        } else {
            None
        }
    }

    /// Read-only slice of the agents list.
    pub fn agents(&self) -> &[AgentInfo] {
        if let Self::Connected { agents, .. } = self {
            agents
        } else {
            &[]
        }
    }

    /// Replace the full agent list (e.g. after a PATCH /v1/agents/:id
    /// so the sidebar reflects a model change).  Preserves the current
    /// `selected_agent` index if it still points to an existing row.
    pub fn refresh_agents(&mut self, new_agents: Vec<AgentInfo>) {
        if let Self::Connected {
            agents,
            selected_agent,
            ..
        } = self
        {
            // Prefer to keep selection by agent id, not by index — the
            // list order could theoretically change.
            let sel_id = selected_agent
                .and_then(|i| agents.get(i).map(|a| a.id.clone()));
            *agents = new_agents;
            *selected_agent = sel_id
                .and_then(|id| agents.iter().position(|a| a.id == id));
        }
    }

    /// Store metrics fetched from `GET /v1/agents/:id/metrics`.
    pub fn on_metrics_loaded(&mut self, m: crate::api::AgentMetrics) {
        if let Self::Connected { agent_metrics, .. } = self {
            *agent_metrics = Some(m);
        }
    }

    /// Read-only access to the last-fetched agent metrics.
    pub fn agent_metrics(&self) -> Option<&crate::api::AgentMetrics> {
        if let Self::Connected { agent_metrics, .. } = self {
            agent_metrics.as_ref()
        } else {
            None
        }
    }

    /// Accumulated input + output tokens for this session.
    /// Returns `(total_in, total_out)`.
    pub fn total_token_usage(&self) -> (u64, u64) {
        if let Self::Connected {
            total_input_tokens,
            total_output_tokens,
            ..
        } = self
        {
            (*total_input_tokens, *total_output_tokens)
        } else {
            (0, 0)
        }
    }

}
