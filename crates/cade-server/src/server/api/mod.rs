// region:    --- Modules

pub mod agents;
pub mod artifacts;
pub mod auth;
pub mod checkpoints;
pub mod evals;
pub mod health;
pub mod memory_evidence;
pub mod messages;
pub mod models;
pub mod providers;
pub mod proxy;
pub mod runs;
pub mod tool_executions;
pub mod tools;

use crate::server::{rate_limit::rate_limit_middleware, state::AppState};
use axum::{
    Router, middleware,
    routing::{delete, get, post, put},
};

// endregion: --- Modules

pub fn router(state: AppState) -> Router {
    // -- Inference routes (rate-limited)
    let inference = Router::new()
        .route(
            "/v1/agents/:id/messages",
            post(messages::send_message)
                .delete(agents::clear_messages_handler)
                .get(agents::search_messages_handler),
        )
        .route(
            "/v1/agents/:id/messages/latest",
            get(agents::latest_assistant_message),
        )
        .route(
            "/v1/agents/:id/messages/stream",
            post(messages::stream_message),
        )
        .layer(middleware::from_fn_with_state(
            state.clone(),
            rate_limit_middleware,
        ));

    // -- All other routes (no rate limit)
    let rest = Router::new()
        // Health + server config
        .route("/v1/health", get(health::get_health))
        .route("/v1/config", get(health::get_config))
        // Real context-window stats (D2)
        .route(
            "/v1/agents/:id/context",
            get(messages::get_context_stats_handler),
        )
        // Agents
        .route(
            "/v1/agents",
            post(agents::create_agent).get(agents::list_agents),
        )
        .route(
            "/v1/agents/:id",
            get(agents::get_agent)
                .delete(agents::delete_agent)
                .patch(agents::patch_agent),
        )
        // Agent tools
        .route(
            "/v1/agents/:id/tools",
            get(agents::get_agent_tools)
                .post(agents::attach_tools)
                .delete(agents::detach_tools),
        )
        // Agent memory
        .route("/v1/agents/:id/memory", get(agents::search_memory_handler))
        .route(
            "/v1/agents/:id/archival",
            post(agents::insert_archival_memory_handler),
        )
        .route(
            "/v1/agents/:id/archival/search",
            get(agents::search_archival_memory_handler),
        )
        .route(
            "/v1/agents/:id/memory/:label",
            put(agents::upsert_memory).delete(agents::delete_memory),
        )
        .route(
            "/v1/agents/:id/memory/:label/tier",
            put(agents::set_memory_tier_handler),
        )
        .route(
            "/v1/agents/:id/memory/:label/history",
            get(agents::get_memory_history),
        )
        .route(
            "/v1/agents/:id/memory/:label/restore/:rev_id",
            put(agents::restore_memory_revision),
        )
        // Memory provenance + reflection
        .route(
            "/v1/agents/:id/memory/:label/evidence",
            get(memory_evidence::list_evidence).post(memory_evidence::add_evidence),
        )
        .route(
            "/v1/agents/:id/memory/:label/why",
            get(memory_evidence::memory_why),
        )
        .route(
            "/v1/agents/:id/reflect",
            post(memory_evidence::trigger_reflect),
        )
        .route(
            "/v1/agents/:id/reflection",
            get(memory_evidence::list_reflection),
        )
        // Conversations
        .route(
            "/v1/agents/:id/conversations",
            get(agents::list_conversations).post(agents::create_conversation),
        )
        .route(
            "/v1/agents/:id/conversations/:conv_id",
            delete(agents::delete_conversation),
        )
        // Runs (background mode)
        .route("/v1/runs/:run_id", get(runs::get_run))
        .route("/v1/runs/:run_id/stream", get(runs::stream_run))
        // Tool execution log
        .route(
            "/v1/agents/:id/tool_executions",
            post(tool_executions::log_tool_execution),
        )
        // Checkpoints
        .route(
            "/v1/agents/:id/checkpoints",
            post(checkpoints::create_checkpoint).get(checkpoints::list_checkpoints),
        )
        .route(
            "/v1/agents/:id/checkpoints/:cp_id",
            get(checkpoints::get_checkpoint_handler).delete(checkpoints::delete_checkpoint_handler),
        )
        .route(
            "/v1/agents/:id/checkpoints/:cp_id/restore",
            post(checkpoints::restore_checkpoint_handler),
        )
        // Artifacts
        .route(
            "/v1/agents/:id/artifacts",
            post(artifacts::create_artifact).get(artifacts::list_artifacts),
        )
        .route(
            "/v1/agents/:id/artifacts/:art_id",
            get(artifacts::get_artifact_handler).delete(artifacts::delete_artifact_handler),
        )
        // Evals
        .route(
            "/v1/evals/tasks",
            post(evals::create_eval_task).get(evals::list_eval_tasks),
        )
        .route(
            "/v1/evals/runs",
            post(evals::create_eval_run_handler).get(evals::list_eval_runs),
        )
        .route(
            "/v1/evals/runs/:id",
            get(evals::get_eval_run).patch(evals::update_eval_run_handler),
        )
        // Tools
        .route("/v1/tools", post(tools::create_tool).get(tools::list_tools))
        // Models
        .route("/v1/models", get(models::list_models))
        // Stream
        .route("/v1/stream", get(proxy::stream_http_handler))
        // Providers
        .route(
            "/v1/providers",
            post(providers::add_provider).get(providers::list_providers),
        )
        .route("/v1/providers/presets", get(providers::list_presets))
        .route("/v1/providers/:name", delete(providers::remove_provider));

    // Merge and apply auth middleware to everything
    Router::new()
        .merge(inference)
        .merge(rest)
        .with_state(state.clone())
        .layer(middleware::from_fn_with_state(state, auth::auth_middleware))
}
