use crate::server::{config::ServerConfig, rate_limit::RateLimiter};
use cade_store::sqlite::Db;
use cade_ai::{LlmProvider, LlmRouter, LlmMessage};
use serde_json::Value;
use std::sync::Arc;
use tokio::sync::RwLock;

/// Re-export so call-sites in api/ can do `use crate::server::state::McpManager`.
pub use cade_agent::mcp::McpManager;

/// Shared application state injected into every axum handler
/// Tracks activity and consolidation state per agent.
#[derive(Debug, Clone)]
pub struct AgentActivity {
    pub last_active_ts: i64,
    pub needs_consolidation: bool,
    pub conversation_id: Option<String>,
    /// Turn counter snapshot at the time the last eager consolidation was
    /// triggered for this agent. Used by `should_eager_consolidate` to
    /// rate-limit the eager path (M3): even if `needs_consolidation` remains
    /// set across many rapid turns, a fresh run fires only once per
    /// `EAGER_CONSOLIDATION_TURN_THRESHOLD` turns. `0` means "never".
    pub last_consolidation_turn: i64,
}

#[derive(Debug, Clone, Default, serde::Serialize)]
pub struct AgentMetrics {
    pub tool_outputs_compacted: usize,
    pub consolidation_runs: usize,
    pub chars_summarised: usize,
    pub chars_produced: usize,
    pub inflation_guard_hits: usize,
}

#[derive(Clone)]
pub struct AppState {
    pub db: Db,
    pub llm: Arc<dyn LlmProvider>,
    /// Router behind RwLock for hot-reload — /connect adds providers without restart
    pub llm_router: Arc<RwLock<LlmRouter>>,
    pub config: Arc<ServerConfig>,
    /// MCP manager — executes tool calls on behalf of the agentic loop.
    /// Populated at startup from merged settings; empty when no MCP servers are configured.
    pub mcp: Arc<McpManager>,
    /// Per-agent token-bucket rate limiter
    pub rate_limiter: RateLimiter,
    /// Per-agent system-prompt cache: key=agent_id, value=(hash, system_prompt_without_tool_rule).
    /// When memory blocks are unchanged the hash matches and we reuse the cached string, keeping
    /// the system-prompt prefix byte-identical across turns so OpenAI/Gemini implicit caches hit.
    pub memory_cache: Arc<std::sync::Mutex<std::collections::HashMap<String, (u64, String)>>>,
    /// Tracks `(last_active_ts, needs_consolidation, conversation_id)` per agent.
    /// `needs_consolidation` is set by `build_context` whenever older turns are
    /// dropped from the context window — the Sleeptime background task picks it
    /// up after 20 s of inactivity and summarises the dropped turns. An eager
    /// turn-count path in `build_context` (see `should_eager_consolidate`)
    /// covers continuous sessions that never hit the idle timer.
    pub agent_activity: Arc<RwLock<std::collections::HashMap<String, AgentActivity>>>,
    /// Tracks lifetime context efficiency metrics per agent.
    pub agent_metrics: Arc<RwLock<std::collections::HashMap<String, AgentMetrics>>>,
    /// LRU cache for `build_context` outputs to avoid recomputing history loops.
    /// Key: `format!("{agent_id}:{conversation_id}")`
    /// Value: `(max_rowid, cached_context_tuple)`
    pub context_cache: Arc<std::sync::Mutex<lru::LruCache<String, (u64, (String, Vec<LlmMessage>, Vec<Value>))>>>,
}
