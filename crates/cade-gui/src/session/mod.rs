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
    pub title: String,
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
    Connected(Box<ConnectedSession>),

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
            | Self::ConnectionFailed { server_url, .. } => server_url,
            Self::Connected(s) => &s.server_url,
        }
    }

    /// The bearer token for this session.
    pub fn token(&self) -> &str {
        match self {
            Self::Connecting { token, .. }
            | Self::HealthOk { token, .. }
            | Self::ConnectionFailed { token, .. } => token,
            Self::Connected(s) => &s.token,
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
        matches!(self, Self::Connected(session) if matches!(&**session, crate::session::ConnectedSession {  ..  }))
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

#[derive(Debug, Clone, PartialEq)]
pub struct ConnectedSession {
    pub server_url: String,
    pub token: String,
    pub health: HealthInfo,
    pub agents: Vec<AgentInfo>,
    /// Index into `agents` of the currently selected agent, if any.
    pub selected_agent: Option<usize>,
    /// Messages for the selected agent (empty until an agent is selected
    /// and the fetch completes).
    pub messages: Vec<ChatMessage>,
    /// The text the user is currently typing in the input bar.
    pub input_buffer: String,
    /// True while we are streaming an assistant response via SSE.
    pub streaming: bool,
    /// Whether the timeline should auto-scroll to the bottom.
    /// Set to `false` when the user scrolls up; restored to `true`
    /// when they click the ↓ button or a new message arrives.
    pub auto_scroll: bool,
    /// Transient error message displayed as a toast overlay.
    pub error_toast: Option<String>,
    /// Active conversation ID (set from SSE metadata, cleared on agent switch).
    pub conversation_id: Option<String>,
    /// Token usage from the last completed turn.
    pub last_usage: Option<(u64, u64, Option<String>)>,
    /// Finish reason from the last completed turn (e.g. "stop", "length").
    pub last_finish_reason: Option<String>,
    /// Conversations for the selected agent.
    pub conversations: Vec<crate::api::ConversationInfo>,
    /// Index into `conversations` of the currently selected conversation.
    pub selected_conversation: Option<usize>,
    /// Whether there are more messages to load (pagination).
    pub has_more_messages: bool,
    /// Whether the slash-command palette overlay is visible.
    pub palette_open: bool,
    /// Current text in the palette filter input.
    pub palette_input: String,
    /// Index of the highlighted entry in the filtered palette list.
    pub palette_selection: usize,
    /// Whether the full-screen command menu overlay is visible.
    pub menu_open: bool,
    /// Current text in the menu filter input.
    pub menu_input: String,
    /// Index of the highlighted entry in the filtered menu list.
    pub menu_selection: usize,
    /// Whether the memory-viewer overlay is visible.
    pub memory_open: bool,
    /// Memory blocks fetched from `GET /v1/agents/:id/memory`.
    pub memory_blocks: Vec<crate::api::MemoryBlock>,
    /// Index into `memory_blocks` of the currently-viewed block.
    pub memory_selection: usize,
    /// Editable buffer mirrored from the selected block — saved on
    /// "Save" click via `PUT /v1/agents/:id/memory/:label`.
    pub memory_edit_buffer: String,
    /// True while the GET request is in flight.
    pub memory_loading: bool,
    /// True while a PUT request is in flight.
    pub memory_saving: bool,
    /// Per-overlay error message (shown inside the memory window).
    pub memory_error: Option<String>,
    /// Transient success notice shown after a successful save
    /// (e.g. "Saved /project").  Cleared when the selection changes,
    /// the overlay closes, or another save starts.
    pub memory_save_notice: Option<String>,

    pub memory_history_open: bool,
    pub memory_history: Vec<crate::api::MemoryHistoryRevision>,
    pub memory_history_loading: bool,

    // ── Checkpoints overlay (M17) ────────────────────────────
    /// Whether the checkpoints overlay is visible.
    pub checkpoints_open: bool,
    /// Rows fetched from `GET /v1/agents/:id/checkpoints`.
    pub checkpoints: Vec<crate::api::CheckpointRow>,
    /// True while the GET request is in flight.
    pub checkpoints_loading: bool,
    /// True while a restore/delete/create request is in flight.
    pub checkpoints_busy: bool,
    /// Per-overlay error message.
    pub checkpoints_error: Option<String>,
    /// Transient success notice (e.g. "Restored cp-1234…").
    pub checkpoints_notice: Option<String>,

    // ── Artifacts overlay (M17) ──────────────────────────────
    /// Whether the artifacts overlay is visible.
    pub artifacts_open: bool,
    /// Summary rows fetched from `GET /v1/agents/:id/artifacts`.
    pub artifacts: Vec<crate::api::ArtifactInfo>,
    /// Index of the currently-selected row; `None` when nothing selected.
    pub artifact_selection: Option<usize>,
    /// Full detail for the selected artifact — lazy-loaded on click.
    /// `None` means not-yet-loaded; a loaded detail whose `id` differs
    /// from the selected row's `id` means stale and will be replaced.
    pub artifact_detail: Option<crate::api::ArtifactDetail>,
    /// True while the list GET is in flight.
    pub artifacts_loading: bool,
    /// True while a per-artifact detail fetch or delete is in flight.
    pub artifacts_busy: bool,
    /// Per-overlay error message.
    pub artifacts_error: Option<String>,

    // ── Tools overlay (M18 — MCP / skills) ──────────────────
    /// Whether the tools/MCP overlay is visible.
    pub tools_open: bool,
    /// Tools fetched from `GET /v1/agents/:id/tools`.
    pub tools: Vec<crate::api::AgentTool>,
    /// True while the GET request is in flight.
    pub tools_loading: bool,
    /// Per-overlay error message.
    pub tools_error: Option<String>,

    // ── Inline question widget (M18 — ask_user_question) ────
    /// The currently-active question received via `ask_user_question`
    /// SSE tool call.  `None` when no question is awaiting an answer.
    pub active_question: Option<crate::api::Question>,
    /// Index of the currently-highlighted option (single-select) or
    /// the last-moved position (multi-select).
    pub question_cursor: usize,
    /// Set of selected option indices (multi-select only).
    pub question_checked: Vec<bool>,

    // ── Server metrics (M19 item 2) ──────────────────────────
    /// Last-fetched server-side consolidation metrics for this agent.
    pub agent_metrics: Option<crate::api::AgentMetrics>,

    // ── Cumulative token usage totals (M19 item 3 /stats) ────
    /// Running total of input tokens across all turns in this session.
    pub total_input_tokens: u64,
    /// Running total of output tokens across all turns in this session.
    pub total_output_tokens: u64,

    // ── Context stats overlay (M19 item 3 /context) ──────────
    /// Whether the context-stats overlay is open.
    pub context_open: bool,
    /// Last-fetched context window stats.
    pub context_stats: Option<crate::api::ContextStats>,
    /// True while the GET /context request is in flight.
    pub context_loading: bool,
    /// Per-overlay error for context panel.
    pub context_error: Option<String>,

    // ── Agents overlay (M19 item 3 /agents) ──────────────────
    /// Whether the agents list overlay is open.
    pub agents_open: bool,

    // ── Stats overlay (M19 item 3 /stats) ────────────────────
    /// Whether the stats overlay is open.
    pub stats_open: bool,

    // ── MCP servers overlay ───────────────────────────────────
    /// Whether the MCP servers overlay is open.
    pub mcp_open: bool,
    /// Servers fetched from `GET /v1/mcp`.
    pub mcp_servers: Vec<crate::api::McpServerInfo>,
    /// True while the GET request is in flight.
    pub mcp_loading: bool,
    /// Per-overlay error message.
    pub mcp_error: Option<String>,

    /// A pending theme update from the backend.
    pub theme_update: Option<String>,

    // ── Model picker overlay ─────────────────────────────────
    /// Whether the model picker overlay is open.
    pub model_picker_open: bool,
    /// Available models fetched from `GET /v1/models`.
    pub model_picker_models: Vec<crate::api::ModelInfo>,
    /// Custom provider names (no model listing available).
    pub model_picker_custom_providers: Vec<String>,
    /// Fuzzy filter query typed in the model picker search box.
    pub model_picker_query: String,
    /// Index of the currently highlighted model in the filtered list.
    pub model_picker_selection: usize,
    /// Whether models are currently being fetched.
    pub model_picker_loading: bool,
    /// Error message from model fetch failure.
    pub model_picker_error: Option<String>,

    // ── Plan panel (mirrors TUI PlanState) ──────────────────
    /// Active plan steps. `None` when no plan has been set.
    pub active_plan: Option<PlanState>,

    // ── Live output (mirrors TUI LiveOutput) ─────────────────
    /// Active live-output blocks keyed by tool call ID.
    /// Each entry is a scrollable block of output lines shown in the
    /// timeline while a long-running tool (e.g. `bash`) is executing.
    pub live_outputs: Vec<LiveOutputBlock>,

    /// Per-category context-window breakdown (fetched on demand).
    pub context_breakdown: Option<crate::api::ContextBreakdown>,
    /// Whether a context-breakdown fetch is in progress.
    pub context_breakdown_loading: bool,

    // ── Settings overlays ────────────────────────────────────
    pub providers_open: bool,
    pub providers: Vec<crate::api::ProviderInfo>,
    pub providers_loading: bool,
    pub permissions_open: bool,
    pub current_permission_mode: String,
    pub theme_picker_open: bool,
    pub available_themes: Vec<String>,
    pub current_theme_name: String,
    pub hooks_open: bool,
    pub hooks: Vec<crate::api::HookInfo>,
    pub hooks_loading: bool,
    pub toolset_open: bool,
    pub current_toolset: String,
    pub pricing_open: bool,
    pub pricing_info: String,
    pub backend_open: bool,
    pub current_backend: String,
    pub reasoning_open: bool,
    pub current_reasoning_effort: String,
    // ── Skills overlay ───────────────────────────────────
    pub skills_overlay_open: bool,
    pub all_skills_list: Vec<crate::api::SkillEntry>,
    pub loaded_skill_ids: Vec<String>,
    pub skills_loading: bool,
    pub skills_filter: String,
    // ── Subagent tracking ────────────────────────────────
    pub subagent_cards: Vec<SubagentCardState>,

    // ── Profiles (Environment parity) ──────────────────────────
    pub profiles_open: bool,
    pub profiles: Vec<(String, String, String)>,
    pub profile_edit_name: String,
    pub profile_edit_url: String,
    pub profile_edit_token: String,
}

// ── GUI-1 network graph topology ───────────────────────────────────────

/// Visual node categories for the overview network graph.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NetworkNodeKind {
    Agent,
    Model,
    Tool,
    Memory,
    McpServer,
    Context,
}

/// One node in the high-fidelity overview topology graph.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NetworkNode {
    pub id: String,
    pub label: String,
    pub kind: NetworkNodeKind,
    pub meta: String,
}

