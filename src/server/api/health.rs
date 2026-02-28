use axum::{extract::State, Json};
use serde_json::{json, Value};
use crate::server::state::AppState;

pub async fn get_health(State(_state): State<AppState>) -> Json<Value> {
    Json(json!({ "status": "ok", "server": "cade-server", "version": env!("CARGO_PKG_VERSION") }))
}
