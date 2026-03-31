use crate::server::{config::ServerConfig, rate_limit::RateLimiter, storage::Db};
use cade_ai::{LlmProvider, LlmRouter};
use std::sync::Arc;
use tokio::sync::RwLock;

/// Shared application state injected into every axum handler
/// Tracks activity and consolidation state per agent.
#[derive(Debug, Clone)]
pub struct AgentActivity {
    pub last_active_ts: i64,
    pub needs_consolidation: bool,
    pub conversation_id: Option<String>,
}

#[derive(Clone)]
pub struct AppState {
    pub db: Db,
    pub llm: Arc<dyn LlmProvider>,
    /// Router behind RwLock for hot-reload — /connect adds providers without restart
    pub llm_router: Arc<RwLock<LlmRouter>>,
    pub config: Arc<ServerConfig>,
    /// Per-agent token-bucket rate limiter
    pub rate_limiter: RateLimiter,
    /// Per-agent system-prompt cache: key=agent_id, value=(hash, system_prompt_without_tool_rule).
    /// When memory blocks are unchanged the hash matches and we reuse the cached string, keeping
    /// the system-prompt prefix byte-identical across turns so OpenAI/Gemini implicit caches hit.
    pub memory_cache: Arc<std::sync::Mutex<std::collections::HashMap<String, (u64, String)>>>,
    /// Tracks `(last_active_ts, needs_consolidation, conversation_id)` per agent.
    /// `needs_consolidation` is set by `build_context` whenever older turns are
    /// dropped from the context window — the Sleeptime background task picks it
    /// up after 60 s of inactivity and summarises the dropped turns.
    pub agent_activity: Arc<RwLock<std::collections::HashMap<String, AgentActivity>>>,
    /// Intelligent tool selection: reranks tools per-prompt to reduce token usage.
    /// `None` when the `reranker` feature is not compiled in or not configured.
    #[cfg(feature = "reranker")]
    pub tool_reranker: Option<Arc<cade_reranker::ToolReranker>>,
}
