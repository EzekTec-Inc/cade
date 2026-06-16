/// Eval harness API handlers.
///
/// Eval tasks describe what to test; eval runs track execution results.
use axum::{
    Json,
    extract::{Path, State},
    http::StatusCode,
};
use serde_json::{Value, json};
use uuid::Uuid;

// region:    --- Types

#[derive(serde::Deserialize)]
pub struct CreateEvalTaskBody {
    pub name: String,
    pub description: Option<String>,
    pub prompt: String,
    pub expected_json: Option<Value>,
    pub tags: Option<Vec<String>>,
}

#[derive(serde::Deserialize)]
pub struct CreateEvalRunBody {
    pub task_id: String,
    pub agent_id: Option<String>,
    pub model: Option<String>,
}

// endregion: --- Types

// region:    --- Handlers

/// POST /v1/evals/tasks
pub async fn create_eval_task(
    State(state): State<crate::server::state::AppState>,
    Json(body): Json<CreateEvalTaskBody>,
) -> Result<Json<Value>, (StatusCode, Json<Value>)> {
    let id = format!("et-{}", Uuid::new_v4());
    let now = unix_ts();
    let tags_json = body
        .tags
        .map(|t| serde_json::to_string(&t).unwrap_or_else(|_| "[]".to_string()))
        .unwrap_or_else(|| "[]".to_string());
    let expected = body
        .expected_json
        .map(|v| serde_json::to_string(&v).unwrap_or_default());

    let conn = state.db.get().map_err(db_err)?;
    conn.execute(
        "INSERT INTO eval_tasks (id, name, description, prompt, expected_json, tags_json, created_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
        rusqlite::params![id, body.name, body.description, body.prompt, expected, tags_json, now],
    ).map_err(db_err)?;
    Ok(Json(
        json!({ "id": id, "name": body.name, "created_at": now }),
    ))
}

/// GET /v1/evals/tasks
pub async fn list_eval_tasks(
    State(state): State<crate::server::state::AppState>,
) -> Result<Json<Value>, (StatusCode, Json<Value>)> {
    let conn = state.db.get().map_err(db_err)?;
    let mut stmt = conn.prepare(
        "SELECT id, name, description, created_at FROM eval_tasks ORDER BY created_at DESC LIMIT 200"
    ).map_err(db_err)?;
    let rows: Vec<Value> = stmt
        .query_map([], |r| {
            Ok(json!({
                "id":          r.get::<_, String>(0)?,
                "name":        r.get::<_, String>(1)?,
                "description": r.get::<_, Option<String>>(2)?,
                "created_at":  r.get::<_, i64>(3)?,
            }))
        })
        .map_err(db_err)?
        .filter_map(|r| r.ok())
        .collect();
    Ok(Json(json!(rows)))
}

/// POST /v1/evals/runs
pub async fn create_eval_run_handler(
    State(state): State<crate::server::state::AppState>,
    Json(body): Json<CreateEvalRunBody>,
) -> Result<Json<Value>, (StatusCode, Json<Value>)> {
    let id = format!("er-{}", Uuid::new_v4());
    let now = unix_ts();
    let conn = state.db.get().map_err(db_err)?;
    conn.execute(
        "INSERT INTO eval_runs (id, task_id, agent_id, model, status, created_at)
         VALUES (?1, ?2, ?3, ?4, 'pending', ?5)",
        rusqlite::params![id, body.task_id, body.agent_id, body.model, now],
    )
    .map_err(db_err)?;
    Ok(Json(
        json!({ "id": id, "status": "pending", "created_at": now }),
    ))
}

/// GET /v1/evals/runs
pub async fn list_eval_runs(
    State(state): State<crate::server::state::AppState>,
) -> Result<Json<Value>, (StatusCode, Json<Value>)> {
    let conn = state.db.get().map_err(db_err)?;
    let mut stmt = conn.prepare(
        "SELECT id, task_id, agent_id, model, status, score, duration_ms, created_at, completed_at
         FROM eval_runs ORDER BY created_at DESC LIMIT 500"
    ).map_err(db_err)?;
    let rows: Vec<Value> = stmt
        .query_map([], |r| {
            Ok(json!({
                "id":           r.get::<_, String>(0)?,
                "task_id":      r.get::<_, String>(1)?,
                "agent_id":     r.get::<_, Option<String>>(2)?,
                "model":        r.get::<_, Option<String>>(3)?,
                "status":       r.get::<_, String>(4)?,
                "score":        r.get::<_, Option<f64>>(5)?,
                "duration_ms":  r.get::<_, Option<i64>>(6)?,
                "created_at":   r.get::<_, i64>(7)?,
                "completed_at": r.get::<_, Option<i64>>(8)?,
            }))
        })
        .map_err(db_err)?
        .filter_map(|r| r.ok())
        .collect();
    Ok(Json(json!(rows)))
}

