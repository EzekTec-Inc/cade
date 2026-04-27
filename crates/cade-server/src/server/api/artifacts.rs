/// Artifact store API handlers.
///
/// Artifacts are binary or text blobs produced during agent runs:
/// screenshots, diffs, test reports, fetched documents, etc.
use axum::{
    Json,
    extract::{Path, State},
    http::StatusCode,
};
use serde_json::{Value, json};
use uuid::Uuid;

// region:    --- Types

#[derive(serde::Deserialize)]
pub struct CreateArtifactBody {
    pub kind: String, // 'screenshot'|'diff'|'log'|'test_report'|'pdf'|'fetched_doc'
    pub content_type: String, // MIME type
    pub data_text: Option<String>, // text artifacts
    pub metadata: Option<Value>, // arbitrary metadata
    pub run_id: Option<String>,
    pub tool_call_id: Option<String>,
}

// endregion: --- Types

// region:    --- Handlers

/// POST /v1/agents/:id/artifacts
pub async fn create_artifact(
    State(state): State<crate::server::state::AppState>,
    Path(agent_id): Path<String>,
    Json(body): Json<CreateArtifactBody>,
) -> Result<Json<Value>, (StatusCode, Json<Value>)> {
    let id = format!("art-{}", Uuid::new_v4());
    let now = unix_ts();
    let metadata_json = body
        .metadata
        .as_ref()
        .map(|m| serde_json::to_string(m).unwrap_or_default())
        .unwrap_or_else(|| "{}".to_string());
    let size_bytes = body.data_text.as_deref().map(|s| s.len()).unwrap_or(0) as i64;

    let conn = state.db.lock();
    conn.execute(
        "INSERT INTO artifacts (id, agent_id, run_id, tool_call_id, kind, content_type, data_text, metadata_json, size_bytes, created_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)",
        rusqlite::params![
            id, agent_id, body.run_id, body.tool_call_id,
            body.kind, body.content_type, body.data_text,
            metadata_json, size_bytes, now
        ],
    ).map_err(db_err)?;
    Ok(Json(json!({ "id": id, "created_at": now })))
}

/// GET /v1/agents/:id/artifacts
pub async fn list_artifacts(
    State(state): State<crate::server::state::AppState>,
    Path(agent_id): Path<String>,
) -> Result<Json<Value>, (StatusCode, Json<Value>)> {
    let conn = state.db.lock();
    let mut stmt = conn
        .prepare(
            "SELECT id, kind, content_type, size_bytes, created_at, run_id
         FROM artifacts WHERE agent_id = ?1 ORDER BY created_at DESC LIMIT 500",
        )
        .map_err(db_err)?;
    let rows: Vec<Value> = stmt
        .query_map(rusqlite::params![agent_id], |r| {
            Ok(json!({
                "id":           r.get::<_, String>(0)?,
                "kind":         r.get::<_, String>(1)?,
                "content_type": r.get::<_, String>(2)?,
                "size_bytes":   r.get::<_, i64>(3)?,
                "created_at":   r.get::<_, i64>(4)?,
                "run_id":       r.get::<_, Option<String>>(5)?,
            }))
        })
        .map_err(db_err)?
        .filter_map(|r| r.ok())
        .collect();
    Ok(Json(json!(rows)))
}

/// GET /v1/agents/:id/artifacts/:art_id
pub async fn get_artifact_handler(
    State(state): State<crate::server::state::AppState>,
    Path((agent_id, art_id)): Path<(String, String)>,
) -> Result<Json<Value>, (StatusCode, Json<Value>)> {
    let conn = state.db.lock();
    let row = conn.query_row(
        "SELECT id, kind, content_type, data_text, metadata_json, size_bytes, created_at
         FROM artifacts WHERE id = ?1 AND agent_id = ?2",
        rusqlite::params![art_id, agent_id],
        |r| Ok(json!({
            "id":           r.get::<_, String>(0)?,
            "kind":         r.get::<_, String>(1)?,
            "content_type": r.get::<_, String>(2)?,
            "data_text":    r.get::<_, Option<String>>(3)?,
            "metadata":     serde_json::from_str::<Value>(r.get::<_, String>(4)?.as_str()).unwrap_or_default(),
            "size_bytes":   r.get::<_, i64>(5)?,
            "created_at":   r.get::<_, i64>(6)?,
        }))
    ).map_err(|_| (StatusCode::NOT_FOUND, Json(json!({ "detail": "Artifact not found" }))))?;
    Ok(Json(row))
}

/// DELETE /v1/agents/:id/artifacts/:art_id
pub async fn delete_artifact_handler(
    State(state): State<crate::server::state::AppState>,
    Path((agent_id, art_id)): Path<(String, String)>,
) -> Result<Json<Value>, (StatusCode, Json<Value>)> {
    let conn = state.db.lock();
    let n = conn
        .execute(
            "DELETE FROM artifacts WHERE id = ?1 AND agent_id = ?2",
            rusqlite::params![art_id, agent_id],
        )
        .map_err(db_err)?;
    Ok(Json(json!({ "deleted": n > 0 })))
}

// endregion: --- Handlers

// region:    --- Support

fn unix_ts() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs() as i64
}

fn db_err(e: rusqlite::Error) -> (StatusCode, Json<Value>) {
    (
        StatusCode::INTERNAL_SERVER_ERROR,
        Json(json!({ "detail": e.to_string() })),
    )
}

// endregion: --- Support
