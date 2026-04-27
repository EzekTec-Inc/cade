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

/// Subagent tracking state (independent of `app::views` to avoid cfg issues).
#[derive(Debug, Clone, PartialEq)]
pub struct SubagentCardState {
    pub subagent_id: String,
    pub task: String,
    pub mode: String,
    pub model: String,
    pub status: String,
    pub elapsed_secs: u32,
    pub tool_calls: u32,
    pub output_lines: u32,
    pub result_preview: String,
    pub is_error: bool,
}

/// Post-login session state.
///
/// Created from `LoginState::Submitted` — the token and server URL are
/// captured at construction and never mutated.
#[derive(Debug, Clone, PartialEq)]
#[allow(clippy::large_enum_variant)] // Connected is intentionally rich; boxing adds no value
pub enum SessionState {
    /// Token submitted, waiting for health + agent-list responses.
    Connecting { server_url: String, token: String },
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
        // ── Subagent tracking ────────────────────────────────
        subagent_cards: Vec<SubagentCardState>,
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

    /// Whether the caller should attempt a retry (re-enter the login flow).
    /// Only meaningful in `ConnectionFailed`.
    pub fn is_failed(&self) -> bool {
        matches!(self, Self::ConnectionFailed { .. })
    }

    /// Whether the session is fully established.
    pub fn is_connected(&self) -> bool {
        matches!(self, Self::Connected { .. })
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

mod agents;
mod artifacts;
mod checkpoints;
mod context_breakdown;
mod conversations;
mod live_output;
mod memory_overlay;
mod messages;
mod overlays;
mod plan;
mod skills;
mod streaming;
mod subagents;
mod theme_update;
mod toasts;
mod tools_overlay;

#[cfg(test)]
mod tests;