/// GET /v1/evals/runs/:id
pub async fn get_eval_run(
    State(state): State<crate::server::state::AppState>,
    Path(run_id): Path<String>,
) -> Result<Json<Value>, (StatusCode, Json<Value>)> {
    let conn = state.db.get().map_err(db_err)?;
    let row = conn.query_row(
        "SELECT id, task_id, agent_id, model, status, score, result_json, duration_ms, created_at, completed_at
         FROM eval_runs WHERE id = ?1",
        rusqlite::params![run_id],
        |r| Ok(json!({
            "id":           r.get::<_, String>(0)?,
            "task_id":      r.get::<_, String>(1)?,
            "agent_id":     r.get::<_, Option<String>>(2)?,
            "model":        r.get::<_, Option<String>>(3)?,
            "status":       r.get::<_, String>(4)?,
            "score":        r.get::<_, Option<f64>>(5)?,
            "result":       r.get::<_, Option<String>>(6)?,
            "duration_ms":  r.get::<_, Option<i64>>(7)?,
            "created_at":   r.get::<_, i64>(8)?,
            "completed_at": r.get::<_, Option<i64>>(9)?,
        }))
    ).map_err(|_| (StatusCode::NOT_FOUND, Json(json!({ "detail": "Eval run not found" }))))?;
    Ok(Json(row))
}

/// PATCH /v1/evals/runs/:id
pub async fn update_eval_run_handler(
    State(state): State<crate::server::state::AppState>,
    Path(run_id): Path<String>,
    Json(body): Json<Value>,
) -> Result<Json<Value>, (StatusCode, Json<Value>)> {
    let conn = state.db.get().map_err(db_err)?;

    // Build dynamic UPDATE query based on provided fields
    let mut sets = Vec::new();
    let mut params: Vec<Box<dyn rusqlite::ToSql>> = Vec::new();
    let mut param_idx = 1;

    if let Some(status) = body.get("status").and_then(|v| v.as_str()) {
        sets.push(format!("status = ?{param_idx}"));
        params.push(Box::new(status.to_string()));
        param_idx += 1;
    }
    if let Some(score) = body.get("score").and_then(|v| v.as_f64()) {
        sets.push(format!("score = ?{param_idx}"));
        params.push(Box::new(score));
        param_idx += 1;
    }
    if let Some(result_json) = body.get("result_json").and_then(|v| v.as_str()) {
        sets.push(format!("result_json = ?{param_idx}"));
        params.push(Box::new(result_json.to_string()));
        param_idx += 1;
    }
    if let Some(duration_ms) = body.get("duration_ms").and_then(|v| v.as_i64()) {
        sets.push(format!("duration_ms = ?{param_idx}"));
        params.push(Box::new(duration_ms));
        param_idx += 1;
    }
    if let Some(completed_at) = body.get("completed_at").and_then(|v| v.as_i64()) {
        sets.push(format!("completed_at = ?{param_idx}"));
        params.push(Box::new(completed_at));
        param_idx += 1;
    }

    if sets.is_empty() {
        return Ok(Json(json!({"updated": false})));
    }

    let query = format!(
        "UPDATE eval_runs SET {} WHERE id = ?{param_idx}",
        sets.join(", ")
    );
    params.push(Box::new(run_id.clone()));

    // Convert Vec<Box<dyn ToSql>> to a slice of &dyn ToSql
    let sql_params: Vec<&dyn rusqlite::ToSql> = params.iter().map(|b| b.as_ref()).collect();

    let n = conn
        .execute(&query, rusqlite::params_from_iter(sql_params))
        .map_err(db_err)?;

    Ok(Json(json!({"updated": n > 0})))
}

// endregion: --- Handlers

// region:    --- Support

fn unix_ts() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs() as i64
}

fn db_err(e: impl std::fmt::Display) -> (StatusCode, Json<Value>) {
    (
        StatusCode::INTERNAL_SERVER_ERROR,
        Json(json!({ "detail": e.to_string() })),
    )
}

// endregion: --- Support

#[cfg(test)]
#[path = "evals_test.rs"]
mod tests;
