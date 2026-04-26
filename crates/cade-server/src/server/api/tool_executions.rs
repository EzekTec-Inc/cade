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
    let output_str = body["output"].as_str().unwrap_or("");
    let output_chars = output_str.chars().count() as i64;
    let conn = state.db.lock().map_err(|e| {
        (
            axum::http::StatusCode::INTERNAL_SERVER_ERROR,
            axum::Json(serde_json::json!({"error": format!("db lock poisoned: {e}")})),
        )
    })?;
    let _ = conn.execute(
        "INSERT OR IGNORE INTO tool_executions
         (id, agent_id, conversation_id, tool_name, arguments_json, output, output_chars, is_error, duration_ms, created_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)",
        rusqlite::params![
            id, agent_id,
            body["conversation_id"].as_str(),
            body["tool_name"].as_str().unwrap_or("unknown"),
            body["arguments_json"].as_str().unwrap_or("{}"),
            output_str,
            output_chars,
            body["is_error"].as_bool().unwrap_or(false) as i64,
            body["duration_ms"].as_i64().unwrap_or(0),
            now,
        ],
    );
    Ok(Json(json!({ "id": id, "output_chars": output_chars })))
}

fn unix_ts() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs() as i64
}

#[cfg(test)]
mod p8_output_chars_tests {
    use cade_store::sqlite as st;

    #[test]
    fn fresh_db_has_output_chars_column() {
        // apply_schema runs in `open` and must include output_chars on a brand-new DB.
        let db = st::open(":memory:").unwrap();
        let conn = db.lock().unwrap();
        let cols: Vec<String> = conn
            .prepare("PRAGMA table_info(tool_executions)")
            .unwrap()
            .query_map([], |r| r.get::<_, String>(1))
            .unwrap()
            .filter_map(Result::ok)
            .collect();
        assert!(cols.contains(&"output_chars".to_string()),
            "output_chars column missing on fresh DB; got: {cols:?}");
    }

    #[test]
    fn insert_persists_output_chars_matching_chars_count() {
        let db = st::open(":memory:").unwrap();
        let conn = db.lock().unwrap();
        // Seed the parent agent for the FK.
        conn.execute(
            "INSERT INTO agents (id, name, model, system_prompt, created_at) VALUES ('a1','A','test','',0)",
            [],
        ).unwrap();
        // Multibyte content: chars().count() = 5; LENGTH() = 11 bytes.
        let output = "héllo";
        let chars = output.chars().count() as i64;
        conn.execute(
            "INSERT INTO tool_executions
             (id, agent_id, tool_name, arguments_json, output, output_chars, is_error, duration_ms, created_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, 0, 0, 0)",
            rusqlite::params!["te1", "a1", "echo", "{}", output, chars],
        ).unwrap();
        let stored: i64 = conn
            .query_row(
                "SELECT output_chars FROM tool_executions WHERE id = 'te1'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(stored, 5);
        assert_eq!(stored, chars);
    }

    #[test]
    fn migration_backfills_output_chars_for_legacy_rows() {
        let db = st::open(":memory:").unwrap();
        let conn = db.lock().unwrap();
        // After open() the user_version must be >= 6 (the P8 migration).
        let v: i64 = conn
            .query_row("PRAGMA user_version", [], |r| r.get(0))
            .unwrap();
        assert!(v >= 6, "expected user_version >= 6 after migration, got {v}");
    }
}
