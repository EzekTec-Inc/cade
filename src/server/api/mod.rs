pub mod agents;
pub mod health;
pub mod messages;
pub mod tools;

use axum::{
    Router,
    routing::{delete, get, post},
};
use crate::server::state::AppState;

pub fn router(state: AppState) -> Router {
    Router::new()
        // Health + server config
        .route("/v1/health", get(health::get_health))
        .route("/v1/config", get(health::get_config))
        // Agents
        .route("/v1/agents",          post(agents::create_agent).get(agents::list_agents))
        .route("/v1/agents/:id",      get(agents::get_agent).delete(agents::delete_agent))
        // Messages
        .route("/v1/agents/:id/messages",        post(messages::send_message))
        .route("/v1/agents/:id/messages/stream", post(messages::stream_message))
        // Tools
        .route("/v1/tools", post(tools::create_tool).get(tools::list_tools))
        .with_state(state)
}
