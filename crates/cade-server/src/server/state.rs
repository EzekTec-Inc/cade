use crate::server::{config::ServerConfig, rate_limit::RateLimiter, storage::Db};
use cade_ai::{LlmProvider, LlmRouter};
use std::sync::Arc;
use tokio::sync::RwLock;

/// Shared application state injected into every axum handler
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
}
