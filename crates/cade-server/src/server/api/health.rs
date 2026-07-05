use crate::server::state::AppState;
use axum::{Json, extract::State, http::StatusCode};
use serde_json::{Value, json};

pub async fn get_health(State(_state): State<AppState>) -> Json<Value> {
    Json(json!({
        "status": "ok",
        "server": "cade-server",
        "version": env!("CARGO_PKG_VERSION")
    }))
}

/// Returns the server's active provider and default model so the CLI can
/// auto-select the right model when creating an agent.
pub async fn get_config(State(state): State<AppState>) -> Json<Value> {
    let available = state.llm_router.read().await.provider_names();
    Json(json!({
        "provider":            state.config.llm_provider.to_string(),
        "default_model":       state.config.default_model,
        "available_providers": available,
        "version":             env!("CARGO_PKG_VERSION")
    }))
}

pub async fn defragment_database_handler(
    State(state): State<AppState>,
) -> Result<Json<Value>, (StatusCode, String)> {
    crate::server::defragment::defragment_database(&state).await;
    Ok(Json(
        json!({ "status": "ok", "message": "Database defragmentation and GC completed." }),
    ))
}
