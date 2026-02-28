use std::sync::Arc;
use crate::server::{config::ServerConfig, llm::{LlmProvider, LlmRouter}, storage::Db};

/// Shared application state injected into every axum handler
#[derive(Clone)]
pub struct AppState {
    pub db: Db,
    pub llm: Arc<dyn LlmProvider>,
    /// The router — kept separately so handlers can call validate_model()
    pub llm_router: Arc<LlmRouter>,
    pub config: Arc<ServerConfig>,
}
