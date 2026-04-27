//! Standalone shared-memory-block endpoints.
//!
//! These routes operate on blocks independently of any agent, enabling
//! Letta-style cross-agent memory sharing.

use axum::{
    Json,
    extract::{Path, Query, State},
    http::StatusCode,
};
use serde_json::{Value, json};
use std::collections::HashMap;

use crate::server::state::AppState;
use cade_store::sqlite;

fn server_err(detail: String) -> (StatusCode, Json<Value>) {
    let request_id = uuid::Uuid::new_v4().to_string();
    tracing::error!(request_id = %request_id, detail = %detail, "500 Internal Server Error");
    (
        StatusCode::INTERNAL_SERVER_ERROR,
        Json(json!({ "error": "internal error", "request_id": request_id })),
    )
}

fn block_to_json(b: &sqlite::BlockInfo) -> Value {
    json!({
        "id": b.id,
        "label": b.label,
        "value": b.value,
        "description": b.description,
        "tier": b.tier,
        "max_chars": b.max_chars,
        "updated_at": b.updated_at,
    })
}

// ── POST /v1/blocks ──────────────────────────────────────────────────────────

/// Create a standalone block (not attached to any agent).
pub async fn create_block(
    State(state): State<AppState>,
    Json(body): Json<Value>,
) -> Result<(StatusCode, Json<Value>), (StatusCode, Json<Value>)> {
    let label = body["label"]
        .as_str()
        .unwrap_or("")
        .trim()
        .to_string();
    let value = body["value"].as_str().unwrap_or("").to_string();
    let description = body["description"].as_str();
    let max_chars = body["max_chars"].as_u64().map(|n| n as usize);

    if label.is_empty() {
        return Err((
            StatusCode::BAD_REQUEST,
            Json(json!({ "detail": "'label' is required" })),
        ));
    }

    let block_id =
        sqlite::create_standalone_block(&state.db, &label, &value, description, max_chars)
            .map_err(|e| server_err(e.to_string()))?;

    let info = sqlite::get_block_by_id(&state.db, &block_id)
        .map_err(|e| server_err(e.to_string()))?
        .expect("just-created block must exist");

    Ok((StatusCode::CREATED, Json(block_to_json(&info))))
}

// ── GET /v1/blocks ───────────────────────────────────────────────────────────

/// List all blocks. Optional `?label=` filter.
pub async fn list_blocks(
    State(state): State<AppState>,
    Query(params): Query<HashMap<String, String>>,
) -> Result<Json<Value>, (StatusCode, Json<Value>)> {
    let label_filter = params.get("label").map(String::as_str);
    let blocks = sqlite::list_all_blocks(&state.db, label_filter)
        .map_err(|e| server_err(e.to_string()))?;
    let arr: Vec<Value> = blocks.iter().map(block_to_json).collect();
    Ok(Json(json!({ "blocks": arr })))
}

// ── GET /v1/blocks/:block_id ─────────────────────────────────────────────────

/// Get a single block by ID.
pub async fn get_block(
    State(state): State<AppState>,
    Path(block_id): Path<String>,
) -> Result<Json<Value>, (StatusCode, Json<Value>)> {
    let info = sqlite::get_block_by_id(&state.db, &block_id)
        .map_err(|e| server_err(e.to_string()))?;
    match info {
        Some(b) => {
            let agents = sqlite::list_agents_for_block(&state.db, &block_id)
                .map_err(|e| server_err(e.to_string()))?;
            let mut obj = block_to_json(&b);
            obj["agents"] = json!(agents);
            Ok(Json(obj))
        }
        None => Err((
            StatusCode::NOT_FOUND,
            Json(json!({ "detail": format!("Block '{block_id}' not found") })),
        )),
    }
}

// ── PUT /v1/blocks/:block_id ─────────────────────────────────────────────────

