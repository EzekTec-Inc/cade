use axum::{Json, extract::State, http::StatusCode};
use serde_json::{Value, json};
use uuid::Uuid;

use crate::server::{
    state::AppState,
    storage::sqlite::{self, ToolRow},
};

pub async fn create_tool(
    State(state): State<AppState>,
    Json(body): Json<Value>,
) -> Result<Json<Value>, (StatusCode, Json<Value>)> {
    let source_code = body["source_code"].as_str().unwrap_or("").to_string();
    let _source_type = body["source_type"].as_str().unwrap_or("python").to_string();
    let json_schema = body.get("json_schema").cloned();
    let tags: Vec<String> = body["tags"]
        .as_array()
        .map(|arr| {
            arr.iter()
                .filter_map(|v| v.as_str())
                .map(String::from)
                .collect()
        })
        .unwrap_or_default();

    // Extract tool name from json_schema["name"] or source_code first line
    let name = json_schema
        .as_ref()
        .and_then(|s| s["name"].as_str())
        .map(String::from)
        .or_else(|| extract_fn_name(&source_code))
        .unwrap_or_else(|| format!("tool-{}", &Uuid::new_v4().to_string()[..8]));

    let description = json_schema
        .as_ref()
        .and_then(|s| s["description"].as_str())
        .map(String::from);

    let id = format!("tool-{}", Uuid::new_v4());
    let row = ToolRow {
        id: id.clone(),
        name: name.clone(),
        description: description.clone(),
        source_code: Some(source_code),
        json_schema,
        tags,
    };

    sqlite::upsert_tool(&state.db, &row).map_err(|e| {
        tracing::error!("500 upsert_tool [{name}]: {e}");
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({"detail": e.to_string()})),
        )
    })?;

    // The upsert may have kept an existing row's id (ON CONFLICT preserves
    // the original PK).  Read back the actual id so callers can attach tools
    // to agents without FK violations.
    let actual_id = sqlite::get_tool_id_by_name(&state.db, &name).unwrap_or(id.clone());

    tracing::debug!("Registered tool: {name} ({actual_id})");
    Ok(Json(
        json!({ "id": actual_id, "name": name, "description": description }),
    ))
}

pub async fn list_tools(
    State(state): State<AppState>,
) -> Result<Json<Value>, (StatusCode, Json<Value>)> {
    let rows = sqlite::list_tools(&state.db).map_err(|e| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({"detail": e.to_string()})),
        )
    })?;
    let tools: Vec<Value> = rows
        .iter()
        .map(|t| {
            json!({
                "id": t.id,
                "name": t.name,
                "description": t.description
            })
        })
        .collect();
    Ok(Json(json!(tools)))
}

/// Extract `def <name>(` from Python source code
fn extract_fn_name(source: &str) -> Option<String> {
    source
        .lines()
        .find(|l| l.trim_start().starts_with("def "))
        .and_then(|l| {
            let after_def = l.trim_start().strip_prefix("def ")?;
            let name = after_def.split('(').next()?.trim();
            if name.is_empty() {
                None
            } else {
                Some(name.to_string())
            }
        })
}
