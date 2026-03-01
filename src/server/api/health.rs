use axum::{extract::State, Json};
use serde_json::{json, Value};
use crate::server::state::AppState;

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
    let mut available: Vec<&str> = vec!["ollama"]; // always available
    if state.config.anthropic_api_key.is_some() { available.push("anthropic"); }
    if state.config.openai_api_key.is_some()    { available.push("openai"); }
    if state.config.google_api_key.is_some()    { available.push("gemini"); }

    Json(json!({
        "provider":            state.config.llm_provider.to_string(),
        "default_model":       state.config.default_model,
        "available_providers": available,
        "version":             env!("CARGO_PKG_VERSION")
    }))
}
