use axum::{Json, extract::State, http::StatusCode};
use serde_json::{Value, json};
use uuid::Uuid;

use crate::server::state::AppState;
use cade_store::sqlite::{self, ToolRow};

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
    let mut tools: Vec<Value> = rows
        .iter()
        .filter(|tool| {
            !tool.tags.iter().any(|tag| tag == "plugin") && !tool.id.starts_with("tool-plugin-")
        })
        .map(|t| {
            json!({
                "id": t.id,
                "name": t.name,
                "description": t.description
            })
        })
        .collect();

    // Dynamically append live capability definitions from CapabilityMesh seam (ADR-0020)
    use cade_core::capabilities::mesh::{CapabilityExecutionContext, CapabilityMesh};
    let cap_cx = CapabilityExecutionContext::new("api");
    let mesh_schemas = state.mcp.active_catalog(&cap_cx).await;
    for cap_s in mesh_schemas {
        let name = cap_s.schema["name"].as_str().unwrap_or("").to_string();
        if name.is_empty() || tools.iter().any(|t| t["name"].as_str() == Some(&name)) {
            continue;
        }
        let description = cap_s.schema["description"].as_str().map(String::from);
        tools.push(json!({
            "id": format!("tool-mesh-{}", name),
            "name": name,
            "description": description
        }));
    }

    // Use the same executable catalogue and collision policy as agent context/dispatch.
    let cwd = crate::server::api::run::runtime::execution_workspace();
    let plugin_catalog =
        crate::server::api::run::plugin_execution::ready_catalog(&cwd, &state.mcp).await;
    for pt in plugin_catalog.tools {
        let description = pt
            .schema
            .get("description")
            .and_then(|d| d.as_str())
            .map(String::from);
        let existing = tools
            .iter()
            .position(|tool| tool["name"].as_str() == Some(pt.name.as_str()));
        let id = existing
            .and_then(|index| tools[index].get("id").cloned())
            .unwrap_or_else(|| json!(format!("tool-plugin-{}", pt.name)));
        let entry = json!({
            "id": id,
            "name": pt.name,
            "description": description,
            "source": "plugin",
            "status": "ready",
            "execution_kind": "native_script",
            "json_schema": pt.schema
        });
        if let Some(index) = existing {
            tools[index] = entry;
        } else {
            tools.push(entry);
        }
    }

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
