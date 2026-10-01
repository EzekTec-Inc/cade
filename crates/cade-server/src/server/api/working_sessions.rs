use crate::server::state::AppState;
use axum::{
    Json,
    extract::{Path, State},
    http::StatusCode,
};
use serde_json::{Value, json};

#[derive(serde::Deserialize)]
pub struct OpenWorkingSession {
    pub cwd: std::path::PathBuf,
}

pub async fn open(
    State(state): State<AppState>,
    Json(request): Json<OpenWorkingSession>,
) -> Result<Json<Value>, (StatusCode, String)> {
    let id = state
        .permission_sessions
        .open(&request.cwd)
        .map_err(|error| (StatusCode::BAD_REQUEST, error))?;
    Ok(Json(
        json!({"id": id, "lease_seconds": crate::server::permission_sessions::WORKING_SESSION_LEASE.as_secs()}),
    ))
}

pub async fn renew(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Result<Json<Value>, (StatusCode, String)> {
    state
        .permission_sessions
        .renew(&id)
        .map_err(|error| (StatusCode::NOT_FOUND, error))?;
    Ok(Json(json!({"id": id})))
}

pub async fn close(State(state): State<AppState>, Path(id): Path<String>) -> Json<Value> {
    state.permission_sessions.close(&id);
    Json(json!({"id": id, "closed": true}))
}
