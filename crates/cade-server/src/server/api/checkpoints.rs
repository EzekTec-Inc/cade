/// Checkpoint API handlers.
use axum::{
    Json,
    extract::{Path, State},
    http::StatusCode,
};
use serde::Deserialize;
use serde_json::{Value, json};
use uuid::Uuid;

use crate::server::state::AppState;

// region:    --- Types

#[derive(Debug, serde::Serialize, serde::Deserialize)]
pub struct CheckpointRow {
    pub id: String,
    pub agent_id: String,
    pub conversation_id: Option<String>,
    pub branch_id: String,
    pub label: Option<String>,
    pub description: Option<String>,
    pub created_at: i64,
    pub git_stash_ref: Option<String>,
    pub git_commit_hash: Option<String>,
    pub parent_id: Option<String>,
}

#[derive(Deserialize)]
pub struct CreateCheckpointBody {
    pub label: Option<String>,
    pub description: Option<String>,
    pub conversation_id: Option<String>,
    pub branch_id: Option<String>,
    pub git_stash_ref: Option<String>,
    pub git_commit_hash: Option<String>,
    pub parent_id: Option<String>,
}

// endregion: --- Types

// region:    --- Handlers

/// POST /v1/agents/:id/checkpoints
pub async fn create_checkpoint(
    State(state): State<AppState>,
    Path(agent_id): Path<String>,
    Json(body): Json<CreateCheckpointBody>,
) -> Result<Json<Value>, (StatusCode, Json<Value>)> {
    let id = format!("cp-{}", Uuid::new_v4());
    let conn = state.db.lock();
    let now = unix_ts();
    conn.execute(
        "INSERT INTO checkpoints (id, agent_id, conversation_id, branch_id, label, description, created_at, git_stash_ref, git_commit_hash, parent_id)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)",
        rusqlite::params![
            id, agent_id,
            body.conversation_id, body.branch_id.as_deref().unwrap_or("main"),
            body.label, body.description, now,
            body.git_stash_ref, body.git_commit_hash, body.parent_id
        ],
    ).map_err(db_err)?;
    Ok(Json(json!({ "id": id, "created_at": now })))
}

/// GET /v1/agents/:id/checkpoints
pub async fn list_checkpoints(
    State(state): State<AppState>,
    Path(agent_id): Path<String>,
) -> Result<Json<Value>, (StatusCode, Json<Value>)> {
    let conn = state.db.lock();
    let mut stmt = conn
        .prepare(
            "SELECT id, agent_id, conversation_id, branch_id, label, description, created_at,
                git_stash_ref, git_commit_hash, parent_id
         FROM checkpoints WHERE agent_id = ?1 ORDER BY created_at DESC LIMIT 200",
        )
        .map_err(db_err)?;
    let rows: Vec<Value> = stmt
        .query_map(rusqlite::params![agent_id], |r| {
            Ok(json!({
                "id":              r.get::<_, String>(0)?,
                "agent_id":        r.get::<_, String>(1)?,
                "conversation_id": r.get::<_, Option<String>>(2)?,
                "branch_id":       r.get::<_, String>(3)?,
                "label":           r.get::<_, Option<String>>(4)?,
                "description":     r.get::<_, Option<String>>(5)?,
                "created_at":      r.get::<_, i64>(6)?,
                "git_stash_ref":   r.get::<_, Option<String>>(7)?,
                "git_commit_hash": r.get::<_, Option<String>>(8)?,
                "parent_id":       r.get::<_, Option<String>>(9)?,
            }))
        })
        .map_err(db_err)?
        .filter_map(|r| r.ok())
        .collect();
    Ok(Json(json!(rows)))
}

/// GET /v1/agents/:id/checkpoints/:cp_id
pub async fn get_checkpoint_handler(
    State(state): State<AppState>,
    Path((agent_id, cp_id)): Path<(String, String)>,
) -> Result<Json<Value>, (StatusCode, Json<Value>)> {
    let conn = state.db.lock();
    let row = conn
        .query_row(
            "SELECT id, agent_id, conversation_id, branch_id, label, description, created_at,
                git_stash_ref, git_commit_hash, parent_id
         FROM checkpoints WHERE id = ?1 AND agent_id = ?2",
            rusqlite::params![cp_id, agent_id],
            |r| {
                Ok(json!({
                    "id":              r.get::<_, String>(0)?,
                    "conversation_id": r.get::<_, Option<String>>(2)?,
                    "branch_id":       r.get::<_, String>(3)?,
                    "label":           r.get::<_, Option<String>>(4)?,
                    "description":     r.get::<_, Option<String>>(5)?,
                    "created_at":      r.get::<_, i64>(6)?,
                }))
            },
        )
        .map_err(|_| {
            (
                StatusCode::NOT_FOUND,
                Json(json!({ "detail": "Checkpoint not found" })),
            )
        })?;
    Ok(Json(row))
}

/// DELETE /v1/agents/:id/checkpoints/:cp_id
pub async fn delete_checkpoint_handler(
    State(state): State<AppState>,
    Path((agent_id, cp_id)): Path<(String, String)>,
) -> Result<Json<Value>, (StatusCode, Json<Value>)> {
    let conn = state.db.lock();
    let n = conn
        .execute(
            "DELETE FROM checkpoints WHERE id = ?1 AND agent_id = ?2",
            rusqlite::params![cp_id, agent_id],
        )
        .map_err(db_err)?;
    Ok(Json(json!({ "deleted": n > 0 })))
}

/// POST /v1/agents/:id/checkpoints/:cp_id/restore
/// Marks the checkpoint as "current" — actual git/message restore is handled by the CLI.
pub async fn restore_checkpoint_handler(
    State(state): State<AppState>,
    Path((agent_id, cp_id)): Path<(String, String)>,
) -> Result<Json<Value>, (StatusCode, Json<Value>)> {
    let conn = state.db.lock();
    // Verify checkpoint exists
    let exists: bool = conn
        .query_row(
            "SELECT COUNT(*) FROM checkpoints WHERE id = ?1 AND agent_id = ?2",
            rusqlite::params![cp_id, agent_id],
            |r| r.get::<_, i64>(0),
        )
        .unwrap_or(0)
        > 0;
    if !exists {
        return Err((
            StatusCode::NOT_FOUND,
            Json(json!({ "detail": "Checkpoint not found" })),
        ));
    }
    Ok(Json(
        json!({ "checkpoint_id": cp_id, "status": "restore_requested" }),
    ))
}

// endregion: --- Handlers

// region:    --- Support

fn unix_ts() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs() as i64
}

/// Public re-export of `unix_ts` for use by server-side meta-tool handlers
/// that need to insert timestamps without going through the HTTP layer.
pub fn unix_ts_pub() -> i64 {
    unix_ts()
}

fn db_err(e: rusqlite::Error) -> (StatusCode, Json<Value>) {
    (
        StatusCode::INTERNAL_SERVER_ERROR,
        Json(json!({ "detail": e.to_string() })),
    )
}

// endregion: --- Support
