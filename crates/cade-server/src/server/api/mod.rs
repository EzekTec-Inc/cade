// region:    --- Modules

pub mod agents;
pub mod auth;
pub mod health;
pub mod messages;
pub mod models;
pub mod providers;
pub mod runs;
pub mod tools;

use axum::{
    Router,
    middleware,
    routing::{delete, get, post, put},
};
use crate::server::{
    rate_limit::rate_limit_middleware,
    state::AppState,
};

// endregion: --- Modules

pub fn router(state: AppState) -> Router {
    // ── Inference routes (rate-limited) ───────────────────────────────────────
    let inference = Router::new()
        .route("/v1/agents/:id/messages",
               post(messages::send_message)
               .delete(agents::clear_messages_handler)
               .get(agents::search_messages_handler))
        .route("/v1/agents/:id/messages/stream", post(messages::stream_message))
        .layer(middleware::from_fn_with_state(state.clone(), rate_limit_middleware));

    // ── All other routes (no rate limit) ─────────────────────────────────────
    let rest = Router::new()
        // Health + server config
        .route("/v1/health", get(health::get_health))
        .route("/v1/config", get(health::get_config))
        // Agents
        .route("/v1/agents",     post(agents::create_agent).get(agents::list_agents))
        .route("/v1/agents/:id", get(agents::get_agent).delete(agents::delete_agent).patch(agents::patch_agent))
        // Agent tools
        .route("/v1/agents/:id/tools",
               get(agents::get_agent_tools)
               .post(agents::attach_tools)
               .delete(agents::detach_tools))
        // Agent memory
        .route("/v1/agents/:id/memory",                             get(agents::search_memory_handler))
        .route("/v1/agents/:id/memory/:label",                      put(agents::upsert_memory).delete(agents::delete_memory))
        .route("/v1/agents/:id/memory/:label/tier",                 put(agents::set_memory_tier_handler))
        .route("/v1/agents/:id/memory/:label/history",              get(agents::get_memory_history))
        .route("/v1/agents/:id/memory/:label/restore/:rev_id",      put(agents::restore_memory_revision))
        // Conversations
        .route("/v1/agents/:id/conversations",
               get(agents::list_conversations).post(agents::create_conversation))
        .route("/v1/agents/:id/conversations/:conv_id",
               delete(agents::delete_conversation))
        // Runs (background mode)
        .route("/v1/runs/:run_id",        get(runs::get_run))
        .route("/v1/runs/:run_id/stream", get(runs::stream_run))
        // Tools
        .route("/v1/tools", post(tools::create_tool).get(tools::list_tools))
        // Models
        .route("/v1/models", get(models::list_models))
        // Providers
        .route("/v1/providers",          post(providers::add_provider).get(providers::list_providers))
        .route("/v1/providers/presets",  get(providers::list_presets))
        .route("/v1/providers/:name",    delete(providers::remove_provider));

    // Merge and apply auth middleware to everything
    Router::new()
        .merge(inference)
        .merge(rest)
        .with_state(state.clone())
        .layer(middleware::from_fn_with_state(state, auth::auth_middleware))
}