/// Directed relationship between two topology graph nodes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NetworkEdge {
    pub from: String,
    pub to: String,
    pub label: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OverviewLayoutSpec {
    pub title: &'static str,
    pub columns: Vec<&'static str>,
    pub metric_cards: Vec<&'static str>,
    pub center_sections: Vec<&'static str>,
    pub operation_panels: Vec<&'static str>,
}

pub fn reference_layout_spec() -> OverviewLayoutSpec {
    OverviewLayoutSpec {
        title: "CADE Command Center",
        columns: vec!["metrics", "network", "operations"],
        metric_cards: vec!["Agents", "Tools", "Memory", "Tokens"],
        center_sections: vec!["Activity", "Network Node Graph"],
        operation_panels: vec!["Session", "MCP Servers", "Recent Tools"],
    }
}

/// Deterministic topology snapshot rendered by GUI-1.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct NetworkGraphTopology {
    pub nodes: Vec<NetworkNode>,
    pub edges: Vec<NetworkEdge>,
}

impl ConnectedSession {
    /// Build a deterministic, bounded topology for the overview network graph.
    pub fn network_graph_topology(&self) -> NetworkGraphTopology {
        const TOOL_LIMIT: usize = 8;
        const MEMORY_LIMIT: usize = 6;
        let mut graph = NetworkGraphTopology::default();

        let Some(agent) = self
            .selected_agent
            .and_then(|idx| self.agents.get(idx))
            .or_else(|| self.agents.first())
        else {
            return graph;
        };

        let agent_id = format!("agent:{}", agent.id);
        graph.nodes.push(NetworkNode {
            id: agent_id.clone(),
            label: agent.name.clone(),
            kind: NetworkNodeKind::Agent,
            meta: agent
                .model
                .clone()
                .unwrap_or_else(|| "no model selected".to_string()),
        });

        let model = self
            .context_stats
            .as_ref()
            .and_then(|stats| stats.model.clone())
            .or_else(|| agent.model.clone())
            .or_else(|| {
                self.last_usage
                    .as_ref()
                    .and_then(|(_, _, model)| model.clone())
            });
        if let Some(model) = model.filter(|m| !m.is_empty()) {
            let model_id = format!("model:{model}");
            graph.nodes.push(NetworkNode {
                id: model_id.clone(),
                label: model.clone(),
                kind: NetworkNodeKind::Model,
                meta: "active model".to_string(),
            });
            graph.edges.push(NetworkEdge {
                from: agent_id.clone(),
                to: model_id,
                label: "uses".to_string(),
            });
        }

        if let Some(stats) = &self.context_stats {
            if stats.window_tokens > 0 || stats.chars_used > 0 {
                let pct = if stats.window_tokens > 0 {
                    ((stats.chars_used.saturating_mul(100)) / stats.window_tokens).min(100)
                } else {
                    0
                };
                let context_id = "context:window".to_string();
                graph.nodes.push(NetworkNode {
                    id: context_id.clone(),
                    label: "Context Window".to_string(),
                    kind: NetworkNodeKind::Context,
                    meta: format!("{pct}% used"),
                });
                graph.edges.push(NetworkEdge {
                    from: agent_id.clone(),
                    to: context_id,
                    label: "packs".to_string(),
                });
            }
        }

        for block in self.memory_blocks.iter().take(MEMORY_LIMIT) {
            let node_id = format!("memory:{}", block.label);
            graph.nodes.push(NetworkNode {
                id: node_id.clone(),
                label: block.label.clone(),
                kind: NetworkNodeKind::Memory,
                meta: block.tier.clone().unwrap_or_else(|| "memory".to_string()),
            });
            graph.edges.push(NetworkEdge {
                from: node_id,
                to: agent_id.clone(),
                label: "grounds".to_string(),
            });
        }

        for server in self.mcp_servers.iter().filter(|s| !s.disabled) {
            let node_id = format!("mcp:{}", server.key);
            graph.nodes.push(NetworkNode {
                id: node_id.clone(),
                label: server.key.clone(),
                kind: NetworkNodeKind::McpServer,
                meta: format!("{} tools", server.tools.len()),
            });
            graph.edges.push(NetworkEdge {
                from: agent_id.clone(),
                to: node_id,
                label: "connects".to_string(),
            });
        }

        for tool in self.tools.iter().take(TOOL_LIMIT) {
            let node_id = format!("tool:{}", tool.name);
            graph.nodes.push(NetworkNode {
                id: node_id.clone(),
                label: tool.name.clone(),
                kind: NetworkNodeKind::Tool,
                meta: tool.id.clone(),
            });
            graph.edges.push(NetworkEdge {
                from: agent_id.clone(),
                to: node_id.clone(),
                label: "can call".to_string(),
            });
            if let Some((server_key, _)) = tool.name.split_once("__") {
                let server_id = format!("mcp:{server_key}");
                if graph.nodes.iter().any(|node| node.id == server_id) {
                    graph.edges.push(NetworkEdge {
                        from: server_id,
                        to: node_id,
                        label: "exposes".to_string(),
                    });
                }
            }
        }

        graph
    }
}
