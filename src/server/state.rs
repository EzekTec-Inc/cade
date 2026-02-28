use std::sync::Arc;
use crate::server::{config::ServerConfig, llm::LlmProvider, storage::Db};

/// Shared application state injected into every axum handler
#[derive(Clone)]
pub struct AppState {
    pub db: Db,
    pub llm: Arc<dyn LlmProvider>,
    pub config: Arc<ServerConfig>,
}
