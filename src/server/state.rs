use std::sync::Arc;
use tokio::sync::RwLock;
use crate::server::{
    config::ServerConfig,
    llm::{LlmProvider, LlmRouter},
    rate_limit::RateLimiter,
    storage::Db,
};

/// Shared application state injected into every axum handler
#[derive(Clone)]
pub struct AppState {
    pub db:           Db,
    pub llm:          Arc<dyn LlmProvider>,
    /// Router behind RwLock for hot-reload — /connect adds providers without restart
    pub llm_router:   Arc<RwLock<LlmRouter>>,
    pub config:       Arc<ServerConfig>,
    /// Per-agent token-bucket rate limiter
    pub rate_limiter: RateLimiter,
}