/// Update a block's value/description/max_chars directly (by ID, agent-agnostic).
pub async fn update_block(
    State(state): State<AppState>,
    Path(block_id): Path<String>,
    Json(body): Json<Value>,
) -> Result<Json<Value>, (StatusCode, Json<Value>)> {
    let existing = sqlite::get_block_by_id(&state.db, &block_id)
        .map_err(|e| server_err(e.to_string()))?;
    let Some(existing) = existing else {
        return Err((
            StatusCode::NOT_FOUND,
            Json(json!({ "detail": format!("Block '{block_id}' not found") })),
        ));
    };

    let value = body["value"].as_str().unwrap_or(&existing.value);
    let description = body["description"]
        .as_str()
        .unwrap_or(&existing.description);
    let max_chars = body["max_chars"]
        .as_u64()
        .map(|n| n as i64)
        .or(existing.max_chars.map(|n| n as i64));
    let ts = chrono::Utc::now().timestamp();

    {
        let conn = state.db.lock();
        conn.execute(
            "UPDATE shared_memory_blocks
             SET value = ?1, description = ?2, max_chars = ?3, updated_at = ?4
             WHERE id = ?5",
            rusqlite::params![value, description, max_chars, ts, block_id],
        )
        .map_err(|e| server_err(e.to_string()))?;
    }

    let updated = sqlite::get_block_by_id(&state.db, &block_id)
        .map_err(|e| server_err(e.to_string()))?
        .expect("just-updated block must exist");
    Ok(Json(block_to_json(&updated)))
}

// ── DELETE /v1/blocks/:block_id ──────────────────────────────────────────────

/// Permanently delete a block (cascades from all agents).
pub async fn delete_block(
    State(state): State<AppState>,
    Path(block_id): Path<String>,
) -> Result<StatusCode, (StatusCode, Json<Value>)> {
    let found = sqlite::delete_block_permanently(&state.db, &block_id)
        .map_err(|e| server_err(e.to_string()))?;
    if found {
        Ok(StatusCode::NO_CONTENT)
    } else {
        Err((
            StatusCode::NOT_FOUND,
            Json(json!({ "detail": format!("Block '{block_id}' not found") })),
        ))
    }
}

// ── POST /v1/agents/:id/blocks/attach ────────────────────────────────────────

/// Attach an existing block (by ID) to an agent.
pub async fn attach_block(
    State(state): State<AppState>,
    Path(agent_id): Path<String>,
    Json(body): Json<Value>,
) -> Result<StatusCode, (StatusCode, Json<Value>)> {
    let block_id = body["block_id"]
        .as_str()
        .unwrap_or("")
        .trim()
        .to_string();
    if block_id.is_empty() {
        return Err((
            StatusCode::BAD_REQUEST,
            Json(json!({ "detail": "'block_id' is required" })),
        ));
    }

    // Verify block exists
    let exists = sqlite::get_block_by_id(&state.db, &block_id)
        .map_err(|e| server_err(e.to_string()))?;
    if exists.is_none() {
        return Err((
            StatusCode::NOT_FOUND,
            Json(json!({ "detail": format!("Block '{block_id}' not found") })),
        ));
    }

    sqlite::link_shared_memory_block(&state.db, &agent_id, &block_id)
        .map_err(|e| server_err(e.to_string()))?;
    Ok(StatusCode::NO_CONTENT)
}

// ── POST /v1/agents/:id/blocks/detach ────────────────────────────────────────

/// Detach a block (by ID) from an agent. The block itself is NOT deleted.
pub async fn detach_block(
    State(state): State<AppState>,
    Path(agent_id): Path<String>,
    Json(body): Json<Value>,
) -> Result<StatusCode, (StatusCode, Json<Value>)> {
    let block_id = body["block_id"]
        .as_str()
        .unwrap_or("")
        .trim()
        .to_string();
    if block_id.is_empty() {
        return Err((
            StatusCode::BAD_REQUEST,
            Json(json!({ "detail": "'block_id' is required" })),
        ));
    }

    let found = sqlite::unlink_shared_memory_block(&state.db, &agent_id, &block_id)
        .map_err(|e| server_err(e.to_string()))?;
    if found {
        Ok(StatusCode::NO_CONTENT)
    } else {
        Err((
            StatusCode::NOT_FOUND,
            Json(json!({ "detail": format!("Block '{block_id}' not attached to agent '{agent_id}'") })),
        ))
    }
}
