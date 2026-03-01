pub mod agents;
pub mod health;
pub mod messages;
pub mod models;
pub mod providers;
pub mod tools;

use axum::{
    Router,
    routing::{delete, get, post, put},
};
use crate::server::state::AppState;

pub fn router(state: AppState) -> Router {
    Router::new()
        // Health + server config
        .route("/v1/health", get(health::get_health))
        .route("/v1/config", get(health::get_config))
        // Agents
        .route("/v1/agents",          post(agents::create_agent).get(agents::list_agents))
        .route("/v1/agents/:id",      get(agents::get_agent).delete(agents::delete_agent).patch(agents::patch_agent))
        // Agent tools
        .route("/v1/agents/:id/tools", post(agents::attach_tools))
        // Agent memory
        .route("/v1/agents/:id/memory",         get(agents::get_memory))
        .route("/v1/agents/:id/memory/:label",  put(agents::upsert_memory).delete(agents::delete_memory))
        // Conversations
        .route("/v1/agents/:id/conversations",
               get(agents::list_conversations).post(agents::create_conversation))
        .route("/v1/agents/:id/conversations/:conv_id",
               delete(agents::delete_conversation))
        // Messages
        .route("/v1/agents/:id/messages",
               post(messages::send_message).delete(agents::clear_messages_handler).get(agents::search_messages_handler))
        .route("/v1/agents/:id/messages/stream", post(messages::stream_message))
        // Tools
        .route("/v1/tools", post(tools::create_tool).get(tools::list_tools))
        // Models
        .route("/v1/models", get(models::list_models))
        // Providers
        .route("/v1/providers",          post(providers::add_provider).get(providers::list_providers))
        .route("/v1/providers/presets",  get(providers::list_presets))
        .route("/v1/providers/:name",    delete(providers::remove_provider))
        .with_state(state)
}
