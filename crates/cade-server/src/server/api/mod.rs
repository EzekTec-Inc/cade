// region:    --- Modules

pub mod agents;
pub mod artifacts;
pub mod auth;
pub mod blocks;
pub mod checkpoints;
pub mod compact;
pub mod complete;
pub mod context_stats;
pub mod csrf;
pub mod dashboard;
pub mod dashboard_assets;
pub mod edit;
pub mod evals;
pub mod health;
pub mod mcp;
pub mod memory_evidence;
pub mod messages;
pub mod models;
pub mod providers;
pub mod proxy;
pub mod run;
pub mod runs;
pub mod skills;
pub mod tool_executions;
pub mod tools;
pub mod workflows;

use crate::server::{rate_limit::rate_limit_middleware, state::AppState};
use axum::{
    Router,
    extract::DefaultBodyLimit,
    middleware,
    routing::{delete, get, post, put},
};

// endregion: --- Modules

pub fn router(state: AppState) -> Router {
    // -- Inference routes (rate-limited)
    let inference = Router::new()
        .route(
            "/v1/agents/{id}/messages",
            post(messages::send_message)
                .delete(agents::clear_messages_handler)
                .get(agents::search_messages_handler),
        )
        .route(
            "/v1/agents/{id}/messages/latest",
            get(agents::latest_assistant_message),
        )
        .route(
            "/v1/agents/{id}/messages/stream",
            post(messages::stream_message),
        )
        .route("/v1/agents/{id}/complete", post(complete::complete))
        .route("/v1/agents/{id}/edit", post(edit::edit))
        .route("/v1/agents/{id}/run", post(run::run_agent))
        .layer(middleware::from_fn_with_state(
            state.clone(),
            rate_limit_middleware,
        ));

    // -- All other routes (no rate limit)
    let rest = Router::new()
        // Health + server config
        .route("/v1/health", get(health::get_health))
        .route("/v1/config", get(health::get_config))
        .route(
            "/v1/workflows/{workflow_name}",
            post(workflows::dispatch_workflow),
        )
        // Dashboard (public, unauthenticated — see auth.rs for exemption)
        // Gzip-compressed so the ~7 MB .wasm transfers as ~2.7 MB.
        // Both `/dashboard` and `/dashboard/` serve index.html — the nest
        // route `/` matches the no-slash path, and we add an explicit
        // top-level route for the trailing-slash form (axum 0.7 nests do
        // not strip trailing slashes automatically).
        .route("/dashboard/", get(dashboard::get_dashboard))
        .nest(
            "/dashboard",
            Router::new()
                .route("/", get(dashboard::get_dashboard))
                .route("/{*path}", get(dashboard::get_dashboard_asset))
                .layer(tower_http::compression::CompressionLayer::new()),
        )
        // Real context-window stats (D2)
        .route(
            "/v1/agents/{id}/context",
            get(messages::get_context_stats_handler),
        )
        .route(
            "/v1/agents/{id}/context-breakdown",
            get(messages::get_context_breakdown_handler),
        )
        // Agents
        .route(
            "/v1/agents",
            post(agents::create_agent).get(agents::list_agents),
        )
        .route(
            "/v1/agents/{id}",
            get(agents::get_agent)
                .delete(agents::delete_agent)
                .patch(agents::patch_agent),
        )
        .route(
            "/v1/agents/{id}/events",
            post(agents::insert_event_handler).get(agents::query_events_handler),
        )
        .route("/v1/agents/{id}/metrics", get(agents::get_agent_metrics))
        .route(
            "/v1/agents/{id}/context_stats",
            get(context_stats::get_context_stats),
        )
        // Agent tools
        .route(
            "/v1/agents/{id}/tools",
            get(agents::get_agent_tools)
                .post(agents::attach_tools)
                .delete(agents::detach_tools),
        )
        // Agent memory
        .route("/v1/agents/{id}/memory", get(agents::search_memory_handler))
        // Standalone shared blocks
        .route(
            "/v1/blocks",
            post(blocks::create_block).get(blocks::list_blocks),
        )
        .route(
            "/v1/blocks/{block_id}",
            get(blocks::get_block)
                .put(blocks::update_block)
                .delete(blocks::delete_block),
        )
        .route("/v1/agents/{id}/blocks/attach", post(blocks::attach_block))
        .route("/v1/agents/{id}/blocks/detach", post(blocks::detach_block))
        .route(
            "/v1/agents/{id}/archival",
            post(agents::insert_archival_memory_handler),
        )
        .route(
            "/v1/agents/{id}/archival/search",
            get(agents::search_archival_memory_handler),
        )
        .route(
            "/v1/agents/{id}/memory/{label}",
            put(agents::upsert_memory).delete(agents::delete_memory),
        )
        .route(
            "/v1/agents/{id}/memory/{label}/tier",
            put(agents::set_memory_tier_handler),
        )
        .route(
            "/v1/agents/{id}/memory/{label}/history",
            get(agents::get_memory_history),
        )
        .route(
            "/v1/agents/{id}/memory/{label}/restore/{rev_id}",
            put(agents::restore_memory_revision),
        )
        .route(
            "/v1/agents/{id}/memory/export",
            post(agents::export_memory_handler),
        )
        // Memory provenance + reflection
        .route(
            "/v1/agents/{id}/memory/{label}/evidence",
            get(memory_evidence::list_evidence).post(memory_evidence::add_evidence),
        )
        .route(
            "/v1/agents/{id}/memory/{label}/why",
            get(memory_evidence::memory_why),
        )
        .route(
            "/v1/agents/{id}/reflect",
            post(memory_evidence::trigger_reflect),
        )
        .route("/v1/agents/{id}/compact", post(compact::compact_handler))
        .route(
            "/v1/agents/{id}/reflection",
            get(memory_evidence::list_reflection),
        )
        // Conversations
        .route(
            "/v1/agents/{id}/conversations",
            get(agents::list_conversations).post(agents::create_conversation),
        )
        .route(
            "/v1/agents/{id}/conversations/{conv_id}",
            delete(agents::delete_conversation),
        )
        // Runs (background mode)
        .route("/v1/runs/{run_id}", get(runs::get_run))
        .route("/v1/runs/{run_id}/stream", get(runs::stream_run))
        // Skills
        .route("/v1/skills", get(skills::list_all_skills))
        .route("/v1/agents/{id}/skills", get(skills::list_agent_skills))
        .route("/v1/agents/{id}/skills/load", post(skills::load_skill))
        .route("/v1/agents/{id}/skills/unload", post(skills::unload_skill))
        .route(
            "/v1/agents/{id}/skills/disable",
            post(skills::disable_skill),
        )
        .route("/v1/agents/{id}/skills/enable", post(skills::enable_skill))
        // Tool execution log
        .route(
            "/v1/agents/{id}/tool_executions",
            post(tool_executions::log_tool_execution),
        )
        // Checkpoints
        .route(
            "/v1/agents/{id}/checkpoints",
            post(checkpoints::create_checkpoint).get(checkpoints::list_checkpoints),
        )
        .route(
            "/v1/agents/{id}/checkpoints/{cp_id}",
            get(checkpoints::get_checkpoint_handler).delete(checkpoints::delete_checkpoint_handler),
        )
        .route(
            "/v1/agents/{id}/checkpoints/{cp_id}/restore",
            post(checkpoints::restore_checkpoint_handler),
        )
        // Artifacts
        .route(
            "/v1/agents/{id}/artifacts",
            post(artifacts::create_artifact).get(artifacts::list_artifacts),
        )
        .route(
            "/v1/agents/{id}/artifacts/{art_id}",
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
            "/v1/evals/runs/{id}",
            get(evals::get_eval_run).patch(evals::update_eval_run_handler),
        )
        // Tools
        .route("/v1/tools", post(tools::create_tool).get(tools::list_tools))
        // Models
        .route("/v1/models", get(models::list_models))
        // MCP servers
        .route("/v1/mcp", get(mcp::list_mcp_servers))
        // Stream
        .route("/v1/stream", get(proxy::stream_http_handler))
        // Providers
        .route(
            "/v1/providers",
            post(providers::add_provider).get(providers::list_providers),
        )
        .route("/v1/providers/presets", get(providers::list_presets))
        .route("/v1/providers/{name}", delete(providers::remove_provider));

    // Merge and apply middleware to everything.
    //
    // Layer order (outermost → innermost; request flows top-down):
    //   1. csrf_middleware  — P2-5: reject mutating requests with a
    //      non-localhost `Origin` header (defense-in-depth on top of
    //      bearer-token auth).  Absent Origin and safe methods pass.
    //   2. auth_middleware  — P1-1: bearer token required on all
    //      non-health routes.
    //   3. DefaultBodyLimit — P1-2: cap every request body at 8 MiB.
    Router::new()
        .merge(inference)
        .merge(rest)
        .with_state(state.clone())
        .layer(DefaultBodyLimit::max(8 * 1024 * 1024))
        .layer(middleware::from_fn_with_state(state, auth::auth_middleware))
        .layer(middleware::from_fn(csrf::csrf_middleware))
}

#[cfg(test)]
#[path = "router_test.rs"]
mod tests;
