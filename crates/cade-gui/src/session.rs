//! Post-login session state machine for the cade-gui WASM app.
//!
//! **Pure logic, no browser dependencies.**  After the user submits a
//! token via [`crate::login::LoginState`], the app transitions into
//! this machine which tracks the connection lifecycle:
//!
//! ```text
//!   LoginState::Submitted { key }
//!          │
//!          ▼
//!   SessionState::Connecting { server_url, token }
//!          │
//!     ┌────┴────┐
//!     ▼         ▼
//!  Connected  ConnectionFailed { error }
//!                    │
//!                    ▼ (on_retry)
//!              back to LoginState
//! ```
//!
//! The wasm render loop (`app.rs`) drives this machine by spawning
//! async tasks that call `http_wasm::{get_health, get_agents}` and
//! feeding the results back via `on_health` / `on_agents` / `on_error`.

use cade_api_types::{AgentInfo, ChatMessage, HealthInfo};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ToastLevel {
    Info,
    Success,
    Warning,
    Error,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Toast {
    pub message: String,
    pub level: ToastLevel,
}

// ── Plan panel types (mirrors cade-tui PlanState) ─────────────────────

/// A single step in the agent's plan checklist.
#[derive(Debug, Clone, PartialEq)]
pub struct PlanStep {
    pub id: usize,
    pub description: String,
    pub is_done: bool,
}

/// The full plan state — a list of steps with a visibility toggle.
#[derive(Debug, Clone, PartialEq)]
pub struct PlanState {
    pub steps: Vec<PlanStep>,
    pub is_visible: bool,
}

// ── Live output types (mirrors cade-tui LiveOutput) ───────────────────

/// A block of streaming output lines from a long-running tool execution.
#[derive(Debug, Clone, PartialEq)]
pub struct LiveOutputBlock {
    /// Tool call ID that produced this output.
    pub call_id: String,
    /// Tool name (e.g. "bash").
    pub tool_name: String,
    /// Accumulated output lines.
    pub lines: Vec<String>,
    /// Whether the tool has finished executing.
    pub done: bool,
    /// Maximum visible lines before scrolling (0 = show all).
    pub max_visible: usize,
}

/// Post-login session state.
///
/// Created from `LoginState::Submitted` — the token and server URL are
/// captured at construction and never mutated.
#[derive(Debug, Clone, PartialEq)]
#[allow(clippy::large_enum_variant)] // Connected is intentionally rich; boxing adds no value
pub enum SessionState {
    /// Token submitted, waiting for health + agent-list responses.
    Connecting {
        server_url: String,
        token: String,
    },
    /// Health check succeeded; waiting for agent list.
    HealthOk {
        server_url: String,
        token: String,
        health: HealthInfo,
    },
    /// Both health and agent list succeeded — session is live.
    Connected {
        server_url: String,
        token: String,
        health: HealthInfo,
        agents: Vec<AgentInfo>,
        /// Index into `agents` of the currently selected agent, if any.
        selected_agent: Option<usize>,
        /// Messages for the selected agent (empty until an agent is selected
        /// and the fetch completes).
        messages: Vec<ChatMessage>,
        /// The text the user is currently typing in the input bar.
        input_buffer: String,
        /// True while we are streaming an assistant response via SSE.
        streaming: bool,
        /// Whether the timeline should auto-scroll to the bottom.
        /// Set to `false` when the user scrolls up; restored to `true`
        /// when they click the ↓ button or a new message arrives.
        auto_scroll: bool,
        /// Transient error message displayed as a toast overlay.
        error_toast: Option<String>,
        /// Active conversation ID (set from SSE metadata, cleared on agent switch).
        conversation_id: Option<String>,
        /// Token usage from the last completed turn.
        last_usage: Option<(u64, u64, Option<String>)>,
        /// Finish reason from the last completed turn (e.g. "stop", "length").
        last_finish_reason: Option<String>,
        /// Conversations for the selected agent.
        conversations: Vec<crate::api::ConversationInfo>,
        /// Index into `conversations` of the currently selected conversation.
        selected_conversation: Option<usize>,
        /// Whether there are more messages to load (pagination).
        has_more_messages: bool,
        /// Whether the slash-command palette overlay is visible.
        palette_open: bool,
        /// Current text in the palette filter input.
        palette_input: String,
        /// Index of the highlighted entry in the filtered palette list.
        palette_selection: usize,
        /// Whether the full-screen command menu overlay is visible.
        menu_open: bool,
        /// Current text in the menu filter input.
        menu_input: String,
        /// Index of the highlighted entry in the filtered menu list.
        menu_selection: usize,
        /// Whether the memory-viewer overlay is visible.
        memory_open: bool,
        /// Memory blocks fetched from `GET /v1/agents/:id/memory`.
        memory_blocks: Vec<crate::api::MemoryBlock>,
        /// Index into `memory_blocks` of the currently-viewed block.
        memory_selection: usize,
        /// Editable buffer mirrored from the selected block — saved on
        /// "Save" click via `PUT /v1/agents/:id/memory/:label`.
        memory_edit_buffer: String,
        /// True while the GET request is in flight.
        memory_loading: bool,
        /// True while a PUT request is in flight.
        memory_saving: bool,
        /// Per-overlay error message (shown inside the memory window).
        memory_error: Option<String>,
        /// Transient success notice shown after a successful save
        /// (e.g. "Saved /project").  Cleared when the selection changes,
        /// the overlay closes, or another save starts.
        memory_save_notice: Option<String>,

        // ── Checkpoints overlay (M17) ────────────────────────────
        /// Whether the checkpoints overlay is visible.
        checkpoints_open: bool,
        /// Rows fetched from `GET /v1/agents/:id/checkpoints`.
        checkpoints: Vec<crate::api::CheckpointRow>,
        /// True while the GET request is in flight.
        checkpoints_loading: bool,
        /// True while a restore/delete/create request is in flight.
        checkpoints_busy: bool,
        /// Per-overlay error message.
        checkpoints_error: Option<String>,
        /// Transient success notice (e.g. "Restored cp-1234…").
        checkpoints_notice: Option<String>,

        // ── Artifacts overlay (M17) ──────────────────────────────
        /// Whether the artifacts overlay is visible.
        artifacts_open: bool,
        /// Summary rows fetched from `GET /v1/agents/:id/artifacts`.
        artifacts: Vec<crate::api::ArtifactInfo>,
        /// Index of the currently-selected row; `None` when nothing selected.
        artifact_selection: Option<usize>,
        /// Full detail for the selected artifact — lazy-loaded on click.
        /// `None` means not-yet-loaded; a loaded detail whose `id` differs
        /// from the selected row's `id` means stale and will be replaced.
        artifact_detail: Option<crate::api::ArtifactDetail>,
        /// True while the list GET is in flight.
        artifacts_loading: bool,
        /// True while a per-artifact detail fetch or delete is in flight.
        artifacts_busy: bool,
        /// Per-overlay error message.
        artifacts_error: Option<String>,

        // ── Tools overlay (M18 — MCP / skills) ──────────────────
        /// Whether the tools/MCP overlay is visible.
        tools_open: bool,
        /// Tools fetched from `GET /v1/agents/:id/tools`.
        tools: Vec<crate::api::AgentTool>,
        /// True while the GET request is in flight.
        tools_loading: bool,
        /// Per-overlay error message.
        tools_error: Option<String>,

        // ── Inline question widget (M18 — ask_user_question) ────
        /// The currently-active question received via `ask_user_question`
        /// SSE tool call.  `None` when no question is awaiting an answer.
        active_question: Option<crate::api::Question>,
        /// Index of the currently-highlighted option (single-select) or
        /// the last-moved position (multi-select).
        question_cursor: usize,
        /// Set of selected option indices (multi-select only).
        question_checked: Vec<bool>,

        // ── Server metrics (M19 item 2) ──────────────────────────
        /// Last-fetched server-side consolidation metrics for this agent.
        agent_metrics: Option<crate::api::AgentMetrics>,

        // ── Cumulative token usage totals (M19 item 3 /stats) ────
        /// Running total of input tokens across all turns in this session.
        total_input_tokens: u64,
        /// Running total of output tokens across all turns in this session.
        total_output_tokens: u64,

        // ── Context stats overlay (M19 item 3 /context) ──────────
        /// Whether the context-stats overlay is open.
        context_open: bool,
        /// Last-fetched context window stats.
        context_stats: Option<crate::api::ContextStats>,
        /// True while the GET /context request is in flight.
        context_loading: bool,
        /// Per-overlay error for context panel.
        context_error: Option<String>,

        // ── Agents overlay (M19 item 3 /agents) ──────────────────
        /// Whether the agents list overlay is open.
        agents_open: bool,

        // ── Stats overlay (M19 item 3 /stats) ────────────────────
        /// Whether the stats overlay is open.
        stats_open: bool,

        // ── MCP servers overlay ───────────────────────────────────
        /// Whether the MCP servers overlay is open.
        mcp_open: bool,
        /// Servers fetched from `GET /v1/mcp`.
        mcp_servers: Vec<crate::api::McpServerInfo>,
        /// True while the GET request is in flight.
        mcp_loading: bool,
        /// Per-overlay error message.
        mcp_error: Option<String>,

        /// A pending theme update from the backend.
        theme_update: Option<crate::theme::ThemeColors>,

        // ── Model picker overlay ─────────────────────────────────
        /// Whether the model picker overlay is open.
        model_picker_open: bool,
        /// Available models fetched from `GET /v1/models`.
        model_picker_models: Vec<crate::api::ModelInfo>,
        /// Custom provider names (no model listing available).
        model_picker_custom_providers: Vec<String>,
        /// Fuzzy filter query typed in the model picker search box.
        model_picker_query: String,
        /// Index of the currently highlighted model in the filtered list.
        model_picker_selection: usize,
        /// Whether models are currently being fetched.
        model_picker_loading: bool,
        /// Error message from model fetch failure.
        model_picker_error: Option<String>,

        // ── Plan panel (mirrors TUI PlanState) ──────────────────
        /// Active plan steps. `None` when no plan has been set.
        active_plan: Option<PlanState>,

        // ── Live output (mirrors TUI LiveOutput) ─────────────────
        /// Active live-output blocks keyed by tool call ID.
        /// Each entry is a scrollable block of output lines shown in the
        /// timeline while a long-running tool (e.g. `bash`) is executing.
        live_outputs: Vec<LiveOutputBlock>,

        /// Per-category context-window breakdown (fetched on demand).
        context_breakdown: Option<crate::api::ContextBreakdown>,
        /// Whether a context-breakdown fetch is in progress.
        context_breakdown_loading: bool,

        // ── Settings overlays ────────────────────────────────────
        providers_open: bool,
        providers: Vec<crate::api::ProviderInfo>,
        providers_loading: bool,
        permissions_open: bool,
        current_permission_mode: String,
        theme_picker_open: bool,
        available_themes: Vec<String>,
        current_theme_name: String,
        hooks_open: bool,
        hooks: Vec<crate::api::HookInfo>,
        hooks_loading: bool,
        toolset_open: bool,
        current_toolset: String,
        pricing_open: bool,
        pricing_info: String,
        backend_open: bool,
        current_backend: String,
        reasoning_open: bool,
        current_reasoning_effort: String,
        // ── Skills overlay ───────────────────────────────────
        skills_overlay_open: bool,
        all_skills_list: Vec<crate::api::SkillEntry>,
        loaded_skill_ids: Vec<String>,
        skills_loading: bool,
        skills_filter: String,
    },
    /// One of the bootstrap requests failed.
    ConnectionFailed {
        server_url: String,
        token: String,
        error: String,
    },
}

impl SessionState {
    /// Begin a new session after the user submits their token.
    ///
    /// `server_url` is the base URL of the cade-server instance (from
    /// `Config::server_url`).  `token` is the trimmed API key from
    /// `LoginState::Submitted { key }`.
    pub fn start(server_url: &str, token: &str) -> Self {
        Self::Connecting {
            server_url: server_url.to_string(),
            token: token.to_string(),
        }
    }

    /// The server URL this session targets.
    pub fn server_url(&self) -> &str {
        match self {
            Self::Connecting { server_url, .. }
            | Self::HealthOk { server_url, .. }
            | Self::Connected { server_url, .. }
            | Self::ConnectionFailed { server_url, .. } => server_url,
        }
    }

    /// The bearer token for this session.
    pub fn token(&self) -> &str {
        match self {
            Self::Connecting { token, .. }
            | Self::HealthOk { token, .. }
            | Self::Connected { token, .. }
            | Self::ConnectionFailed { token, .. } => token,
        }
    }

    /// Feed a successful health-check result.
    ///
    /// Only transitions from `Connecting` → `HealthOk`.
    /// No-op in any other state (idempotent against duplicate calls).
    pub fn on_health(&mut self, health: HealthInfo) {
        if let Self::Connecting {
            server_url, token, ..
        } = self
        {
            *self = Self::HealthOk {
                server_url: std::mem::take(server_url),
                token: std::mem::take(token),
                health,
            };
        }
    }

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
            };
        }
    }

    /// Feed an error from either the health or agent-list request.
    ///
    /// Transitions from `Connecting` or `HealthOk` → `ConnectionFailed`.
    /// No-op if already `Connected` or `ConnectionFailed`.
    pub fn on_error(&mut self, error: String) {
        match self {
            Self::Connecting {
                server_url, token, ..
            }
            | Self::HealthOk {
                server_url, token, ..
            } => {
                *self = Self::ConnectionFailed {
                    server_url: std::mem::take(server_url),
                    token: std::mem::take(token),
                    error,
                };
            }
            _ => {}
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

    /// Feed the message list fetched for the currently selected agent.
    ///
    /// Only applies when `Connected` and an agent is selected.  No-op
    /// otherwise.
    pub fn on_messages(&mut self, msgs: Vec<ChatMessage>) {
        if let Self::Connected { messages, .. } = self {
            *messages = msgs;
        }
    }

    /// Set messages and pagination flag from a paged fetch.
    pub fn on_messages_paged(&mut self, msgs: Vec<ChatMessage>, has_more: bool) {
        if let Self::Connected {
            messages,
            has_more_messages,
            ..
        } = self
        {
            *messages = msgs;
            *has_more_messages = has_more;
        }
    }

    /// Prepend older messages (from "Load more") to the beginning.
    pub fn on_prepend_messages(&mut self, older: Vec<ChatMessage>, has_more: bool) {
        if let Self::Connected {
            messages,
            has_more_messages,
            ..
        } = self
        {
            let mut combined = older;
            combined.append(messages);
            *messages = combined;
            *has_more_messages = has_more;
        }
    }

    /// Whether there are older messages to load.
    pub fn has_more_messages(&self) -> bool {
        if let Self::Connected {
            has_more_messages, ..
        } = self
        {
            *has_more_messages
        } else {
            false
        }
    }

    /// Current message count (used as offset for pagination).
    pub fn message_count(&self) -> usize {
        if let Self::Connected { messages, .. } = self {
            messages.len()
        } else {
            0
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

    /// Submit the current input buffer as a user message.
    ///
    /// Returns the trimmed input text if the send is valid (agent selected,
    /// non-empty buffer, not already streaming).  Returns `None` if it's a
    /// no-op.  On success:
    ///   1. Appends a `ChatMessage { role: "user", content: input }` to messages.
    ///   2. Clears the input buffer.
    ///   3. Sets `streaming = true`.
    pub fn on_send(&mut self) -> Option<String> {
        if let Self::Connected {
            selected_agent: Some(_),
            input_buffer,
            messages,
            streaming,
            last_usage,
            last_finish_reason,
            ..
        } = self
        {
            if *streaming {
                return None;
            }
            let trimmed = input_buffer.trim().to_string();
            if trimmed.is_empty() {
                return None;
            }
            messages.push(ChatMessage {
                id: String::new(), // server assigns a real ID
                role: "user".to_string(),
                content: serde_json::Value::String(trimmed.clone()),
                conversation_id: None,
            });
            input_buffer.clear();
            *streaming = true;
            *last_usage = None;
            *last_finish_reason = None;
            Some(trimmed)
        } else {
            None
        }
    }

    /// Append a chunk of streamed assistant text.
    ///
    /// If the last message is already an assistant message (the one we're
    /// building), the chunk is appended to its content.  Otherwise a new
    /// assistant message is created.
    pub fn on_stream_chunk(&mut self, text: &str) {
        if let Self::Connected {
            messages,
            streaming: true,
            auto_scroll,
            ..
        } = self
        {
            // Append to existing assistant message or create one.
            if let Some(last) = messages.last_mut()
                && last.role == "assistant"
                && last.id.is_empty()
            {
                // Accumulate into the in-progress message.
                if let serde_json::Value::String(ref mut s) = last.content {
                    s.push_str(text);
                }
                return;
            }
            // First chunk — create the assistant message and re-enable scroll.
            *auto_scroll = true;
            messages.push(ChatMessage {
                id: String::new(),
                role: "assistant".to_string(),
                content: serde_json::Value::String(text.to_string()),
                conversation_id: None,
            });
        }
    }

    /// Mark the SSE stream as complete.
    pub fn on_stream_done(&mut self) {
        if let Self::Connected { streaming, .. } = self {
            *streaming = false;
        }
    }

    /// Append a chunk of streamed reasoning text.
    ///
    /// Works like `on_stream_chunk` but uses `role = "reasoning"`.
    /// Consecutive reasoning chunks accumulate into the same message.
    pub fn on_stream_reasoning(&mut self, text: &str) {
        if let Self::Connected {
            messages,
            streaming: true,
            ..
        } = self
        {
            if let Some(last) = messages.last_mut()
                && last.role == "reasoning"
                && last.id.is_empty()
            {
                if let serde_json::Value::String(ref mut s) = last.content {
                    s.push_str(text);
                }
                return;
            }
            messages.push(ChatMessage {
                id: String::new(),
                role: "reasoning".to_string(),
                content: serde_json::Value::String(text.to_string()),
                conversation_id: None,
            });
        }
    }

    /// Record a tool call emitted by the assistant during streaming.
    ///
    /// Each tool call becomes its own message with `role = "tool_call"`
    /// and structured JSON content.
    ///
    /// Special case: when `name == "ask_user_question"` the arguments are
    /// also parsed into an [`crate::api::Question`] and set as the active
    /// question so the inline widget can render.
    pub fn on_stream_tool_call(&mut self, id: &str, name: &str, arguments: &str) {
        if let Self::Connected {
            messages,
            streaming: true,
            active_question,
            question_cursor,
            question_checked,
            active_plan,
            ..
        } = self
        {
            messages.push(ChatMessage {
                id: String::new(),
                role: "tool_call".to_string(),
                content: serde_json::json!({
                    "id": id,
                    "name": name,
                    "arguments": arguments,
                }),
                conversation_id: None,
            });

            // Surface inline question widget for `ask_user_question`.
            if name == "ask_user_question" {
                if let Some(q) = crate::api::parse_ask_question(arguments) {
                    let n = q.options.len();
                    *question_cursor = 0;
                    *question_checked = vec![false; n];
                    *active_question = Some(q);
                }
            }

            // Intercept plan panel tool calls.
            if name == "set_plan" {
                let steps: Vec<String> = serde_json::from_str::<serde_json::Value>(arguments)
                    .ok()
                    .and_then(|v| v["steps"].as_array().map(|arr| {
                        arr.iter()
                            .filter_map(|s| s.as_str().map(|s| s.to_string()))
                            .collect()
                    }))
                    .unwrap_or_default();
                if steps.is_empty() {
                    *active_plan = None;
                } else {
                    *active_plan = Some(PlanState {
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
                }
            }

            if name == "UpdatePlan" {
                if let Ok(v) = serde_json::from_str::<serde_json::Value>(arguments) {
                    let step_id = v["step_id"].as_u64().unwrap_or(0) as usize;
                    let done = v["done"].as_bool().unwrap_or(true);
                    if let Some(plan) = active_plan {
                        if let Some(step) = plan.steps.iter_mut().find(|s| s.id == step_id) {
                            step.is_done = done;
                        }
                    }
                }
            }
        }
    }

    /// Record a tool result emitted by the server during the agentic loop.
    ///
    /// Appended as a `role = "tool_result"` message for display in the timeline.
    pub fn on_stream_tool_result(&mut self, id: &str, name: &str, output: &str, is_error: bool) {
        if let Self::Connected {
            messages,
            streaming: true,
            ..
        } = self
        {
            messages.push(ChatMessage {
                id: String::new(),
                role: "tool_result".to_string(),
                content: serde_json::json!({
                    "id":       id,
                    "name":     name,
                    "output":   output,
                    "is_error": is_error,
                }),
                conversation_id: None,
            });
        }
    }

    /// Whether the session is currently streaming an assistant response.
    pub fn is_streaming(&self) -> bool {
        matches!(
            self,
            Self::Connected {
                streaming: true,
                ..
            }
        )
    }

    /// Whether the timeline should auto-scroll to the bottom.
    pub fn auto_scroll(&self) -> bool {
        if let Self::Connected { auto_scroll, .. } = self {
            *auto_scroll
        } else {
            true
        }
    }

    /// Disable auto-scroll (user scrolled up manually).
    pub fn disable_auto_scroll(&mut self) {
        if let Self::Connected { auto_scroll, .. } = self {
            *auto_scroll = false;
        }
    }

    /// Re-enable auto-scroll (user clicked ↓ button).
    pub fn enable_auto_scroll(&mut self) {
        if let Self::Connected { auto_scroll, .. } = self {
            *auto_scroll = true;
        }
    }

    /// Whether the caller should attempt a retry (re-enter the login flow).
    /// Only meaningful in `ConnectionFailed`.
    pub fn is_failed(&self) -> bool {
        matches!(self, Self::ConnectionFailed { .. })
    }

    // ── Error toast API ────────────────────────────────────────────────

    /// Store an error message for display as a toast overlay.
    ///
    /// If a stream is in progress it is marked complete so the UI unblocks.
    /// Replaces any previously stored error.
    pub fn push_error(&mut self, msg: &str) {
        if let Self::Connected {
            streaming,
            error_toast,
            ..
        } = self
        {
            *streaming = false;
            *error_toast = Some(msg.to_string());
        }
    }

    /// Clear the current error toast (e.g. after the user dismisses it).
    pub fn dismiss_error(&mut self) {
        if let Self::Connected { error_toast, .. } = self {
            *error_toast = None;
        }
    }

    /// The current error message, if any.
    pub fn error_toast(&self) -> Option<&str> {
        if let Self::Connected { error_toast, .. } = self {
            error_toast.as_deref()
        } else {
            None
        }
    }

    // ── Conversation ID API ────────────────────────────────────────────

    /// Store a conversation_id received from the server (e.g. SSE metadata).
    pub fn on_conversation_id(&mut self, id: &str) {
        if let Self::Connected {
            conversation_id, ..
        } = self
        {
            *conversation_id = Some(id.to_string());
        }
    }

    /// The active conversation ID, if any.
    pub fn conversation_id(&self) -> Option<&str> {
        if let Self::Connected {
            conversation_id, ..
        } = self
        {
            conversation_id.as_deref()
        } else {
            None
        }
    }

    // ── Usage / finish reason API ──────────────────────────────────────

    /// Store token usage statistics from a `usage_statistics` SSE event.
    pub fn on_usage(&mut self, input_tokens: u64, output_tokens: u64, model: Option<&str>) {
        if let Self::Connected {
            last_usage,
            total_input_tokens,
            total_output_tokens,
            ..
        } = self
        {
            *last_usage = Some((input_tokens, output_tokens, model.map(String::from)));
            *total_input_tokens += input_tokens;
            *total_output_tokens += output_tokens;
        }
    }

    /// Store the finish reason from a `finish_reason` SSE event.
    pub fn on_finish_reason(&mut self, reason: &str) {
        if let Self::Connected {
            last_finish_reason, ..
        } = self
        {
            *last_finish_reason = Some(reason.to_string());
        }
    }

    /// Last token usage: `(input_tokens, output_tokens, model)`.
    pub fn last_usage(&self) -> Option<(u64, u64, Option<&str>)> {
        if let Self::Connected { last_usage, .. } = self {
            last_usage
                .as_ref()
                .map(|(i, o, m)| (*i, *o, m.as_deref()))
        } else {
            None
        }
    }

    /// Last finish reason (e.g. "stop", "length").
    pub fn last_finish_reason(&self) -> Option<&str> {
        if let Self::Connected {
            last_finish_reason, ..
        } = self
        {
            last_finish_reason.as_deref()
        } else {
            None
        }
    }

    // ── Conversation management ─────────────────────────────────────────

    /// Store conversations fetched from the server.
    pub fn on_conversations(&mut self, convs: Vec<crate::api::ConversationInfo>) {
        if let Self::Connected {
            conversations, ..
        } = self
        {
            *conversations = convs;
        }
    }

    /// The current list of conversations.
    pub fn conversations(&self) -> &[crate::api::ConversationInfo] {
        if let Self::Connected {
            conversations, ..
        } = self
        {
            conversations
        } else {
            &[]
        }
    }

    /// Currently selected conversation index.
    pub fn selected_conversation(&self) -> Option<usize> {
        if let Self::Connected {
            selected_conversation,
            ..
        } = self
        {
            *selected_conversation
        } else {
            None
        }
    }

    /// Select a conversation by index.  Returns `true` if the selection
    /// changed.  When changed, clears messages and sets conversation_id
    /// so the caller can re-fetch messages for that conversation.
    pub fn on_select_conversation(&mut self, idx: usize) -> bool {
        if let Self::Connected {
            conversations,
            selected_conversation,
            messages,
            conversation_id,
            ..
        } = self
        {
            if idx >= conversations.len() {
                return false;
            }
            if *selected_conversation == Some(idx) {
                return false;
            }
            *selected_conversation = Some(idx);
            *conversation_id = Some(conversations[idx].id.clone());
            messages.clear();
            true
        } else {
            false
        }
    }

    /// Start a fresh conversation — clears conversation_id, messages,
    /// and selected_conversation so the next send creates a new one on
    /// the server.
    pub fn on_new_conversation(&mut self) {
        if let Self::Connected {
            conversation_id,
            messages,
            selected_conversation,
            ..
        } = self
        {
            *conversation_id = None;
            messages.clear();
            *selected_conversation = None;
        }
    }

    /// Remove a conversation at `idx` from the local list.
    ///
    /// If the deleted conversation was selected, the selection is cleared and
    /// `messages` / `conversation_id` are reset so the user starts fresh.
    pub fn on_conversation_deleted(&mut self, idx: usize) {
        if let Self::Connected {
            conversations,
            selected_conversation,
            messages,
            conversation_id,
            ..
        } = self
        {
            if idx >= conversations.len() {
                return;
            }
            conversations.remove(idx);
            match *selected_conversation {
                Some(sel) if sel == idx => {
                    // Deleted the currently-active conversation — reset.
                    *selected_conversation = None;
                    *conversation_id = None;
                    messages.clear();
                }
                Some(sel) if sel > idx => {
                    // Shift selection down by one to keep it pointing at the
                    // same conversation (which moved up in the list).
                    *selected_conversation = Some(sel - 1);
                }
                _ => {}
            }
        }
    }

    /// Clear the local timeline display only.  Does NOT touch the
    /// server — reselecting the agent or sending a message will refetch.
    pub fn clear_timeline_local(&mut self) {
        if let Self::Connected { messages, .. } = self {
            messages.clear();
        }
    }

    /// Return the content of the most recent assistant message, if any.
    /// Used by the `/copy` palette command.
    pub fn last_assistant_content(&self) -> Option<String> {
        if let Self::Connected { messages, .. } = self {
            messages
                .iter()
                .rev()
                .find(|m| m.role == "assistant")
                .map(|m| match &m.content {
                    serde_json::Value::String(s) => s.clone(),
                    other => other.to_string(),
                })
        } else {
            None
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

    /// Whether the session is fully established.
    pub fn is_connected(&self) -> bool {
        matches!(self, Self::Connected { .. })
    }

    // ── Memory overlay state ───────────────────────────────────────

    /// Open the memory overlay. Marks the panel as loading and clears any
    /// previous error; caller is responsible for spawning the fetch.
    pub fn open_memory_overlay(&mut self) {
        if let Self::Connected {
            memory_open,
            memory_loading,
            memory_error,
            memory_save_notice,
            ..
        } = self
        {
            *memory_open = true;
            *memory_loading = true;
            *memory_error = None;
            *memory_save_notice = None;
        }
    }

    /// Close the memory overlay.  Does not clear blocks (so reopening is
    /// instant) but does reset the edit buffer + error.
    pub fn close_memory_overlay(&mut self) {
        if let Self::Connected {
            memory_open,
            memory_saving,
            memory_error,
            memory_save_notice,
            ..
        } = self
        {
            *memory_open = false;
            *memory_saving = false;
            *memory_error = None;
            *memory_save_notice = None;
        }
    }

    /// Whether the memory overlay is currently open.
    pub fn is_memory_open(&self) -> bool {
        matches!(self, Self::Connected { memory_open: true, .. })
    }

    /// Feed the result of a successful memory fetch.  Resets selection
    /// to 0 and seeds the edit buffer with the first block.
    pub fn on_memory_loaded(&mut self, blocks: Vec<crate::api::MemoryBlock>) {
        if let Self::Connected {
            memory_blocks,
            memory_selection,
            memory_edit_buffer,
            memory_loading,
            memory_error,
            ..
        } = self
        {
            *memory_loading = false;
            *memory_error = None;
            *memory_selection = 0;
            *memory_edit_buffer = blocks
                .first()
                .map(|b| b.value.clone())
                .unwrap_or_default();
            *memory_blocks = blocks;
        }
    }

    /// Feed an error from the memory fetch.  Clears the loading flag.
    pub fn on_memory_error(&mut self, err: &str) {
        if let Self::Connected {
            memory_loading,
            memory_saving,
            memory_error,
            memory_save_notice,
            ..
        } = self
        {
            *memory_loading = false;
            *memory_saving = false;
            *memory_error = Some(err.to_string());
            *memory_save_notice = None;
        }
    }

    /// Change which memory block is currently highlighted.  Seeds the
    /// edit buffer with the new block's value (discarding unsaved edits).
    /// Returns `true` if the selection changed, `false` otherwise.
    pub fn select_memory_block(&mut self, idx: usize) -> bool {
        if let Self::Connected {
            memory_blocks,
            memory_selection,
            memory_edit_buffer,
            memory_save_notice,
            ..
        } = self
        {
            if idx >= memory_blocks.len() {
                return false;
            }
            if *memory_selection == idx {
                return false;
            }
            *memory_selection = idx;
            *memory_edit_buffer = memory_blocks[idx].value.clone();
            *memory_save_notice = None;
            true
        } else {
            false
        }
    }

    /// Replace the edit-buffer contents — called on every TextEdit change.
    pub fn set_memory_edit_buffer(&mut self, value: &str) {
        if let Self::Connected {
            memory_edit_buffer,
            ..
        } = self
        {
            *memory_edit_buffer = value.to_string();
        }
    }

    /// Mark a save request as in-flight.
    pub fn on_memory_save_start(&mut self) {
        if let Self::Connected {
            memory_saving,
            memory_error,
            memory_save_notice,
            ..
        } = self
        {
            *memory_saving = true;
            *memory_error = None;
            *memory_save_notice = None;
        }
    }

    /// On successful save, persist the edit buffer into the corresponding
    /// block so the sidebar list reflects the new value, and set a
    /// transient success notice for the overlay (e.g. "Saved /project").
    pub fn on_memory_save_ok(&mut self) {
        if let Self::Connected {
            memory_blocks,
            memory_selection,
            memory_edit_buffer,
            memory_saving,
            memory_error,
            memory_save_notice,
            ..
        } = self
        {
            *memory_saving = false;
            *memory_error = None;
            if let Some(b) = memory_blocks.get_mut(*memory_selection) {
                b.value = memory_edit_buffer.clone();
                *memory_save_notice = Some(format!("Saved /{}", b.label));
            }
        }
    }

    /// Extract the `(label, value)` tuple currently being edited, so the
    /// spawn-helper can issue the PUT.  Returns `None` when the overlay
    /// is closed or no block is selected.
    pub fn memory_selected_label_value(&self) -> Option<(String, String)> {
        if let Self::Connected {
            memory_open: true,
            memory_blocks,
            memory_selection,
            memory_edit_buffer,
            ..
        } = self
        {
            memory_blocks
                .get(*memory_selection)
                .map(|b| (b.label.clone(), memory_edit_buffer.clone()))
        } else {
            None
        }
    }

    /// Whether the in-memory edit buffer differs from the currently-
    /// selected block's saved value.  Used to enable/disable the Save
    /// button and show a dirty indicator.  Returns `false` when the
    /// overlay is closed, no block is selected, or buffer == saved value.
    pub fn is_memory_dirty(&self) -> bool {
        if let Self::Connected {
            memory_open: true,
            memory_blocks,
            memory_selection,
            memory_edit_buffer,
            ..
        } = self
        {
            match memory_blocks.get(*memory_selection) {
                Some(b) => b.value != *memory_edit_buffer,
                None => false,
            }
        } else {
            false
        }
    }

    /// Transient success notice shown after a successful save.  Returns
    /// `None` when no save has completed since the last open/select/error.
    pub fn memory_save_notice(&self) -> Option<&str> {
        if let Self::Connected {
            memory_save_notice: Some(n),
            ..
        } = self
        {
            Some(n.as_str())
        } else {
            None
        }
    }

    // ── Checkpoints overlay (M17) ──────────────────────────────────

    /// Open the checkpoints overlay.  Caller is expected to spawn a
    /// fetch; this just marks the panel as loading and clears error.
    pub fn open_checkpoints_overlay(&mut self) {
        if let Self::Connected {
            checkpoints_open,
            checkpoints_loading,
            checkpoints_error,
            checkpoints_notice,
            ..
        } = self
        {
            *checkpoints_open = true;
            *checkpoints_loading = true;
            *checkpoints_error = None;
            *checkpoints_notice = None;
        }
    }

    /// Close the checkpoints overlay.  Retains the cached list so a
    /// reopen is instant; clears transient flags.
    pub fn close_checkpoints_overlay(&mut self) {
        if let Self::Connected {
            checkpoints_open,
            checkpoints_busy,
            checkpoints_error,
            checkpoints_notice,
            ..
        } = self
        {
            *checkpoints_open = false;
            *checkpoints_busy = false;
            *checkpoints_error = None;
            *checkpoints_notice = None;
        }
    }

    /// Whether the checkpoints overlay is currently visible.
    pub fn is_checkpoints_open(&self) -> bool {
        matches!(
            self,
            Self::Connected {
                checkpoints_open: true,
                ..
            }
        )
    }

    /// Feed the result of a successful checkpoints fetch.
    pub fn on_checkpoints_loaded(&mut self, rows: Vec<crate::api::CheckpointRow>) {
        if let Self::Connected {
            checkpoints,
            checkpoints_loading,
            checkpoints_error,
            ..
        } = self
        {
            *checkpoints_loading = false;
            *checkpoints_error = None;
            *checkpoints = rows;
        }
    }

    /// Feed an error from a checkpoint fetch or action.  Clears
    /// loading + busy flags so the UI becomes interactable again.
    pub fn on_checkpoints_error(&mut self, err: &str) {
        if let Self::Connected {
            checkpoints_loading,
            checkpoints_busy,
            checkpoints_error,
            checkpoints_notice,
            ..
        } = self
        {
            *checkpoints_loading = false;
            *checkpoints_busy = false;
            *checkpoints_error = Some(err.to_string());
            *checkpoints_notice = None;
        }
    }

    /// Mark a restore/create/delete request as in-flight.
    pub fn on_checkpoints_action_start(&mut self) {
        if let Self::Connected {
            checkpoints_busy,
            checkpoints_error,
            checkpoints_notice,
            ..
        } = self
        {
            *checkpoints_busy = true;
            *checkpoints_error = None;
            *checkpoints_notice = None;
        }
    }

    /// Mark an action as completed successfully with a transient notice.
    pub fn on_checkpoints_action_ok(&mut self, notice: &str) {
        if let Self::Connected {
            checkpoints_busy,
            checkpoints_error,
            checkpoints_notice,
            ..
        } = self
        {
            *checkpoints_busy = false;
            *checkpoints_error = None;
            *checkpoints_notice = Some(notice.to_string());
        }
    }

    /// Read-only snapshot of the cached checkpoint list, for tests +
    /// the renderer.  Returns `&[]` when not connected.
    pub fn checkpoints_snapshot(&self) -> &[crate::api::CheckpointRow] {
        if let Self::Connected { checkpoints, .. } = self {
            checkpoints
        } else {
            &[]
        }
    }

    /// Read the current notice string (e.g. "Restored cp-abc…").
    pub fn checkpoints_notice(&self) -> Option<&str> {
        if let Self::Connected {
            checkpoints_notice: Some(n),
            ..
        } = self
        {
            Some(n.as_str())
        } else {
            None
        }
    }

    // ── Artifacts overlay (M17) ────────────────────────────────────

    /// Open the artifacts overlay.  Caller is expected to spawn a list
    /// fetch; this marks the panel as loading and clears error/selection.
    pub fn open_artifacts_overlay(&mut self) {
        if let Self::Connected {
            artifacts_open,
            artifacts_loading,
            artifacts_error,
            artifact_selection,
            artifact_detail,
            ..
        } = self
        {
            *artifacts_open = true;
            *artifacts_loading = true;
            *artifacts_error = None;
            *artifact_selection = None;
            *artifact_detail = None;
        }
    }

    /// Close the artifacts overlay.  Retains cached list for instant
    /// reopen; clears transient flags.
    pub fn close_artifacts_overlay(&mut self) {
        if let Self::Connected {
            artifacts_open,
            artifacts_busy,
            artifacts_error,
            ..
        } = self
        {
            *artifacts_open = false;
            *artifacts_busy = false;
            *artifacts_error = None;
        }
    }

    /// Whether the artifacts overlay is currently visible.
    pub fn is_artifacts_open(&self) -> bool {
        matches!(
            self,
            Self::Connected {
                artifacts_open: true,
                ..
            }
        )
    }

    /// Feed the result of a successful artifacts-list fetch.
    pub fn on_artifacts_loaded(&mut self, rows: Vec<crate::api::ArtifactInfo>) {
        if let Self::Connected {
            artifacts,
            artifacts_loading,
            artifacts_error,
            ..
        } = self
        {
            *artifacts_loading = false;
            *artifacts_error = None;
            *artifacts = rows;
        }
    }

    /// Feed an error from an artifact fetch or action.
    pub fn on_artifacts_error(&mut self, err: &str) {
        if let Self::Connected {
            artifacts_loading,
            artifacts_busy,
            artifacts_error,
            ..
        } = self
        {
            *artifacts_loading = false;
            *artifacts_busy = false;
            *artifacts_error = Some(err.to_string());
        }
    }

    /// Mark a detail/delete request as in-flight.
    pub fn on_artifacts_action_start(&mut self) {
        if let Self::Connected {
            artifacts_busy,
            artifacts_error,
            ..
        } = self
        {
            *artifacts_busy = true;
            *artifacts_error = None;
        }
    }

    /// Select an artifact row.  Clears stale detail so the renderer
    /// shows a loading indicator while the per-id fetch runs.  Returns
    /// the selected artifact id (so the spawn helper can issue the GET)
    /// or `None` when the index is out of bounds / not connected.
    pub fn select_artifact(&mut self, idx: usize) -> Option<String> {
        if let Self::Connected {
            artifacts,
            artifact_selection,
            artifact_detail,
            artifacts_busy,
            artifacts_error,
            ..
        } = self
        {
            let id = artifacts.get(idx).map(|a| a.id.clone());
            if id.is_some() {
                *artifact_selection = Some(idx);
                *artifact_detail = None;
                *artifacts_busy = true;
                *artifacts_error = None;
            }
            id
        } else {
            None
        }
    }

    /// Feed full detail after a successful per-id fetch.
    pub fn on_artifact_detail_loaded(&mut self, detail: crate::api::ArtifactDetail) {
        if let Self::Connected {
            artifact_detail,
            artifacts_busy,
            artifacts_error,
            ..
        } = self
        {
            *artifacts_busy = false;
            *artifacts_error = None;
            *artifact_detail = Some(detail);
        }
    }

    /// Return the id of the artifact currently selected, if any.  Used
    /// by the delete button to pass the right id to the spawn helper.
    pub fn selected_artifact_id(&self) -> Option<String> {
        if let Self::Connected {
            artifacts,
            artifact_selection: Some(idx),
            ..
        } = self
        {
            artifacts.get(*idx).map(|a| a.id.clone())
        } else {
            None
        }
    }

    /// Read-only snapshot of the cached artifact list.
    pub fn artifacts_snapshot(&self) -> &[crate::api::ArtifactInfo] {
        if let Self::Connected { artifacts, .. } = self {
            artifacts
        } else {
            &[]
        }
    }

    /// Read-only access to the currently-loaded artifact detail (if any).
    pub fn artifact_detail(&self) -> Option<&crate::api::ArtifactDetail> {
        if let Self::Connected {
            artifact_detail, ..
        } = self
        {
            artifact_detail.as_ref()
        } else {
            None
        }
    }

    // ── Tools overlay (M18) ────────────────────────────────────────

    /// Open the tools overlay.  Caller spawns the fetch.
    pub fn open_tools_overlay(&mut self) {
        if let Self::Connected {
            tools_open,
            tools_loading,
            tools_error,
            ..
        } = self
        {
            *tools_open = true;
            *tools_loading = true;
            *tools_error = None;
        }
    }

    /// Close the tools overlay.
    pub fn close_tools_overlay(&mut self) {
        if let Self::Connected {
            tools_open,
            tools_error,
            ..
        } = self
        {
            *tools_open = false;
            *tools_error = None;
        }
    }

    /// Whether the tools overlay is currently visible.
    pub fn is_tools_open(&self) -> bool {
        matches!(self, Self::Connected { tools_open: true, .. })
    }

    /// Feed the result of a successful tools fetch.
    pub fn on_tools_loaded(&mut self, rows: Vec<crate::api::AgentTool>) {
        if let Self::Connected {
            tools,
            tools_loading,
            tools_error,
            ..
        } = self
        {
            *tools_loading = false;
            *tools_error = None;
            *tools = rows;
        }
    }

    /// Feed an error from a tools fetch.
    pub fn on_tools_error(&mut self, err: &str) {
        if let Self::Connected {
            tools_loading,
            tools_error,
            ..
        } = self
        {
            *tools_loading = false;
            *tools_error = Some(err.to_string());
        }
    }

    /// Read-only snapshot of the cached tool list.
    pub fn tools_snapshot(&self) -> &[crate::api::AgentTool] {
        if let Self::Connected { tools, .. } = self {
            tools
        } else {
            &[]
        }
    }

    // ── Inline question widget (M18) ──────────────────────────────

    /// Present a question from an `ask_user_question` tool call.
    ///
    /// Initialises cursor to 0 and checked-vec to all-false.  If a
    /// question is already active it is replaced (the server serialises
    /// tool calls so this shouldn't happen in practice).
    pub fn set_active_question(&mut self, q: crate::api::Question) {
        if let Self::Connected {
            active_question,
            question_cursor,
            question_checked,
            ..
        } = self
        {
            let n = q.options.len();
            *question_cursor = 0;
            *question_checked = vec![false; n];
            *active_question = Some(q);
        }
    }

    /// Clear the active question (after the user answers or cancels).
    pub fn clear_active_question(&mut self) {
        if let Self::Connected {
            active_question,
            question_cursor,
            question_checked,
            ..
        } = self
        {
            *active_question = None;
            *question_cursor = 0;
            question_checked.clear();
        }
    }

    /// Whether a question is currently awaiting an answer.
    pub fn has_active_question(&self) -> bool {
        matches!(
            self,
            Self::Connected {
                active_question: Some(_),
                ..
            }
        )
    }

    /// Immutable reference to the active question, if any.
    pub fn active_question(&self) -> Option<&crate::api::Question> {
        if let Self::Connected {
            active_question: Some(q),
            ..
        } = self
        {
            Some(q)
        } else {
            None
        }
    }

    /// Move the question cursor up or down (wraps).  `delta` is -1 or +1.
    pub fn move_question_cursor(&mut self, delta: i32) {
        if let Self::Connected {
            active_question: Some(q),
            question_cursor,
            ..
        } = self
        {
            let n = q.options.len();
            if n == 0 {
                return;
            }
            let cur = *question_cursor as i32;
            *question_cursor = ((cur + delta).rem_euclid(n as i32)) as usize;
        }
    }

    /// Toggle the checked state for the option at the current cursor
    /// (multi-select mode only).
    pub fn toggle_question_checked(&mut self) {
        if let Self::Connected {
            active_question: Some(q),
            question_cursor,
            question_checked,
            ..
        } = self
        {
            if q.multi_select {
                let idx = *question_cursor;
                if let Some(v) = question_checked.get_mut(idx) {
                    *v = !*v;
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
        if let Self::Connected {
            active_question: Some(q),
            question_cursor,
            question_checked,
            ..
        } = self
        {
            let answer = if q.multi_select {
                let labels: Vec<&str> = question_checked
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
                q.options
                    .get(*question_cursor)
                    .map(|o| o.label.clone())?
            };
            Some(answer)
        } else {
            None
        }
    }

    // ── Agent metrics (M19 item 2) ─────────────────────────────────

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

    // ── Cumulative token totals (M19 item 3 /stats) ────────────────

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

    // ── Context stats overlay (M19 item 3 /context) ────────────────

    /// Open the context-stats overlay.  Caller spawns the fetch.
    pub fn open_context_overlay(&mut self) {
        if let Self::Connected {
            context_open,
            context_loading,
            context_error,
            context_breakdown_loading,
            ..
        } = self
        {
            *context_open = true;
            *context_loading = true;
            *context_error = None;
            *context_breakdown_loading = true;
        }
    }

    /// Close the context-stats overlay.
    pub fn close_context_overlay(&mut self) {
        if let Self::Connected {
            context_open,
            context_error,
            ..
        } = self
        {
            *context_open = false;
            *context_error = None;
        }
    }

    /// Whether the context overlay is open.
    pub fn is_context_open(&self) -> bool {
        matches!(self, Self::Connected { context_open: true, .. })
    }

    /// Feed a successful context-stats response.
    pub fn on_context_loaded(&mut self, stats: crate::api::ContextStats) {
        if let Self::Connected {
            context_stats,
            context_loading,
            context_error,
            ..
        } = self
        {
            *context_loading = false;
            *context_error = None;
            *context_stats = Some(stats);
        }
    }

    /// Feed an error from the context fetch.
    pub fn on_context_error(&mut self, err: &str) {
        if let Self::Connected {
            context_loading,
            context_error,
            ..
        } = self
        {
            *context_loading = false;
            *context_error = Some(err.to_string());
        }
    }

    /// Read-only access to last-fetched context stats.
    pub fn context_stats(&self) -> Option<&crate::api::ContextStats> {
        if let Self::Connected { context_stats, .. } = self {
            context_stats.as_ref()
        } else {
            None
        }
    }

    // ── Agents overlay (M19 item 3 /agents) ────────────────────────

    /// Open the agents list overlay.
    pub fn open_agents_overlay(&mut self) {
        if let Self::Connected { agents_open, .. } = self {
            *agents_open = true;
        }
    }

    /// Close the agents list overlay.
    pub fn close_agents_overlay(&mut self) {
        if let Self::Connected { agents_open, .. } = self {
            *agents_open = false;
        }
    }

    /// Whether the agents overlay is open.
    pub fn is_agents_open(&self) -> bool {
        matches!(self, Self::Connected { agents_open: true, .. })
    }

    // ── Stats overlay (M19 item 3 /stats) ──────────────────────────

    /// Open the stats overlay.
    pub fn open_stats_overlay(&mut self) {
        if let Self::Connected { stats_open, .. } = self {
            *stats_open = true;
        }
    }

    /// Close the stats overlay.
    pub fn close_stats_overlay(&mut self) {
        if let Self::Connected { stats_open, .. } = self {
            *stats_open = false;
        }
    }

    /// Whether the stats overlay is open.
    pub fn is_stats_open(&self) -> bool {
        matches!(self, Self::Connected { stats_open: true, .. })
    }

    // ── MCP servers overlay ────────────────────────────────────────

    /// Open the MCP servers overlay and mark loading state.
    pub fn open_mcp_overlay(&mut self) {
        if let Self::Connected { mcp_open, mcp_loading, mcp_error, .. } = self {
            *mcp_open = true;
            *mcp_loading = true;
            *mcp_error = None;
        }
    }

    /// Close the MCP servers overlay and clear any error.
    pub fn close_mcp_overlay(&mut self) {
        if let Self::Connected { mcp_open, mcp_error, .. } = self {
            *mcp_open = false;
            *mcp_error = None;
        }
    }

    /// Returns `true` when the MCP overlay is visible.
    pub fn is_mcp_open(&self) -> bool {
        matches!(self, Self::Connected { mcp_open: true, .. })
    }

    /// Store freshly-fetched MCP server list and clear loading state.
    pub fn on_mcp_loaded(&mut self, servers: Vec<crate::api::McpServerInfo>) {
        if let Self::Connected { mcp_servers, mcp_loading, mcp_error, .. } = self {
            *mcp_servers = servers;
            *mcp_loading = false;
            *mcp_error = None;
        }
    }

    /// Record a fetch error and clear the loading flag.
    pub fn on_mcp_error(&mut self, err: String) {
        if let Self::Connected { mcp_loading, mcp_error, .. } = self {
            *mcp_loading = false;
            *mcp_error = Some(err);
        }
    }

    // ── Model picker overlay ───────────────────────────────────────

    /// Open the model picker overlay and mark loading state.
    pub fn open_model_picker(&mut self) {
        if let Self::Connected {
            model_picker_open,
            model_picker_query,
            model_picker_selection,
            model_picker_loading,
            model_picker_error,
            ..
        } = self
        {
            *model_picker_open = true;
            model_picker_query.clear();
            *model_picker_selection = 0;
            *model_picker_loading = true;
            *model_picker_error = None;
        }
    }

    /// Close the model picker overlay.
    pub fn close_model_picker(&mut self) {
        if let Self::Connected {
            model_picker_open, ..
        } = self
        {
            *model_picker_open = false;
        }
    }

    /// Whether the model picker overlay is open.
    pub fn is_model_picker_open(&self) -> bool {
        matches!(
            self,
            Self::Connected {
                model_picker_open: true,
                ..
            }
        )
    }

    /// Called when models are successfully fetched.
    pub fn on_models_loaded(
        &mut self,
        models: Vec<crate::api::ModelInfo>,
        custom_providers: Vec<String>,
    ) {
        if let Self::Connected {
            model_picker_models,
            model_picker_custom_providers,
            model_picker_loading,
            model_picker_error,
            ..
        } = self
        {
            *model_picker_models = models;
            *model_picker_custom_providers = custom_providers;
            *model_picker_loading = false;
            *model_picker_error = None;
        }
    }

    /// Called when model fetch fails.
    pub fn on_models_error(&mut self, err: String) {
        if let Self::Connected {
            model_picker_loading,
            model_picker_error,
            ..
        } = self
        {
            *model_picker_loading = false;
            *model_picker_error = Some(err);
        }
    }

    /// Set the model picker search query and reset selection to 0.
    pub fn set_model_picker_query(&mut self, q: String) {
        if let Self::Connected {
            model_picker_query,
            model_picker_selection,
            ..
        } = self
        {
            *model_picker_query = q;
            *model_picker_selection = 0;
        }
    }

    /// Set the model picker selection index.
    pub fn set_model_picker_selection(&mut self, idx: usize) {
        if let Self::Connected {
            model_picker_selection,
            ..
        } = self
        {
            *model_picker_selection = idx;
        }
    }

    /// Get the currently selected model ID from the filtered list.
    ///
    /// Returns `None` if no models or selection is out of range.
    pub fn selected_model_id(&self) -> Option<String> {
        if let Self::Connected {
            model_picker_models,
            model_picker_query,
            model_picker_selection,
            ..
        } = self
        {
            let filtered = filter_models(model_picker_models, model_picker_query);
            filtered.get(*model_picker_selection).map(|m| m.id.clone())
        } else {
            None
        }
    }

    // ── Palette (slash-command) state ──────────────────────────────

    /// Open the slash-command palette overlay.
    ///
    /// Resets query + selection.  Optional `initial_input` pre-fills the
    /// filter text (used when the user typed `/foo` in the input bar).
    pub fn open_palette(&mut self, initial_input: &str) {
        if let Self::Connected {
            palette_open,
            palette_input,
            palette_selection,
            ..
        } = self
        {
            *palette_open = true;
            *palette_input = initial_input.to_string();
            *palette_selection = 0;
        }
    }

    /// Close the palette.  Clears query + selection.
    pub fn close_palette(&mut self) {
        if let Self::Connected {
            palette_open,
            palette_input,
            palette_selection,
            ..
        } = self
        {
            *palette_open = false;
            palette_input.clear();
            *palette_selection = 0;
        }
    }

    /// Replace the palette filter input. Resets selection to 0 so the top
    /// result stays highlighted as the user types.
    pub fn set_palette_input(&mut self, query: &str) {
        if let Self::Connected {
            palette_input,
            palette_selection,
            ..
        } = self
        {
            *palette_input = query.to_string();
            *palette_selection = 0;
        }
    }

    /// Move the palette selection up (-1) or down (+1), clamped to the
    /// number of filtered entries for the current query.
    pub fn move_palette_selection(&mut self, delta: i32) {
        if let Self::Connected {
            palette_input,
            palette_selection,
            ..
        } = self
        {
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
        matches!(self, Self::Connected { palette_open: true, .. })
    }

    /// Parse the currently-selected palette entry into a concrete
    /// [`crate::palette::PaletteCmd`].  Returns `None` if the palette is
    /// closed or there are no matching entries for the query.
    pub fn selected_palette_cmd(&self) -> Option<crate::palette::PaletteCmd> {
        if let Self::Connected {
            palette_open: true,
            palette_input,
            palette_selection,
            ..
        } = self
        {
            let filtered = crate::palette::fuzzy_filter(palette_input);
            if filtered.is_empty() {
                return None;
            }
            let idx = (*palette_selection).min(filtered.len() - 1);
            Some(crate::palette::parse_palette_input(
                filtered[idx].def.trigger,
            ))
        } else {
            None
        }
    }

    // ── Full-Screen Command Menu State ──────────────────────────

    /// Open the full-screen command menu overlay.
    pub fn open_menu(&mut self, initial_input: &str) {
        if let Self::Connected {
            menu_open,
            menu_input,
            menu_selection,
            ..
        } = self
        {
            *menu_open = true;
            *menu_input = initial_input.to_string();
            *menu_selection = 0;
        }
    }

    /// Close the command menu.
    pub fn close_menu(&mut self) {
        if let Self::Connected {
            menu_open,
            menu_input,
            menu_selection,
            ..
        } = self
        {
            *menu_open = false;
            menu_input.clear();
            *menu_selection = 0;
        }
    }

    /// Replace the menu filter input and reset selection.
    pub fn set_menu_input(&mut self, query: &str) {
        if let Self::Connected {
            menu_input,
            menu_selection,
            ..
        } = self
        {
            *menu_input = query.to_string();
            *menu_selection = 0;
        }
    }

    /// Move the menu selection up (-1) or down (+1).
    pub fn move_menu_selection(&mut self, delta: i32) {
        if let Self::Connected {
            menu_input,
            menu_selection,
            ..
        } = self
        {
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
        matches!(self, Self::Connected { menu_open: true, .. })
    }

    /// Parse the currently-selected menu entry.
    pub fn selected_menu_cmd(&self) -> Option<crate::palette::PaletteCmd> {
        if let Self::Connected {
            menu_open: true,
            menu_input,
            menu_selection,
            ..
        } = self
        {
            let filtered = crate::palette::fuzzy_filter(menu_input);
            if filtered.is_empty() {
                return None;
            }
            let idx = (*menu_selection).min(filtered.len() - 1);
            Some(crate::palette::parse_palette_input(
                filtered[idx].def.trigger,
            ))
        } else {
            None
        }
    }
}

// ── Model filtering helper ────────────────────────────────────────────

/// Filter models by a fuzzy query.  Matches against `id`, `display_name`,
/// and `provider` (case-insensitive substring).
pub fn filter_models<'a>(
    models: &'a [crate::api::ModelInfo],
    query: &str,
) -> Vec<&'a crate::api::ModelInfo> {
    if query.is_empty() {
        return models.iter().collect();
    }
    let q = query.to_lowercase();
    models
        .iter()
        .filter(|m| {
            m.id.to_lowercase().contains(&q)
                || m.display_name.to_lowercase().contains(&q)
                || m.provider.to_lowercase().contains(&q)
        })
        .collect()
}
impl SessionState {
    pub fn on_theme_update(&mut self, theme: crate::theme::ThemeColors) {
        if let Self::Connected { theme_update, .. } = self {
            *theme_update = Some(theme);
        }
    }
}

// ── Plan panel methods ──────────────────────────────────────────────────

impl SessionState {
    /// Set the plan from a `set_plan` tool call. Replaces any existing plan.
    pub fn set_plan(&mut self, steps: Vec<String>) {
        if let Self::Connected { active_plan, .. } = self {
            if steps.is_empty() {
                *active_plan = None;
            } else {
                *active_plan = Some(PlanState {
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
            }
        }
    }

    /// Mark a plan step as done or not done. `step_id` is 1-based.
    pub fn update_plan_step(&mut self, step_id: usize, done: bool) -> bool {
        if let Self::Connected { active_plan: Some(plan), .. } = self {
            if let Some(step) = plan.steps.iter_mut().find(|s| s.id == step_id) {
                step.is_done = done;
                return true;
            }
        }
        false
    }

    /// Read-only access to the active plan.
    pub fn active_plan(&self) -> Option<&PlanState> {
        if let Self::Connected { active_plan, .. } = self {
            active_plan.as_ref()
        } else {
            None
        }
    }
}

// ── Live output methods ─────────────────────────────────────────────────

impl SessionState {
    /// Begin a new live-output block for a tool call.
    pub fn begin_live_output(&mut self, call_id: &str, tool_name: &str) {
        if let Self::Connected { live_outputs, .. } = self {
            live_outputs.push(LiveOutputBlock {
                call_id: call_id.to_string(),
                tool_name: tool_name.to_string(),
                lines: Vec::new(),
                done: false,
                max_visible: 8,
            });
        }
    }

    /// Append a line to an existing live-output block.
    pub fn append_live_output(&mut self, call_id: &str, line: String) {
        if let Self::Connected { live_outputs, .. } = self {
            if let Some(block) = live_outputs.iter_mut().find(|b| b.call_id == call_id) {
                block.lines.push(line);
            }
        }
    }

    /// Mark a live-output block as finished.
    pub fn finish_live_output(&mut self, call_id: &str) {
        if let Self::Connected { live_outputs, .. } = self {
            if let Some(block) = live_outputs.iter_mut().find(|b| b.call_id == call_id) {
                block.done = true;
            }
        }
    }

    /// Read-only access to live output blocks.
    pub fn live_outputs(&self) -> &[LiveOutputBlock] {
        if let Self::Connected { live_outputs, .. } = self {
            live_outputs
        } else {
            &[]
        }
    }
}

// ── Context breakdown methods ───────────────────────────────────────────

impl SessionState {
    /// Start loading context breakdown.
    pub fn start_context_breakdown_loading(&mut self) {
        if let Self::Connected { context_breakdown_loading, .. } = self {
            *context_breakdown_loading = true;
        }
    }

    /// Store fetched context breakdown.
    pub fn on_context_breakdown(&mut self, breakdown: crate::api::ContextBreakdown) {
        if let Self::Connected { context_breakdown, context_breakdown_loading, .. } = self {
            *context_breakdown = Some(breakdown);
            *context_breakdown_loading = false;
        }
    }

    /// Clear context breakdown on error.
    pub fn on_context_breakdown_error(&mut self) {
        if let Self::Connected { context_breakdown_loading, .. } = self {
            *context_breakdown_loading = false;
        }
    }

    /// Read-only access to context breakdown.
    pub fn context_breakdown(&self) -> Option<&crate::api::ContextBreakdown> {
        if let Self::Connected { context_breakdown, .. } = self {
            context_breakdown.as_ref()
        } else {
            None
        }
    }

    /// Whether a context breakdown fetch is in progress.
    pub fn is_context_breakdown_loading(&self) -> bool {
        if let Self::Connected { context_breakdown_loading, .. } = self {
            *context_breakdown_loading
        } else {
            false
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use cade_api_types::HealthInfo;

    fn test_health() -> HealthInfo {
        HealthInfo {
            status: "ok".to_string(),
            server: Some("cade-server".to_string()),
            version: Some("0.2.0".to_string()),
        }
    }

    fn test_agents() -> Vec<AgentInfo> {
        vec![
            AgentInfo {
                id: "agent-1".to_string(),
                name: "Test Agent".to_string(),
                model: Some("gpt-4o".to_string()),
                provider: None,
            },
            AgentInfo {
                id: "agent-2".to_string(),
                name: "Second Agent".to_string(),
                model: None,
                provider: None,
            },
        ]
    }

    // ── Construction ────────────────────────────────────────────────────

    #[test]
    fn start_enters_connecting_state() {
        let s = SessionState::start("http://localhost:8284", "my-token");
        assert!(matches!(s, SessionState::Connecting { .. }));
        assert_eq!(s.server_url(), "http://localhost:8284");
        assert_eq!(s.token(), "my-token");
    }

    // ── Happy path: Connecting → HealthOk → Connected ───────────────────

    #[test]
    fn on_health_transitions_connecting_to_health_ok() {
        let mut s = SessionState::start("http://x", "tok");
        s.on_health(test_health());
        match &s {
            SessionState::HealthOk { health, .. } => {
                assert_eq!(health.status, "ok");
            }
            other => panic!("expected HealthOk, got {other:?}"),
        }
    }

    #[test]
    fn on_agents_transitions_health_ok_to_connected() {
        let mut s = SessionState::start("http://x", "tok");
        s.on_health(test_health());
        s.on_agents(test_agents());
        match &s {
            SessionState::Connected { agents, health, .. } => {
                assert_eq!(agents.len(), 2);
                assert_eq!(agents[0].id, "agent-1");
                assert_eq!(agents[1].id, "agent-2");
                assert_eq!(health.status, "ok");
            }
            other => panic!("expected Connected, got {other:?}"),
        }
        assert!(s.is_connected());
    }

    #[test]
    fn connected_preserves_server_url_and_token() {
        let mut s = SessionState::start("http://my-server:9000", "secret-key");
        s.on_health(test_health());
        s.on_agents(vec![]);
        assert_eq!(s.server_url(), "http://my-server:9000");
        assert_eq!(s.token(), "secret-key");
    }

    // ── Error path ──────────────────────────────────────────────────────

    #[test]
    fn on_error_from_connecting_transitions_to_failed() {
        let mut s = SessionState::start("http://x", "tok");
        s.on_error("unauthorized".to_string());
        match &s {
            SessionState::ConnectionFailed { error, .. } => {
                assert_eq!(error, "unauthorized");
            }
            other => panic!("expected ConnectionFailed, got {other:?}"),
        }
        assert!(s.is_failed());
        assert!(!s.is_connected());
    }

    #[test]
    fn on_error_from_health_ok_transitions_to_failed() {
        let mut s = SessionState::start("http://x", "tok");
        s.on_health(test_health());
        s.on_error("agent fetch failed".to_string());
        assert!(s.is_failed());
    }

    #[test]
    fn on_error_preserves_server_url_and_token() {
        let mut s = SessionState::start("http://y", "t");
        s.on_error("boom".to_string());
        assert_eq!(s.server_url(), "http://y");
        assert_eq!(s.token(), "t");
    }

    // ── Idempotency / no-op guards ─────────────────────────────────────

    #[test]
    fn on_health_is_noop_after_connected() {
        let mut s = SessionState::start("http://x", "tok");
        s.on_health(test_health());
        s.on_agents(test_agents());
        assert!(s.is_connected());
        // Second health call should be ignored.
        s.on_health(HealthInfo {
            status: "changed".to_string(),
            server: None,
            version: None,
        });
        // Still connected with original health.
        match &s {
            SessionState::Connected { health, .. } => assert_eq!(health.status, "ok"),
            other => panic!("expected Connected, got {other:?}"),
        }
    }

    #[test]
    fn on_agents_is_noop_from_connecting() {
        let mut s = SessionState::start("http://x", "tok");
        // Calling on_agents before on_health should be a no-op.
        s.on_agents(test_agents());
        assert!(matches!(s, SessionState::Connecting { .. }));
    }

    #[test]
    fn on_error_is_noop_after_connected() {
        let mut s = SessionState::start("http://x", "tok");
        s.on_health(test_health());
        s.on_agents(test_agents());
        assert!(s.is_connected());
        s.on_error("late error".to_string());
        // Should still be connected — error after success is ignored.
        assert!(s.is_connected());
    }

    #[test]
    fn on_error_is_noop_after_already_failed() {
        let mut s = SessionState::start("http://x", "tok");
        s.on_error("first".to_string());
        s.on_error("second".to_string());
        // First error sticks.
        match &s {
            SessionState::ConnectionFailed { error, .. } => assert_eq!(error, "first"),
            other => panic!("expected ConnectionFailed, got {other:?}"),
        }
    }

    // ── Empty agents list ───────────────────────────────────────────────

    #[test]
    fn connected_with_empty_agents_is_valid() {
        let mut s = SessionState::start("http://x", "tok");
        s.on_health(test_health());
        s.on_agents(vec![]);
        assert!(s.is_connected());
        match &s {
            SessionState::Connected { agents, .. } => assert!(agents.is_empty()),
            other => panic!("expected Connected, got {other:?}"),
        }
    }

    // ── Agent selection ─────────────────────────────────────────────────

    fn make_connected() -> SessionState {
        let mut s = SessionState::start("http://x", "tok");
        s.on_health(test_health());
        s.on_agents(test_agents());
        s
    }

    #[test]
    fn on_select_agent_sets_selection_and_clears_messages() {
        let mut s = make_connected();
        assert!(s.on_select_agent(0));
        assert_eq!(s.selected_agent_id(), Some("agent-1"));
        match &s {
            SessionState::Connected {
                selected_agent,
                messages,
                ..
            } => {
                assert_eq!(*selected_agent, Some(0));
                assert!(messages.is_empty());
            }
            other => panic!("expected Connected, got {other:?}"),
        }
    }

    #[test]
    fn on_select_agent_same_index_is_noop() {
        let mut s = make_connected();
        assert!(s.on_select_agent(0));
        // Second call with same index returns false.
        assert!(!s.on_select_agent(0));
    }

    #[test]
    fn on_select_agent_out_of_bounds_is_noop() {
        let mut s = make_connected();
        assert!(!s.on_select_agent(99));
        assert_eq!(s.selected_agent_id(), None);
    }

    #[test]
    fn on_select_agent_not_connected_is_noop() {
        let mut s = SessionState::start("http://x", "tok");
        assert!(!s.on_select_agent(0));
    }

    #[test]
    fn on_messages_populates_messages() {
        let mut s = make_connected();
        s.on_select_agent(0);
        let msgs = vec![ChatMessage {
            id: "m1".to_string(),
            role: "user".to_string(),
            content: serde_json::Value::String("hello".to_string()),
            conversation_id: None,
        }];
        s.on_messages(msgs.clone());
        match &s {
            SessionState::Connected { messages, .. } => {
                assert_eq!(messages.len(), 1);
                assert_eq!(messages[0].id, "m1");
            }
            other => panic!("expected Connected, got {other:?}"),
        }
    }

    #[test]
    fn on_select_agent_clears_previous_messages() {
        let mut s = make_connected();
        // Add a second agent so we can switch.
        if let SessionState::Connected { agents, .. } = &mut s {
            agents.push(AgentInfo {
                id: "agent-2".to_string(),
                name: "Second".to_string(),
                model: None,
                provider: None,
            });
        }
        s.on_select_agent(0);
        s.on_messages(vec![ChatMessage {
            id: "m1".to_string(),
            role: "user".to_string(),
            content: serde_json::Value::String("hi".to_string()),
            conversation_id: None,
        }]);
        // Switch to agent 2 — messages should be cleared.
        assert!(s.on_select_agent(1));
        assert_eq!(s.selected_agent_id(), Some("agent-2"));
        match &s {
            SessionState::Connected { messages, .. } => {
                assert!(messages.is_empty(), "messages should be cleared on agent switch");
            }
            other => panic!("expected Connected, got {other:?}"),
        }
    }

    #[test]
    fn selected_agent_id_none_when_no_selection() {
        let s = make_connected();
        assert_eq!(s.selected_agent_id(), None);
    }

    // ── Input / Send / Stream ───────────────────────────────────────────

    fn make_connected_with_agent_selected() -> SessionState {
        let mut s = make_connected();
        s.on_select_agent(0);
        s
    }

    #[test]
    fn on_send_returns_trimmed_input_and_appends_user_message() {
        let mut s = make_connected_with_agent_selected();
        if let SessionState::Connected { input_buffer, .. } = &mut s {
            *input_buffer = "  hello world  ".to_string();
        }
        let result = s.on_send();
        assert_eq!(result.as_deref(), Some("hello world"));
        if let SessionState::Connected {
            messages,
            input_buffer,
            streaming,
            ..
        } = &s
        {
            assert_eq!(messages.len(), 1);
            assert_eq!(messages[0].role, "user");
            assert_eq!(
                messages[0].content,
                serde_json::Value::String("hello world".to_string())
            );
            assert!(input_buffer.is_empty());
            assert!(*streaming);
        } else {
            panic!("expected Connected");
        }
    }

    #[test]
    fn on_send_noop_when_no_agent_selected() {
        let mut s = make_connected();
        if let SessionState::Connected { input_buffer, .. } = &mut s {
            *input_buffer = "hello".to_string();
        }
        assert_eq!(s.on_send(), None);
    }

    #[test]
    fn on_send_noop_when_empty_buffer() {
        let mut s = make_connected_with_agent_selected();
        assert_eq!(s.on_send(), None);
    }

    #[test]
    fn on_send_noop_when_whitespace_only() {
        let mut s = make_connected_with_agent_selected();
        if let SessionState::Connected { input_buffer, .. } = &mut s {
            *input_buffer = "   ".to_string();
        }
        assert_eq!(s.on_send(), None);
    }

    #[test]
    fn on_send_noop_while_streaming() {
        let mut s = make_connected_with_agent_selected();
        if let SessionState::Connected {
            input_buffer,
            streaming,
            ..
        } = &mut s
        {
            *input_buffer = "hello".to_string();
            *streaming = true;
        }
        assert_eq!(s.on_send(), None);
    }

    #[test]
    fn on_stream_chunk_creates_then_appends_assistant_message() {
        let mut s = make_connected_with_agent_selected();
        if let SessionState::Connected { streaming, .. } = &mut s {
            *streaming = true;
        }
        s.on_stream_chunk("Hello");
        s.on_stream_chunk(", world!");

        if let SessionState::Connected { messages, .. } = &s {
            assert_eq!(messages.len(), 1);
            assert_eq!(messages[0].role, "assistant");
            assert_eq!(
                messages[0].content,
                serde_json::Value::String("Hello, world!".to_string())
            );
        } else {
            panic!("expected Connected");
        }
    }

    #[test]
    fn on_stream_chunk_noop_when_not_streaming() {
        let mut s = make_connected_with_agent_selected();
        s.on_stream_chunk("ignored");
        if let SessionState::Connected { messages, .. } = &s {
            assert!(messages.is_empty());
        }
    }

    #[test]
    fn auto_scroll_true_by_default() {
        let s = make_connected();
        assert!(s.auto_scroll());
    }

    #[test]
    fn disable_auto_scroll_sets_false() {
        let mut s = make_connected();
        s.disable_auto_scroll();
        assert!(!s.auto_scroll());
    }

    #[test]
    fn enable_auto_scroll_restores_true() {
        let mut s = make_connected();
        s.disable_auto_scroll();
        s.enable_auto_scroll();
        assert!(s.auto_scroll());
    }

    #[test]
    fn on_stream_chunk_re_enables_auto_scroll() {
        let mut s = make_connected_with_agent_selected();
        s.disable_auto_scroll();
        assert!(!s.auto_scroll());
        if let SessionState::Connected { input_buffer, .. } = &mut s {
            *input_buffer = "hi".into();
        }
        s.on_send().unwrap();
        s.on_stream_chunk("Hello");
        assert!(s.auto_scroll(), "first chunk should re-enable auto_scroll");
    }

    #[test]
    fn on_stream_done_clears_streaming_flag() {
        let mut s = make_connected_with_agent_selected();
        if let SessionState::Connected { streaming, .. } = &mut s {
            *streaming = true;
        }
        assert!(s.is_streaming());
        s.on_stream_done();
        assert!(!s.is_streaming());
    }

    #[test]
    fn full_send_stream_cycle() {
        let mut s = make_connected_with_agent_selected();
        // Type and send.
        if let SessionState::Connected { input_buffer, .. } = &mut s {
            *input_buffer = "What is Rust?".to_string();
        }
        let input = s.on_send().expect("should send");
        assert_eq!(input, "What is Rust?");
        assert!(s.is_streaming());

        // Stream chunks arrive.
        s.on_stream_chunk("Rust is ");
        s.on_stream_chunk("a systems programming language.");
        s.on_stream_done();

        assert!(!s.is_streaming());
        if let SessionState::Connected { messages, .. } = &s {
            assert_eq!(messages.len(), 2); // user + assistant
            assert_eq!(messages[0].role, "user");
            assert_eq!(messages[1].role, "assistant");
            assert_eq!(
                messages[1].content,
                serde_json::Value::String(
                    "Rust is a systems programming language.".to_string()
                )
            );
        } else {
            panic!("expected Connected");
        }
    }

    // ── Error toast ────────────────────────────────────────────────────

    #[test]
    fn push_error_stores_message() {
        let mut s = make_connected_with_agent_selected();
        s.push_error("stream failed");
        assert_eq!(s.error_toast(), Some("stream failed"));
    }

    #[test]
    fn dismiss_error_clears_toast() {
        let mut s = make_connected_with_agent_selected();
        s.push_error("oops");
        s.dismiss_error();
        assert_eq!(s.error_toast(), None);
    }

    #[test]
    fn push_error_replaces_previous() {
        let mut s = make_connected_with_agent_selected();
        s.push_error("first");
        s.push_error("second");
        assert_eq!(s.error_toast(), Some("second"));
    }

    #[test]
    fn error_toast_none_when_no_error() {
        let s = make_connected_with_agent_selected();
        assert_eq!(s.error_toast(), None);
    }

    #[test]
    fn push_error_also_clears_streaming() {
        let mut s = make_connected_with_agent_selected();
        // Start a stream, then an error arrives.
        if let SessionState::Connected { input_buffer, .. } = &mut s {
            *input_buffer = "hello".to_string();
        }
        let _ = s.on_send();
        assert!(s.is_streaming());
        s.push_error("connection lost");
        assert!(!s.is_streaming(), "streaming should be cleared on error");
        assert_eq!(s.error_toast(), Some("connection lost"));
    }

    // ── Conversation ID ────────────────────────────────────────────────

    #[test]
    fn conversation_id_none_initially() {
        let s = make_connected_with_agent_selected();
        assert_eq!(s.conversation_id(), None);
    }

    #[test]
    fn on_conversation_id_stores_id() {
        let mut s = make_connected_with_agent_selected();
        s.on_conversation_id("conv-abc-123");
        assert_eq!(s.conversation_id(), Some("conv-abc-123"));
    }

    #[test]
    fn on_conversation_id_replaces_previous() {
        let mut s = make_connected_with_agent_selected();
        s.on_conversation_id("conv-1");
        s.on_conversation_id("conv-2");
        assert_eq!(s.conversation_id(), Some("conv-2"));
    }

    #[test]
    fn select_agent_clears_conversation_id() {
        let mut s = make_connected_with_agent_selected();
        // Add a second agent so we can actually switch.
        if let SessionState::Connected { agents, .. } = &mut s {
            agents.push(AgentInfo {
                id: "agent-2".to_string(),
                name: "Second Agent".to_string(),
                model: None,
                provider: None,
            });
        }
        s.on_conversation_id("conv-old");
        assert!(s.on_select_agent(1)); // switch to agent-2
        assert_eq!(s.conversation_id(), None);
    }

    // ── Reasoning stream ───────────────────────────────────────────────

    #[test]
    fn on_stream_reasoning_creates_reasoning_message() {
        let mut s = make_connected_with_agent_selected();
        if let SessionState::Connected { input_buffer, .. } = &mut s {
            *input_buffer = "explain".to_string();
        }
        let _ = s.on_send();

        s.on_stream_reasoning("Let me think");
        s.on_stream_reasoning(" about this.");

        if let SessionState::Connected { messages, .. } = &s {
            // user + reasoning
            assert_eq!(messages.len(), 2);
            assert_eq!(messages[1].role, "reasoning");
            assert_eq!(
                messages[1].content,
                serde_json::Value::String("Let me think about this.".to_string())
            );
        } else {
            panic!("expected Connected");
        }
    }

    #[test]
    fn reasoning_then_assistant_are_separate_messages() {
        let mut s = make_connected_with_agent_selected();
        if let SessionState::Connected { input_buffer, .. } = &mut s {
            *input_buffer = "hello".to_string();
        }
        let _ = s.on_send();

        s.on_stream_reasoning("thinking...");
        s.on_stream_chunk("The answer is 42.");

        if let SessionState::Connected { messages, .. } = &s {
            assert_eq!(messages.len(), 3); // user + reasoning + assistant
            assert_eq!(messages[1].role, "reasoning");
            assert_eq!(messages[2].role, "assistant");
        } else {
            panic!("expected Connected");
        }
    }

    // ── Tool call stream ───────────────────────────────────────────────

    #[test]
    fn on_stream_tool_call_creates_tool_call_message() {
        let mut s = make_connected_with_agent_selected();
        if let SessionState::Connected { input_buffer, .. } = &mut s {
            *input_buffer = "search".to_string();
        }
        let _ = s.on_send();

        s.on_stream_tool_call("tc-1", "web_search", r#"{"query":"rust"}"#);

        if let SessionState::Connected { messages, .. } = &s {
            assert_eq!(messages.len(), 2); // user + tool_call
            assert_eq!(messages[1].role, "tool_call");
            let tc = &messages[1].content;
            assert_eq!(tc["name"], "web_search");
            assert_eq!(tc["id"], "tc-1");
            assert_eq!(tc["arguments"], r#"{"query":"rust"}"#);
        } else {
            panic!("expected Connected");
        }
    }

    #[test]
    fn multiple_tool_calls_are_separate_messages() {
        let mut s = make_connected_with_agent_selected();
        if let SessionState::Connected { input_buffer, .. } = &mut s {
            *input_buffer = "do stuff".to_string();
        }
        let _ = s.on_send();

        s.on_stream_tool_call("tc-1", "read_file", r#"{"path":"a.rs"}"#);
        s.on_stream_tool_call("tc-2", "write_file", r#"{"path":"b.rs"}"#);

        if let SessionState::Connected { messages, .. } = &s {
            assert_eq!(messages.len(), 3); // user + 2 tool_calls
            assert_eq!(messages[1].content["name"], "read_file");
            assert_eq!(messages[2].content["name"], "write_file");
        } else {
            panic!("expected Connected");
        }
    }

    // ── Usage / finish reason ──────────────────────────────────────────

    #[test]
    fn on_usage_stores_stats() {
        let mut s = make_connected_with_agent_selected();
        if let SessionState::Connected { input_buffer, .. } = &mut s {
            *input_buffer = "hi".to_string();
        }
        let _ = s.on_send();
        s.on_usage(100, 50, Some("gpt-4o"));
        assert_eq!(s.last_usage(), Some((100, 50, Some("gpt-4o"))));
    }

    #[test]
    fn on_finish_reason_stores_reason() {
        let mut s = make_connected_with_agent_selected();
        if let SessionState::Connected { input_buffer, .. } = &mut s {
            *input_buffer = "hi".to_string();
        }
        let _ = s.on_send();
        s.on_finish_reason("stop");
        assert_eq!(s.last_finish_reason(), Some("stop"));
    }

    #[test]
    fn usage_and_finish_reason_none_initially() {
        let s = make_connected_with_agent_selected();
        assert_eq!(s.last_usage(), None);
        assert_eq!(s.last_finish_reason(), None);
    }

    #[test]
    fn on_send_clears_usage_and_finish_reason() {
        let mut s = make_connected_with_agent_selected();
        if let SessionState::Connected { input_buffer, .. } = &mut s {
            *input_buffer = "first".to_string();
        }
        let _ = s.on_send();
        s.on_usage(10, 5, None);
        s.on_finish_reason("stop");
        s.on_stream_done();

        // Send again — usage/finish should reset.
        if let SessionState::Connected { input_buffer, .. } = &mut s {
            *input_buffer = "second".to_string();
        }
        let _ = s.on_send();
        assert_eq!(s.last_usage(), None);
        assert_eq!(s.last_finish_reason(), None);
    }

    // ── Conversation management tests ───────────────────────────────

    fn test_conversations() -> Vec<crate::api::ConversationInfo> {
        vec![
            crate::api::ConversationInfo {
                id: "conv-1".to_string(),
                title: "First chat".to_string(),
                message_count: 3,
                updated_at: "2025-01-01T00:00:00Z".to_string(),
            },
            crate::api::ConversationInfo {
                id: "conv-2".to_string(),
                title: "Second chat".to_string(),
                message_count: 0,
                updated_at: "2025-01-02T00:00:00Z".to_string(),
            },
        ]
    }

    #[test]
    fn on_conversations_stores_list() {
        let mut s = make_connected_with_agent_selected();
        assert!(s.conversations().is_empty());
        s.on_conversations(test_conversations());
        assert_eq!(s.conversations().len(), 2);
        assert_eq!(s.conversations()[0].id, "conv-1");
    }

    #[test]
    fn on_select_conversation_returns_true_when_changed() {
        let mut s = make_connected_with_agent_selected();
        s.on_conversations(test_conversations());
        assert!(s.on_select_conversation(0));
        assert_eq!(s.selected_conversation(), Some(0));
    }

    #[test]
    fn on_select_conversation_returns_false_when_same() {
        let mut s = make_connected_with_agent_selected();
        s.on_conversations(test_conversations());
        s.on_select_conversation(0);
        assert!(!s.on_select_conversation(0));
    }

    #[test]
    fn on_select_conversation_clears_messages() {
        let mut s = make_connected_with_agent_selected();
        s.on_messages(vec![ChatMessage {
            id: "m1".into(),
            role: "user".into(),
            content: serde_json::Value::String("hi".into()),
            conversation_id: None,
        }]);
        s.on_conversations(test_conversations());
        s.on_select_conversation(1);
        if let SessionState::Connected { messages, .. } = &s {
            assert!(messages.is_empty());
        } else {
            panic!("not connected");
        }
    }

    #[test]
    fn on_select_conversation_sets_conversation_id() {
        let mut s = make_connected_with_agent_selected();
        s.on_conversations(test_conversations());
        s.on_select_conversation(1);
        assert_eq!(s.conversation_id(), Some("conv-2"));
    }

    #[test]
    fn on_select_conversation_out_of_bounds_is_noop() {
        let mut s = make_connected_with_agent_selected();
        s.on_conversations(test_conversations());
        assert!(!s.on_select_conversation(99));
        assert_eq!(s.selected_conversation(), None);
    }

    #[test]
    fn on_new_conversation_clears_state() {
        let mut s = make_connected_with_agent_selected();
        s.on_conversations(test_conversations());
        s.on_select_conversation(0);
        s.on_conversation_id("conv-1");
        s.on_messages(vec![ChatMessage {
            id: "m1".into(),
            role: "user".into(),
            content: serde_json::Value::String("hi".into()),
            conversation_id: None,
        }]);
        s.on_new_conversation();
        assert_eq!(s.conversation_id(), None);
        assert_eq!(s.selected_conversation(), None);
        if let SessionState::Connected { messages, .. } = &s {
            assert!(messages.is_empty());
        }
    }

    #[test]
    fn on_conversation_deleted_removes_entry() {
        let mut s = make_connected_with_agent_selected();
        s.on_conversations(test_conversations()); // 2 conversations
        assert_eq!(s.conversations().len(), 2);
        s.on_conversation_deleted(0);
        assert_eq!(s.conversations().len(), 1);
    }

    #[test]
    fn on_conversation_deleted_out_of_bounds_is_noop() {
        let mut s = make_connected_with_agent_selected();
        s.on_conversations(test_conversations());
        s.on_conversation_deleted(99);
        assert_eq!(s.conversations().len(), 2);
    }

    #[test]
    fn on_conversation_deleted_clears_state_when_active() {
        let mut s = make_connected_with_agent_selected();
        s.on_conversations(test_conversations());
        s.on_select_conversation(0);
        s.on_conversation_id("conv-1");
        s.on_messages(vec![ChatMessage {
            id: "m1".into(),
            role: "user".into(),
            content: serde_json::Value::String("hi".into()),
            conversation_id: None,
        }]);
        // Delete the active conversation
        s.on_conversation_deleted(0);
        assert_eq!(s.selected_conversation(), None);
        assert_eq!(s.conversation_id(), None);
        if let SessionState::Connected { messages, .. } = &s {
            assert!(messages.is_empty());
        }
    }

    #[test]
    fn on_conversation_deleted_shifts_selection_down() {
        let mut s = make_connected_with_agent_selected();
        s.on_conversations(test_conversations());
        // Select second conversation (idx 1)
        s.on_select_conversation(1);
        assert_eq!(s.selected_conversation(), Some(1));
        // Delete first conversation (idx 0) — selection should shift to 0
        s.on_conversation_deleted(0);
        assert_eq!(s.selected_conversation(), Some(0));
        assert_eq!(s.conversations().len(), 1);
    }

    #[test]
    fn on_select_agent_clears_conversations() {
        let mut s = make_connected();
        s.on_select_agent(0);
        s.on_conversations(test_conversations());
        s.on_select_conversation(0);
        // Now switch agent — should clear conversations.
        s.on_select_agent(1);
        assert!(s.conversations().is_empty());
        assert_eq!(s.selected_conversation(), None);
    }

    // ── Pagination tests ────────────────────────────────────────────

    #[test]
    fn on_messages_paged_stores_has_more() {
        let mut s = make_connected_with_agent_selected();
        let msgs = vec![ChatMessage {
            id: "m1".into(),
            role: "user".into(),
            content: serde_json::Value::String("hi".into()),
            conversation_id: None,
        }];
        s.on_messages_paged(msgs, true);
        assert!(s.has_more_messages());
        assert_eq!(s.message_count(), 1);
    }

    #[test]
    fn on_messages_paged_no_more() {
        let mut s = make_connected_with_agent_selected();
        s.on_messages_paged(vec![], false);
        assert!(!s.has_more_messages());
        assert_eq!(s.message_count(), 0);
    }

    #[test]
    fn on_prepend_messages_adds_to_front() {
        let mut s = make_connected_with_agent_selected();
        let recent = vec![ChatMessage {
            id: "m2".into(),
            role: "assistant".into(),
            content: serde_json::Value::String("hello".into()),
            conversation_id: None,
        }];
        s.on_messages_paged(recent, true);

        let older = vec![ChatMessage {
            id: "m1".into(),
            role: "user".into(),
            content: serde_json::Value::String("hi".into()),
            conversation_id: None,
        }];
        s.on_prepend_messages(older, false);

        assert_eq!(s.message_count(), 2);
        assert!(!s.has_more_messages());
        // The older message should be first.
        if let SessionState::Connected { messages, .. } = &s {
            assert_eq!(messages[0].id, "m1");
            assert_eq!(messages[1].id, "m2");
        }
    }

    #[test]
    fn parse_messages_paged_with_has_more() {
        let body = r#"{"messages":[
            {"id":"m1","role":"user","content":"hi","conversation_id":null}
        ],"has_more":true,"query":""}"#;
        let (msgs, has_more) = crate::api::parse_messages_paged(200, body).unwrap();
        assert_eq!(msgs.len(), 1);
        assert!(has_more);
    }

    #[test]
    fn parse_messages_paged_without_has_more() {
        let body = r#"{"messages":[],"query":""}"#;
        let (msgs, has_more) = crate::api::parse_messages_paged(200, body).unwrap();
        assert!(msgs.is_empty());
        assert!(!has_more); // defaults to false when missing
    }

    // ── Palette (M15) ──────────────────────────────────────────────

    fn connected_session() -> SessionState {
        let mut s = SessionState::start("http://localhost:8080", "tok");
        s.on_health(test_health());
        s.on_agents(test_agents());
        s
    }

    #[test]
    fn palette_starts_closed() {
        let s = connected_session();
        assert!(!s.is_palette_open());
        assert!(s.selected_palette_cmd().is_none());
    }

    #[test]
    fn palette_open_and_close() {
        let mut s = connected_session();
        s.open_palette("");
        assert!(s.is_palette_open());
        s.close_palette();
        assert!(!s.is_palette_open());
    }

    #[test]
    fn palette_open_preserves_initial_input() {
        let mut s = connected_session();
        s.open_palette("hel");
        if let SessionState::Connected {
            palette_input,
            palette_selection,
            ..
        } = &s
        {
            assert_eq!(palette_input, "hel");
            assert_eq!(*palette_selection, 0);
        } else {
            panic!("not connected");
        }
    }

    #[test]
    fn palette_close_resets_input_and_selection() {
        let mut s = connected_session();
        s.open_palette("mem");
        s.move_palette_selection(1);
        s.close_palette();
        if let SessionState::Connected {
            palette_input,
            palette_selection,
            palette_open,
            ..
        } = &s
        {
            assert!(!*palette_open);
            assert!(palette_input.is_empty());
            assert_eq!(*palette_selection, 0);
        } else {
            panic!("not connected");
        }
    }

    #[test]
    fn palette_set_input_resets_selection() {
        let mut s = connected_session();
        s.open_palette("");
        s.move_palette_selection(3);
        s.set_palette_input("hel"); // typing new query — selection back to 0
        if let SessionState::Connected {
            palette_input,
            palette_selection,
            ..
        } = &s
        {
            assert_eq!(palette_input, "hel");
            assert_eq!(*palette_selection, 0);
        } else {
            panic!("not connected");
        }
    }

    #[test]
    fn palette_move_selection_clamps_to_bounds() {
        let mut s = connected_session();
        s.open_palette(""); // empty query → all commands
        s.move_palette_selection(-1); // can't go below 0
        if let SessionState::Connected {
            palette_selection, ..
        } = &s
        {
            assert_eq!(*palette_selection, 0);
        } else {
            panic!("not connected");
        }

        // Move down past end should clamp.
        for _ in 0..100 {
            s.move_palette_selection(1);
        }
        let filtered_count = crate::palette::fuzzy_filter("").len();
        if let SessionState::Connected {
            palette_selection, ..
        } = &s
        {
            assert_eq!(*palette_selection, filtered_count - 1);
        } else {
            panic!("not connected");
        }
    }

    #[test]
    fn palette_selected_cmd_returns_first_match() {
        let mut s = connected_session();
        s.open_palette("help");
        // `help` is an exact trigger — first filtered entry should be Help.
        assert_eq!(
            s.selected_palette_cmd(),
            Some(crate::palette::PaletteCmd::Help)
        );
    }

    #[test]
    fn palette_selected_cmd_respects_selection_index() {
        let mut s = connected_session();
        s.open_palette(""); // all entries
        s.move_palette_selection(1);
        // The second entry's trigger should be the one returned.
        let filtered = crate::palette::fuzzy_filter("");
        let expected = crate::palette::parse_palette_input(filtered[1].def.trigger);
        assert_eq!(s.selected_palette_cmd(), Some(expected));
    }

    #[test]
    fn palette_selected_cmd_none_when_closed() {
        let s = connected_session();
        assert!(s.selected_palette_cmd().is_none());
    }

    #[test]
    fn palette_selected_cmd_none_when_no_matches() {
        let mut s = connected_session();
        s.open_palette("zzznonexistentquery");
        assert!(s.selected_palette_cmd().is_none());
    }

    #[test]
    fn palette_methods_noop_when_not_connected() {
        let mut s = SessionState::start("http://localhost:8080", "tok");
        // Still in Connecting — all palette methods should be no-ops.
        s.open_palette("foo");
        assert!(!s.is_palette_open());
        s.set_palette_input("bar");
        s.move_palette_selection(5);
        s.close_palette();
        assert!(s.selected_palette_cmd().is_none());
    }

    #[test]
    fn clear_timeline_local_clears_messages_only() {
        let mut s = connected_session();
        s.on_select_agent(0);
        s.on_messages(vec![ChatMessage {
            id: "m1".into(),
            role: "user".into(),
            content: serde_json::Value::String("hi".into()),
            conversation_id: Some("c1".into()),
        }]);
        // Set a conversation_id to verify it's NOT cleared.
        if let SessionState::Connected {
            conversation_id, ..
        } = &mut s
        {
            *conversation_id = Some("c1".into());
        }
        s.clear_timeline_local();
        if let SessionState::Connected {
            messages,
            conversation_id,
            ..
        } = &s
        {
            assert!(messages.is_empty());
            assert_eq!(conversation_id.as_deref(), Some("c1")); // preserved
        } else {
            panic!("not connected");
        }
    }

    #[test]
    fn last_assistant_content_finds_most_recent() {
        let mut s = connected_session();
        s.on_select_agent(0);
        s.on_messages(vec![
            ChatMessage {
                id: "m1".into(),
                role: "user".into(),
                content: serde_json::Value::String("q1".into()),
                conversation_id: None,
            },
            ChatMessage {
                id: "m2".into(),
                role: "assistant".into(),
                content: serde_json::Value::String("a1".into()),
                conversation_id: None,
            },
            ChatMessage {
                id: "m3".into(),
                role: "user".into(),
                content: serde_json::Value::String("q2".into()),
                conversation_id: None,
            },
            ChatMessage {
                id: "m4".into(),
                role: "assistant".into(),
                content: serde_json::Value::String("a2 final".into()),
                conversation_id: None,
            },
        ]);
        assert_eq!(s.last_assistant_content().as_deref(), Some("a2 final"));
    }

    #[test]
    fn last_assistant_content_none_when_no_assistant_messages() {
        let mut s = connected_session();
        s.on_select_agent(0);
        s.on_messages(vec![ChatMessage {
            id: "m1".into(),
            role: "user".into(),
            content: serde_json::Value::String("hi".into()),
            conversation_id: None,
        }]);
        assert!(s.last_assistant_content().is_none());
    }

    // ── Memory overlay (M16) ───────────────────────────────────────

    fn test_blocks() -> Vec<crate::api::MemoryBlock> {
        vec![
            crate::api::MemoryBlock {
                label: "human".into(),
                value: "User loves Rust".into(),
                description: Some("User info".into()),
                tier: Some("short".into()),
            },
            crate::api::MemoryBlock {
                label: "project".into(),
                value: "CADE project".into(),
                description: None,
                tier: None,
            },
        ]
    }

    #[test]
    fn memory_starts_closed() {
        let s = connected_session();
        assert!(!s.is_memory_open());
    }

    #[test]
    fn open_memory_sets_flags() {
        let mut s = connected_session();
        s.open_memory_overlay();
        assert!(s.is_memory_open());
        if let SessionState::Connected {
            memory_loading,
            memory_error,
            ..
        } = &s
        {
            assert!(*memory_loading);
            assert!(memory_error.is_none());
        } else {
            panic!("not connected");
        }
    }

    #[test]
    fn close_memory_resets_transient_flags() {
        let mut s = connected_session();
        s.open_memory_overlay();
        s.on_memory_error("boom");
        assert_eq!(
            match &s {
                SessionState::Connected { memory_error, .. } => memory_error.clone(),
                _ => None,
            },
            Some("boom".to_string())
        );
        s.close_memory_overlay();
        assert!(!s.is_memory_open());
        if let SessionState::Connected {
            memory_error,
            memory_saving,
            ..
        } = &s
        {
            assert!(memory_error.is_none());
            assert!(!*memory_saving);
        } else {
            panic!("not connected");
        }
    }

    #[test]
    fn memory_loaded_seeds_edit_buffer_with_first_block() {
        let mut s = connected_session();
        s.open_memory_overlay();
        s.on_memory_loaded(test_blocks());
        if let SessionState::Connected {
            memory_blocks,
            memory_selection,
            memory_edit_buffer,
            memory_loading,
            ..
        } = &s
        {
            assert_eq!(memory_blocks.len(), 2);
            assert_eq!(*memory_selection, 0);
            assert_eq!(memory_edit_buffer, "User loves Rust");
            assert!(!*memory_loading);
        } else {
            panic!("not connected");
        }
    }

    #[test]
    fn memory_loaded_with_empty_list_keeps_empty_buffer() {
        let mut s = connected_session();
        s.open_memory_overlay();
        s.on_memory_loaded(Vec::new());
        if let SessionState::Connected {
            memory_blocks,
            memory_edit_buffer,
            memory_loading,
            ..
        } = &s
        {
            assert!(memory_blocks.is_empty());
            assert!(memory_edit_buffer.is_empty());
            assert!(!*memory_loading);
        } else {
            panic!("not connected");
        }
    }

    #[test]
    fn memory_error_clears_loading_and_saving() {
        let mut s = connected_session();
        s.open_memory_overlay();
        s.on_memory_save_start();
        s.on_memory_error("nope");
        if let SessionState::Connected {
            memory_loading,
            memory_saving,
            memory_error,
            ..
        } = &s
        {
            assert!(!*memory_loading);
            assert!(!*memory_saving);
            assert_eq!(memory_error.as_deref(), Some("nope"));
        } else {
            panic!("not connected");
        }
    }

    #[test]
    fn select_memory_block_updates_buffer() {
        let mut s = connected_session();
        s.open_memory_overlay();
        s.on_memory_loaded(test_blocks());
        // Edit the buffer — this simulates the user typing.
        s.set_memory_edit_buffer("unsaved edit");
        let changed = s.select_memory_block(1);
        assert!(changed);
        if let SessionState::Connected {
            memory_selection,
            memory_edit_buffer,
            ..
        } = &s
        {
            assert_eq!(*memory_selection, 1);
            // Buffer is reset to the new block's value — unsaved edit is lost.
            assert_eq!(memory_edit_buffer, "CADE project");
        } else {
            panic!("not connected");
        }
    }

    #[test]
    fn select_memory_block_same_index_returns_false() {
        let mut s = connected_session();
        s.open_memory_overlay();
        s.on_memory_loaded(test_blocks());
        assert!(!s.select_memory_block(0));
    }

    #[test]
    fn select_memory_block_out_of_bounds_returns_false() {
        let mut s = connected_session();
        s.open_memory_overlay();
        s.on_memory_loaded(test_blocks());
        assert!(!s.select_memory_block(99));
    }

    #[test]
    fn memory_save_ok_persists_buffer_into_block() {
        let mut s = connected_session();
        s.open_memory_overlay();
        s.on_memory_loaded(test_blocks());
        s.set_memory_edit_buffer("User loves Rust AND Python");
        s.on_memory_save_start();
        s.on_memory_save_ok();
        if let SessionState::Connected {
            memory_blocks,
            memory_saving,
            memory_error,
            ..
        } = &s
        {
            assert_eq!(memory_blocks[0].value, "User loves Rust AND Python");
            assert!(!*memory_saving);
            assert!(memory_error.is_none());
        } else {
            panic!("not connected");
        }
    }

    #[test]
    fn memory_selected_label_value_returns_current() {
        let mut s = connected_session();
        s.open_memory_overlay();
        s.on_memory_loaded(test_blocks());
        s.set_memory_edit_buffer("new content");
        assert_eq!(
            s.memory_selected_label_value(),
            Some(("human".to_string(), "new content".to_string()))
        );
    }

    #[test]
    fn memory_selected_label_value_none_when_closed() {
        let mut s = connected_session();
        s.on_memory_loaded(test_blocks()); // noop because overlay closed
        assert!(s.memory_selected_label_value().is_none());
    }

    #[test]
    fn memory_methods_noop_when_not_connected() {
        let mut s = SessionState::start("http://localhost:8080", "tok");
        // Still in Connecting — all memory methods should be no-ops.
        s.open_memory_overlay();
        assert!(!s.is_memory_open());
        s.on_memory_loaded(test_blocks());
        s.on_memory_error("nope");
        s.set_memory_edit_buffer("x");
        s.on_memory_save_start();
        s.on_memory_save_ok();
        s.close_memory_overlay();
        assert!(s.memory_selected_label_value().is_none());
        assert!(!s.select_memory_block(0));
    }

    // ── is_memory_dirty / memory_save_notice ──────────────────────

    #[test]
    fn is_memory_dirty_false_when_closed() {
        let s = connected_session();
        assert!(!s.is_memory_dirty());
    }

    #[test]
    fn is_memory_dirty_false_right_after_load() {
        let mut s = connected_session();
        s.open_memory_overlay();
        s.on_memory_loaded(test_blocks());
        assert!(!s.is_memory_dirty(),
            "fresh load should have buffer == block value, not dirty");
    }

    #[test]
    fn is_memory_dirty_true_after_edit() {
        let mut s = connected_session();
        s.open_memory_overlay();
        s.on_memory_loaded(test_blocks());
        s.set_memory_edit_buffer("something different");
        assert!(s.is_memory_dirty());
    }

    #[test]
    fn is_memory_dirty_false_after_save() {
        let mut s = connected_session();
        s.open_memory_overlay();
        s.on_memory_loaded(test_blocks());
        s.set_memory_edit_buffer("edited");
        assert!(s.is_memory_dirty());
        s.on_memory_save_start();
        s.on_memory_save_ok();
        assert!(!s.is_memory_dirty(),
            "after save the block's saved value == buffer, no longer dirty");
    }

    #[test]
    fn is_memory_dirty_false_after_selecting_different_block() {
        let mut s = connected_session();
        s.open_memory_overlay();
        s.on_memory_loaded(test_blocks());
        s.set_memory_edit_buffer("dirty here");
        assert!(s.is_memory_dirty());
        // Selecting another block seeds the buffer with its saved value,
        // so dirty should flip back to false.
        assert!(s.select_memory_block(1));
        assert!(!s.is_memory_dirty());
    }

    #[test]
    fn memory_save_notice_none_by_default() {
        let s = connected_session();
        assert!(s.memory_save_notice().is_none());
    }

    #[test]
    fn memory_save_notice_set_on_save_ok() {
        let mut s = connected_session();
        s.open_memory_overlay();
        s.on_memory_loaded(test_blocks());
        s.set_memory_edit_buffer("new val");
        s.on_memory_save_start();
        s.on_memory_save_ok();
        assert_eq!(s.memory_save_notice(), Some("Saved /human"));
    }

    #[test]
    fn memory_save_notice_cleared_on_select() {
        let mut s = connected_session();
        s.open_memory_overlay();
        s.on_memory_loaded(test_blocks());
        s.on_memory_save_start();
        s.on_memory_save_ok();
        assert!(s.memory_save_notice().is_some());
        assert!(s.select_memory_block(1));
        assert!(s.memory_save_notice().is_none());
    }

    #[test]
    fn memory_save_notice_cleared_on_close() {
        let mut s = connected_session();
        s.open_memory_overlay();
        s.on_memory_loaded(test_blocks());
        s.on_memory_save_start();
        s.on_memory_save_ok();
        assert!(s.memory_save_notice().is_some());
        s.close_memory_overlay();
        assert!(s.memory_save_notice().is_none());
    }

    #[test]
    fn memory_save_notice_cleared_on_error() {
        let mut s = connected_session();
        s.open_memory_overlay();
        s.on_memory_loaded(test_blocks());
        s.on_memory_save_start();
        s.on_memory_save_ok();
        assert!(s.memory_save_notice().is_some());
        s.on_memory_error("boom");
        assert!(s.memory_save_notice().is_none());
    }

    #[test]
    fn memory_save_notice_cleared_on_save_start() {
        let mut s = connected_session();
        s.open_memory_overlay();
        s.on_memory_loaded(test_blocks());
        s.on_memory_save_start();
        s.on_memory_save_ok();
        assert!(s.memory_save_notice().is_some());
        // A second save begins — should clear the previous notice.
        s.on_memory_save_start();
        assert!(s.memory_save_notice().is_none());
    }

    // ── refresh_agents ─────────────────────────────────────────────

    #[test]
    fn refresh_agents_preserves_selection_by_id() {
        let mut s = connected_session();
        s.on_select_agent(1); // pick second agent
        let selected_id = s.selected_agent_id().unwrap().to_string();

        // Simulate server returning a reordered list with an extra agent.
        let mut new_agents = test_agents();
        new_agents.reverse();
        new_agents.push(AgentInfo {
            id: "agent-3".into(),
            name: "New Agent".into(),
            model: None,
            provider: None,
        });
        s.refresh_agents(new_agents);
        // Selection should follow the id, so it's still the same agent.
        assert_eq!(s.selected_agent_id(), Some(selected_id.as_str()));
    }

    #[test]
    fn refresh_agents_drops_selection_when_agent_removed() {
        let mut s = connected_session();
        s.on_select_agent(0);
        let new_agents = vec![AgentInfo {
            id: "different-agent".into(),
            name: "Different".into(),
            model: None,
            provider: None,
        }];
        s.refresh_agents(new_agents);
        assert!(s.selected_agent_id().is_none());
    }

    #[test]
    fn refresh_agents_noop_when_not_connected() {
        let mut s = SessionState::start("http://localhost:8080", "tok");
        s.refresh_agents(test_agents());
        // Still Connecting — no panic, no transition.
        assert!(!s.is_connected());
    }

    // ── Checkpoints overlay (M17) ──────────────────────────────────

    fn test_checkpoint_rows() -> Vec<crate::api::CheckpointRow> {
        vec![
            crate::api::CheckpointRow {
                id: "cp-1".into(),
                agent_id: "agent-1".into(),
                conversation_id: None,
                branch_id: "main".into(),
                label: Some("before-refactor".into()),
                description: None,
                created_at: 1_700_000_000,
                git_stash_ref: Some("stash@{0}".into()),
                git_commit_hash: None,
                parent_id: None,
            },
            crate::api::CheckpointRow {
                id: "cp-2".into(),
                agent_id: "agent-1".into(),
                conversation_id: None,
                branch_id: "main".into(),
                label: None,
                description: Some("auto-save".into()),
                created_at: 1_700_001_000,
                git_stash_ref: None,
                git_commit_hash: None,
                parent_id: Some("cp-1".into()),
            },
        ]
    }

    #[test]
    fn checkpoints_starts_closed() {
        let s = connected_session();
        assert!(!s.is_checkpoints_open());
        assert!(s.checkpoints_snapshot().is_empty());
    }

    #[test]
    fn open_checkpoints_sets_loading_and_clears_error() {
        let mut s = connected_session();
        s.on_checkpoints_error("stale");
        s.open_checkpoints_overlay();
        assert!(s.is_checkpoints_open());
        match &s {
            SessionState::Connected {
                checkpoints_loading,
                checkpoints_error,
                ..
            } => {
                assert!(*checkpoints_loading);
                assert!(checkpoints_error.is_none());
            }
            _ => panic!("not connected"),
        }
    }

    #[test]
    fn checkpoints_loaded_populates_list() {
        let mut s = connected_session();
        s.open_checkpoints_overlay();
        s.on_checkpoints_loaded(test_checkpoint_rows());
        assert_eq!(s.checkpoints_snapshot().len(), 2);
        match &s {
            SessionState::Connected {
                checkpoints_loading,
                ..
            } => assert!(!*checkpoints_loading),
            _ => panic!(),
        }
    }

    #[test]
    fn checkpoints_error_clears_loading_and_busy() {
        let mut s = connected_session();
        s.open_checkpoints_overlay();
        s.on_checkpoints_action_start();
        s.on_checkpoints_error("network down");
        match &s {
            SessionState::Connected {
                checkpoints_loading,
                checkpoints_busy,
                checkpoints_error,
                ..
            } => {
                assert!(!*checkpoints_loading);
                assert!(!*checkpoints_busy);
                assert_eq!(checkpoints_error.as_deref(), Some("network down"));
            }
            _ => panic!(),
        }
    }

    #[test]
    fn checkpoints_action_ok_sets_notice() {
        let mut s = connected_session();
        s.open_checkpoints_overlay();
        s.on_checkpoints_action_start();
        s.on_checkpoints_action_ok("Restored cp-1");
        assert_eq!(s.checkpoints_notice(), Some("Restored cp-1"));
    }

    #[test]
    fn checkpoints_notice_cleared_on_new_action() {
        let mut s = connected_session();
        s.on_checkpoints_action_ok("Done");
        s.on_checkpoints_action_start();
        assert!(s.checkpoints_notice().is_none());
    }

    #[test]
    fn checkpoints_notice_cleared_on_close() {
        let mut s = connected_session();
        s.on_checkpoints_action_ok("Done");
        s.close_checkpoints_overlay();
        assert!(s.checkpoints_notice().is_none());
    }

    #[test]
    fn checkpoints_methods_noop_when_not_connected() {
        let mut s = SessionState::start("http://localhost:8080", "tok");
        s.open_checkpoints_overlay();
        assert!(!s.is_checkpoints_open());
        s.on_checkpoints_loaded(test_checkpoint_rows());
        assert!(s.checkpoints_snapshot().is_empty());
        s.on_checkpoints_error("x");
        s.on_checkpoints_action_start();
        s.on_checkpoints_action_ok("x");
        s.close_checkpoints_overlay();
    }

    // ── Artifacts overlay (M17) ────────────────────────────────────

    fn test_artifact_rows() -> Vec<crate::api::ArtifactInfo> {
        vec![
            crate::api::ArtifactInfo {
                id: "art-1".into(),
                kind: "log".into(),
                content_type: "text/plain".into(),
                size_bytes: 42,
                created_at: 1_700_000_000,
                run_id: Some("run-1".into()),
            },
            crate::api::ArtifactInfo {
                id: "art-2".into(),
                kind: "diff".into(),
                content_type: "text/x-diff".into(),
                size_bytes: 128,
                created_at: 1_700_001_000,
                run_id: None,
            },
        ]
    }

    fn test_artifact_detail(id: &str) -> crate::api::ArtifactDetail {
        crate::api::ArtifactDetail {
            id: id.into(),
            kind: "log".into(),
            content_type: "text/plain".into(),
            data_text: Some("hello".into()),
            metadata: serde_json::json!({}),
            size_bytes: 5,
            created_at: 1_700_000_000,
        }
    }

    #[test]
    fn artifacts_starts_closed() {
        let s = connected_session();
        assert!(!s.is_artifacts_open());
        assert!(s.artifacts_snapshot().is_empty());
        assert!(s.artifact_detail().is_none());
    }

    #[test]
    fn open_artifacts_clears_selection() {
        let mut s = connected_session();
        s.open_artifacts_overlay();
        s.on_artifacts_loaded(test_artifact_rows());
        s.select_artifact(0);
        s.on_artifact_detail_loaded(test_artifact_detail("art-1"));
        // Reopening (e.g. via palette) should reset selection.
        s.open_artifacts_overlay();
        assert!(s.selected_artifact_id().is_none());
        assert!(s.artifact_detail().is_none());
    }

    #[test]
    fn artifacts_loaded_populates_list() {
        let mut s = connected_session();
        s.open_artifacts_overlay();
        s.on_artifacts_loaded(test_artifact_rows());
        assert_eq!(s.artifacts_snapshot().len(), 2);
    }

    #[test]
    fn select_artifact_returns_id_and_sets_busy() {
        let mut s = connected_session();
        s.open_artifacts_overlay();
        s.on_artifacts_loaded(test_artifact_rows());
        let id = s.select_artifact(1);
        assert_eq!(id.as_deref(), Some("art-2"));
        assert_eq!(s.selected_artifact_id().as_deref(), Some("art-2"));
        match &s {
            SessionState::Connected {
                artifacts_busy, ..
            } => assert!(*artifacts_busy),
            _ => panic!(),
        }
    }

    #[test]
    fn select_artifact_out_of_bounds_returns_none() {
        let mut s = connected_session();
        s.open_artifacts_overlay();
        s.on_artifacts_loaded(test_artifact_rows());
        assert!(s.select_artifact(99).is_none());
        assert!(s.selected_artifact_id().is_none());
    }

    #[test]
    fn artifact_detail_loaded_clears_busy() {
        let mut s = connected_session();
        s.open_artifacts_overlay();
        s.on_artifacts_loaded(test_artifact_rows());
        s.select_artifact(0);
        s.on_artifact_detail_loaded(test_artifact_detail("art-1"));
        match &s {
            SessionState::Connected {
                artifacts_busy, ..
            } => assert!(!*artifacts_busy),
            _ => panic!(),
        }
        assert_eq!(s.artifact_detail().map(|d| d.id.as_str()), Some("art-1"));
    }

    #[test]
    fn artifacts_error_clears_busy_and_loading() {
        let mut s = connected_session();
        s.open_artifacts_overlay();
        s.on_artifacts_action_start();
        s.on_artifacts_error("oops");
        match &s {
            SessionState::Connected {
                artifacts_loading,
                artifacts_busy,
                artifacts_error,
                ..
            } => {
                assert!(!*artifacts_loading);
                assert!(!*artifacts_busy);
                assert_eq!(artifacts_error.as_deref(), Some("oops"));
            }
            _ => panic!(),
        }
    }

    #[test]
    fn artifacts_methods_noop_when_not_connected() {
        let mut s = SessionState::start("http://localhost:8080", "tok");
        s.open_artifacts_overlay();
        assert!(!s.is_artifacts_open());
        s.on_artifacts_loaded(test_artifact_rows());
        assert!(s.artifacts_snapshot().is_empty());
        assert!(s.select_artifact(0).is_none());
        s.on_artifact_detail_loaded(test_artifact_detail("x"));
        s.on_artifacts_error("x");
        s.close_artifacts_overlay();
    }

    // ── Tools overlay (M18) ────────────────────────────────────────

    #[test]
    fn tools_starts_closed() {
        let s = connected_session();
        assert!(!s.is_tools_open());
        assert!(s.tools_snapshot().is_empty());
    }

    #[test]
    fn open_tools_sets_loading() {
        let mut s = connected_session();
        s.open_tools_overlay();
        assert!(s.is_tools_open());
        match &s {
            SessionState::Connected { tools_loading, .. } => assert!(*tools_loading),
            _ => panic!(),
        }
    }

    #[test]
    fn tools_loaded_populates_list() {
        let mut s = connected_session();
        s.open_tools_overlay();
        s.on_tools_loaded(vec![
            crate::api::AgentTool { id: "t1".into(), name: "bash".into() },
            crate::api::AgentTool { id: "t2".into(), name: "read_file".into() },
        ]);
        assert_eq!(s.tools_snapshot().len(), 2);
    }

    #[test]
    fn tools_error_clears_loading() {
        let mut s = connected_session();
        s.open_tools_overlay();
        s.on_tools_error("net error");
        match &s {
            SessionState::Connected { tools_loading, tools_error, .. } => {
                assert!(!*tools_loading);
                assert_eq!(tools_error.as_deref(), Some("net error"));
            }
            _ => panic!(),
        }
    }

    #[test]
    fn tools_methods_noop_when_not_connected() {
        let mut s = SessionState::start("http://localhost", "tok");
        s.open_tools_overlay();
        assert!(!s.is_tools_open());
        s.on_tools_loaded(vec![]);
        s.on_tools_error("x");
        s.close_tools_overlay();
    }

    // ── Question widget (M18) ──────────────────────────────────────

    fn test_question() -> crate::api::Question {
        crate::api::Question {
            header: "Choose".into(),
            question: "Pick one".into(),
            options: vec![
                crate::api::QuestionOption { label: "A".into(), description: "Alpha".into() },
                crate::api::QuestionOption { label: "B".into(), description: "Beta".into() },
                crate::api::QuestionOption { label: "C".into(), description: "Gamma".into() },
            ],
            multi_select: false,
        }
    }

    #[test]
    fn no_active_question_initially() {
        let s = connected_session();
        assert!(!s.has_active_question());
        assert!(s.active_question().is_none());
    }

    #[test]
    fn set_active_question_initialises_cursor() {
        let mut s = connected_session();
        s.set_active_question(test_question());
        assert!(s.has_active_question());
        match &s {
            SessionState::Connected { question_cursor, .. } => assert_eq!(*question_cursor, 0),
            _ => panic!(),
        }
    }

    #[test]
    fn move_question_cursor_wraps() {
        let mut s = connected_session();
        s.set_active_question(test_question());
        s.move_question_cursor(-1); // 0 - 1 wraps to 2 (3 options)
        match &s {
            SessionState::Connected { question_cursor, .. } => assert_eq!(*question_cursor, 2),
            _ => panic!(),
        }
        s.move_question_cursor(1);
        match &s {
            SessionState::Connected { question_cursor, .. } => assert_eq!(*question_cursor, 0),
            _ => panic!(),
        }
    }

    #[test]
    fn commit_question_answer_single_select() {
        let mut s = connected_session();
        s.set_active_question(test_question());
        s.move_question_cursor(1); // cursor at index 1 = "B"
        let answer = s.commit_question_answer();
        assert_eq!(answer.as_deref(), Some("B"));
    }

    #[test]
    fn commit_question_answer_multi_select() {
        let mut s = connected_session();
        let mut q = test_question();
        q.multi_select = true;
        s.set_active_question(q);
        // Check options 0 and 2
        s.toggle_question_checked(); // cursor=0, check A
        s.move_question_cursor(1);
        s.move_question_cursor(1); // cursor=2
        s.toggle_question_checked(); // check C
        let answer = s.commit_question_answer();
        assert_eq!(answer.as_deref(), Some("A, C"));
    }

    #[test]
    fn commit_question_multi_select_none_checked_returns_none() {
        let mut s = connected_session();
        let mut q = test_question();
        q.multi_select = true;
        s.set_active_question(q);
        assert!(s.commit_question_answer().is_none());
    }

    #[test]
    fn clear_active_question_removes_it() {
        let mut s = connected_session();
        s.set_active_question(test_question());
        s.clear_active_question();
        assert!(!s.has_active_question());
    }

    #[test]
    fn on_stream_tool_call_sets_question_for_ask_user_question() {
        let mut s = connected_session();
        s.on_select_agent(0);
        // Seed input buffer then send to enter streaming state
        if let SessionState::Connected { input_buffer, .. } = &mut s {
            *input_buffer = "hello".to_string();
        }
        s.on_send().unwrap();
        let args = r#"{"questions":[{
            "header":"Auth","question":"Which?",
            "options":[{"label":"JWT","description":""},{"label":"Sessions","description":""}],
            "multiSelect":false
        }]}"#;
        s.on_stream_tool_call("tc-1", "ask_user_question", args);
        assert!(s.has_active_question());
        assert_eq!(s.active_question().map(|q| q.header.as_str()), Some("Auth"));
    }

    #[test]
    fn on_stream_tool_call_non_question_does_not_set_widget() {
        let mut s = connected_session();
        s.on_select_agent(0);
        if let SessionState::Connected { input_buffer, .. } = &mut s {
            *input_buffer = "hello".to_string();
        }
        s.on_send().unwrap();
        s.on_stream_tool_call("tc-1", "bash", r#"{"command":"ls"}"#);
        assert!(!s.has_active_question());
    }

    // ── Metrics (M19 item 2) ───────────────────────────────────────

    #[test]
    fn metrics_none_initially() {
        let s = connected_session();
        assert!(s.agent_metrics().is_none());
    }

    #[test]
    fn on_metrics_loaded_stores_value() {
        let mut s = connected_session();
        s.on_metrics_loaded(crate::api::AgentMetrics {
            consolidation_runs: 5,
            ..Default::default()
        });
        assert_eq!(s.agent_metrics().map(|m| m.consolidation_runs), Some(5));
    }

    #[test]
    fn metrics_noop_when_not_connected() {
        let mut s = SessionState::start("http://localhost", "tok");
        s.on_metrics_loaded(crate::api::AgentMetrics::default());
        assert!(s.agent_metrics().is_none());
    }

    // ── Cumulative token totals (M19 item 3) ──────────────────────

    #[test]
    fn total_tokens_zero_initially() {
        let s = connected_session();
        assert_eq!(s.total_token_usage(), (0, 0));
    }

    #[test]
    fn total_tokens_accumulate_across_turns() {
        let mut s = connected_session();
        s.on_usage(100, 50, None);
        s.on_usage(200, 80, None);
        assert_eq!(s.total_token_usage(), (300, 130));
    }

    // ── Context overlay (M19 item 3) ──────────────────────────────

    #[test]
    fn context_starts_closed() {
        let s = connected_session();
        assert!(!s.is_context_open());
        assert!(s.context_stats().is_none());
    }

    #[test]
    fn open_context_sets_loading() {
        let mut s = connected_session();
        s.open_context_overlay();
        assert!(s.is_context_open());
        match &s {
            SessionState::Connected { context_loading, .. } => assert!(*context_loading),
            _ => panic!(),
        }
    }

    #[test]
    fn context_loaded_stores_stats() {
        let mut s = connected_session();
        s.open_context_overlay();
        s.on_context_loaded(crate::api::ContextStats {
            window_tokens: 128000,
            ..Default::default()
        });
        assert_eq!(s.context_stats().map(|c| c.window_tokens), Some(128000));
    }

    #[test]
    fn context_error_clears_loading() {
        let mut s = connected_session();
        s.open_context_overlay();
        s.on_context_error("timeout");
        match &s {
            SessionState::Connected { context_loading, context_error, .. } => {
                assert!(!*context_loading);
                assert_eq!(context_error.as_deref(), Some("timeout"));
            }
            _ => panic!(),
        }
    }

    // ── Agents + stats overlays (M19 item 3) ──────────────────────

    #[test]
    fn agents_overlay_open_close() {
        let mut s = connected_session();
        assert!(!s.is_agents_open());
        s.open_agents_overlay();
        assert!(s.is_agents_open());
        s.close_agents_overlay();
        assert!(!s.is_agents_open());
    }

    #[test]
    fn stats_overlay_open_close() {
        let mut s = connected_session();
        assert!(!s.is_stats_open());
        s.open_stats_overlay();
        assert!(s.is_stats_open());
        s.close_stats_overlay();
        assert!(!s.is_stats_open());
    }

    // ── Model picker tests ───────────────────────────────────────────

    fn sample_models() -> Vec<crate::api::ModelInfo> {
        vec![
            crate::api::ModelInfo {
                provider: "anthropic".into(),
                id: "claude-3-5-sonnet".into(),
                display_name: "Claude 3.5 Sonnet".into(),
                context_window: 200_000,
            },
            crate::api::ModelInfo {
                provider: "openai".into(),
                id: "gpt-4o".into(),
                display_name: "GPT-4o".into(),
                context_window: 128_000,
            },
            crate::api::ModelInfo {
                provider: "anthropic".into(),
                id: "claude-3-haiku".into(),
                display_name: "Claude 3 Haiku".into(),
                context_window: 200_000,
            },
        ]
    }

    #[test]
    fn model_picker_open_close() {
        let mut s = connected_session();
        assert!(!s.is_model_picker_open());
        s.open_model_picker();
        assert!(s.is_model_picker_open());
        s.close_model_picker();
        assert!(!s.is_model_picker_open());
    }

    #[test]
    fn model_picker_loads_models() {
        let mut s = connected_session();
        s.open_model_picker();
        s.on_models_loaded(sample_models(), vec!["custom-local".into()]);
        if let SessionState::Connected {
            model_picker_models,
            model_picker_custom_providers,
            model_picker_loading,
            ..
        } = &s
        {
            assert_eq!(model_picker_models.len(), 3);
            assert_eq!(model_picker_custom_providers, &["custom-local"]);
            assert!(!model_picker_loading);
        } else {
            panic!("expected Connected");
        }
    }

    #[test]
    fn model_picker_error_state() {
        let mut s = connected_session();
        s.open_model_picker();
        s.on_models_error("network error".into());
        if let SessionState::Connected {
            model_picker_loading,
            model_picker_error,
            ..
        } = &s
        {
            assert!(!model_picker_loading);
            assert_eq!(model_picker_error.as_deref(), Some("network error"));
        } else {
            panic!("expected Connected");
        }
    }

    #[test]
    fn model_picker_query_resets_selection() {
        let mut s = connected_session();
        s.open_model_picker();
        s.on_models_loaded(sample_models(), vec![]);
        s.set_model_picker_selection(2);
        s.set_model_picker_query("gpt".into());
        if let SessionState::Connected {
            model_picker_selection,
            model_picker_query,
            ..
        } = &s
        {
            assert_eq!(*model_picker_selection, 0);
            assert_eq!(model_picker_query, "gpt");
        } else {
            panic!("expected Connected");
        }
    }

    #[test]
    fn filter_models_matches_id_provider_display() {
        let models = sample_models();
        assert_eq!(super::filter_models(&models, "").len(), 3);
        assert_eq!(super::filter_models(&models, "claude").len(), 2);
        assert_eq!(super::filter_models(&models, "openai").len(), 1);
        assert_eq!(super::filter_models(&models, "GPT").len(), 1);
        assert_eq!(super::filter_models(&models, "haiku").len(), 1);
        assert_eq!(super::filter_models(&models, "xyz").len(), 0);
    }

    #[test]
    fn selected_model_id_returns_correct_id() {
        let mut s = connected_session();
        s.open_model_picker();
        s.on_models_loaded(sample_models(), vec![]);
        assert_eq!(s.selected_model_id(), Some("claude-3-5-sonnet".into()));
        s.set_model_picker_selection(1);
        assert_eq!(s.selected_model_id(), Some("gpt-4o".into()));
    }

    // ── MCP overlay ─────────────────────────────────────────────────────

    fn sample_mcp_servers() -> Vec<crate::api::McpServerInfo> {
        vec![
            crate::api::McpServerInfo {
                key: "desktop-commander".into(),
                command: "npx @desktop-commander/mcp-server".into(),
                tools: vec![
                    "desktop-commander__bash".into(),
                    "desktop-commander__read_file".into(),
                ],
                disabled: false,
            },
            crate::api::McpServerInfo {
                key: "old-server".into(),
                command: "old-cmd".into(),
                tools: vec![],
                disabled: true,
            },
        ]
    }

    #[test]
    fn mcp_overlay_open_sets_loading() {
        let mut s = connected_session();
        assert!(!s.is_mcp_open());
        s.open_mcp_overlay();
        assert!(s.is_mcp_open());
        if let SessionState::Connected { mcp_loading, mcp_error, .. } = &s {
            assert!(mcp_loading);
            assert!(mcp_error.is_none());
        } else {
            panic!("expected Connected");
        }
    }

    #[test]
    fn mcp_overlay_close_resets_state() {
        let mut s = connected_session();
        s.open_mcp_overlay();
        s.close_mcp_overlay();
        assert!(!s.is_mcp_open());
    }

    #[test]
    fn mcp_on_loaded_populates_servers() {
        let mut s = connected_session();
        s.open_mcp_overlay();
        s.on_mcp_loaded(sample_mcp_servers());
        if let SessionState::Connected { mcp_servers, mcp_loading, mcp_error, .. } = &s {
            assert_eq!(mcp_servers.len(), 2);
            assert_eq!(mcp_servers[0].key, "desktop-commander");
            assert_eq!(mcp_servers[0].tools.len(), 2);
            assert!(mcp_servers[1].disabled);
            assert!(!mcp_loading);
            assert!(mcp_error.is_none());
        } else {
            panic!("expected Connected");
        }
    }

    #[test]
    fn mcp_on_error_sets_message() {
        let mut s = connected_session();
        s.open_mcp_overlay();
        s.on_mcp_error("connection refused".into());
        if let SessionState::Connected { mcp_loading, mcp_error, .. } = &s {
            assert!(!mcp_loading);
            assert_eq!(mcp_error.as_deref(), Some("connection refused"));
        } else {
            panic!("expected Connected");
        }
    }

    // ── Plan panel tests ──────────────────────────────────────────

    #[test]
    fn no_active_plan_initially() {
        let s = connected_session();
        assert!(s.active_plan().is_none());
    }

    #[test]
    fn set_plan_creates_steps() {
        let mut s = connected_session();
        s.set_plan(vec!["Step 1".into(), "Step 2".into(), "Step 3".into()]);
        let plan = s.active_plan().unwrap();
        assert_eq!(plan.steps.len(), 3);
        assert!(plan.is_visible);
        assert_eq!(plan.steps[0].id, 1);
        assert_eq!(plan.steps[0].description, "Step 1");
        assert!(!plan.steps[0].is_done);
        assert_eq!(plan.steps[2].id, 3);
    }

    #[test]
    fn set_plan_empty_clears() {
        let mut s = connected_session();
        s.set_plan(vec!["A".into()]);
        assert!(s.active_plan().is_some());
        s.set_plan(vec![]);
        assert!(s.active_plan().is_none());
    }

    #[test]
    fn update_plan_step_marks_done() {
        let mut s = connected_session();
        s.set_plan(vec!["A".into(), "B".into()]);
        assert!(s.update_plan_step(1, true));
        let plan = s.active_plan().unwrap();
        assert!(plan.steps[0].is_done);
        assert!(!plan.steps[1].is_done);
    }

    #[test]
    fn update_plan_step_invalid_id_returns_false() {
        let mut s = connected_session();
        s.set_plan(vec!["A".into()]);
        assert!(!s.update_plan_step(99, true));
    }

    #[test]
    fn on_stream_tool_call_intercepts_set_plan() {
        let mut s = connected_session();
        s.on_select_agent(0);
        if let SessionState::Connected { input_buffer, .. } = &mut s {
            *input_buffer = "go".to_string();
        }
        s.on_send();
        s.on_stream_tool_call("tc-1", "set_plan", r#"{"steps":["Read","Write","Test"]}"#);
        let plan = s.active_plan().unwrap();
        assert_eq!(plan.steps.len(), 3);
        assert_eq!(plan.steps[0].description, "Read");
    }

    #[test]
    fn on_stream_tool_call_intercepts_update_plan() {
        let mut s = connected_session();
        s.on_select_agent(0);
        if let SessionState::Connected { input_buffer, .. } = &mut s {
            *input_buffer = "go".to_string();
        }
        s.on_send();
        s.on_stream_tool_call("tc-1", "set_plan", r#"{"steps":["A","B"]}"#);
        s.on_stream_tool_call("tc-2", "UpdatePlan", r#"{"step_id":1,"done":true}"#);
        let plan = s.active_plan().unwrap();
        assert!(plan.steps[0].is_done);
        assert!(!plan.steps[1].is_done);
    }

    // ── Live output tests ─────────────────────────────────────────

    #[test]
    fn live_outputs_empty_initially() {
        let s = connected_session();
        assert!(s.live_outputs().is_empty());
    }

    #[test]
    fn begin_live_output_creates_block() {
        let mut s = connected_session();
        s.begin_live_output("tc-1", "bash");
        assert_eq!(s.live_outputs().len(), 1);
        assert_eq!(s.live_outputs()[0].call_id, "tc-1");
        assert_eq!(s.live_outputs()[0].tool_name, "bash");
        assert!(!s.live_outputs()[0].done);
    }

    #[test]
    fn append_live_output_adds_lines() {
        let mut s = connected_session();
        s.begin_live_output("tc-1", "bash");
        s.append_live_output("tc-1", "line 1".into());
        s.append_live_output("tc-1", "line 2".into());
        assert_eq!(s.live_outputs()[0].lines.len(), 2);
    }

    #[test]
    fn finish_live_output_marks_done() {
        let mut s = connected_session();
        s.begin_live_output("tc-1", "bash");
        s.finish_live_output("tc-1");
        assert!(s.live_outputs()[0].done);
    }

    #[test]
    fn append_to_unknown_call_id_is_noop() {
        let mut s = connected_session();
        s.begin_live_output("tc-1", "bash");
        s.append_live_output("tc-99", "orphan".into());
        assert!(s.live_outputs()[0].lines.is_empty());
    }
}
