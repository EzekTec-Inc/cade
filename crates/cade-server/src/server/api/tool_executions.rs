/// Tool execution log handler.
use axum::{
    Json,
    extract::{Path, State},
    http::StatusCode,
};
use serde_json::{Value, json};
use uuid::Uuid;

/// POST /v1/agents/:id/tool_executions
pub async fn log_tool_execution(
    State(state): State<crate::server::state::AppState>,
    Path(agent_id): Path<String>,
    Json(body): Json<Value>,
) -> Result<Json<Value>, (StatusCode, Json<Value>)> {
    let id = format!("te-{}", Uuid::new_v4());
    let now = unix_ts();
    let conn = state.db.lock().map_err(|e| {
        (
            axum::http::StatusCode::INTERNAL_SERVER_ERROR,
            axum::Json(serde_json::json!({"error": format!("db lock poisoned: {e}")})),
        )
    })?;
    let _ = conn.execute(
        "INSERT OR IGNORE INTO tool_executions
         (id, agent_id, conversation_id, tool_name, arguments_json, output, is_error, duration_ms, created_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
        rusqlite::params![
            id, agent_id,
            body["conversation_id"].as_str(),
            body["tool_name"].as_str().unwrap_or("unknown"),
            body["arguments_json"].as_str().unwrap_or("{}"),
            body["output"].as_str().unwrap_or(""),
            body["is_error"].as_bool().unwrap_or(false) as i64,
            body["duration_ms"].as_i64().unwrap_or(0),
            now,
        ],
    );
    Ok(Json(json!({ "id": id })))
}

fn unix_ts() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs() as i64
}
