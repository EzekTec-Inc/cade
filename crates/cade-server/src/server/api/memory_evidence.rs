/// Memory provenance and reflection API handlers.
use axum::{
    Json,
    extract::{Path, State},
    http::StatusCode,
};
use serde_json::{Value, json};

use crate::server::{reflection, state::AppState};
use cade_store::sqlite;

// region:    --- Evidence endpoints

/// GET /v1/agents/:id/memory/:label/evidence
pub async fn list_evidence(
    State(state): State<AppState>,
    Path((agent_id, label)): Path<(String, String)>,
) -> Result<Json<Value>, (StatusCode, Json<Value>)> {
    let rows = sqlite::list_memory_evidence(&state.db, &agent_id, &label).map_err(|e: cade_store::error::Error| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({ "detail": e.to_string() })),
        )
    })?;

    let items: Vec<Value> = rows
        .iter()
        .map(|(id, kind, reference, excerpt, confidence, created_at)| {
            json!({
                "id":         id,
                "kind":       kind,
                "reference":  reference,
                "excerpt":    excerpt,
                "confidence": confidence,
                "created_at": created_at,
            })
        })
        .collect();

    Ok(Json(json!(items)))
}

/// POST /v1/agents/:id/memory/:label/evidence
pub async fn add_evidence(
    State(state): State<AppState>,
    Path((agent_id, label)): Path<(String, String)>,
    Json(body): Json<Value>,
) -> Result<Json<Value>, (StatusCode, Json<Value>)> {
    let kind = body["kind"].as_str().ok_or((
        StatusCode::BAD_REQUEST,
        Json(json!({ "detail": "kind required" })),
    ))?;
    let reference = body["reference"].as_str().ok_or((
        StatusCode::BAD_REQUEST,
        Json(json!({ "detail": "reference required" })),
    ))?;
    let excerpt = body["excerpt"].as_str();
    let confidence = body["confidence"].as_f64().unwrap_or(1.0);

    let id = sqlite::insert_memory_evidence(
        &state.db, &agent_id, &label, kind, reference, excerpt, confidence,
    )
    .map_err(|e: cade_store::error::Error| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({ "detail": e.to_string() })),
        )
    })?;

    Ok(Json(json!({ "id": id })))
}

/// GET /v1/agents/:id/memory/:label/why
/// Returns a human-readable explanation of why a block exists.
pub async fn memory_why(
    State(state): State<AppState>,
    Path((agent_id, label)): Path<(String, String)>,
) -> Result<Json<Value>, (StatusCode, Json<Value>)> {
    let evidence = sqlite::list_memory_evidence(&state.db, &agent_id, &label).unwrap_or_default();

    let blocks = sqlite::get_memory_blocks_full(&state.db, &agent_id).unwrap_or_default();
    let block = blocks.iter().find(|(l, _, _, _)| l == &label);
    let value = block
        .map(|(_, v, _, _): &(String, String, String, String)| v.as_str())
        .unwrap_or("(not found)");
    let memory_type = block.map(|(_, _, _, _)| "").unwrap_or("generic");

    let summary = if evidence.is_empty() {
        format!(
            "Block '{label}' ({memory_type}) = \"{}\"\nNo explicit evidence linked. May have been set manually or by the agent during a session.",
            truncate(value, 200)
        )
    } else {
        let mut s = format!(
            "Block '{label}' = \"{}\"\n\nEvidence ({} source(s)):\n",
            truncate(value, 200),
            evidence.len()
        );
        for (_, kind, reference, excerpt, confidence, _) in &evidence {
            s.push_str(&format!(
                "  • [{kind}] {reference}  (confidence: {confidence:.0}%)\n",
                confidence = confidence * 100.0,
            ));
            if let Some(ex) = excerpt {
                s.push_str(&format!("    \"{}\"\n", truncate(ex, 120)));
            }
        }
        s
    };

    Ok(Json(json!({ "label": label, "summary": summary })))
}

// endregion: --- Evidence endpoints

// region:    --- Reflection endpoints

/// POST /v1/agents/:id/reflect
pub async fn trigger_reflect(
    State(state): State<AppState>,
    Path(agent_id): Path<String>,
    Json(body): Json<Value>,
) -> Result<Json<Value>, (StatusCode, Json<Value>)> {
    let focus = body["focus"].as_str().map(String::from);
    let trigger = body["trigger"].as_str().unwrap_or("manual");

    let result =
        reflection::reflect_agent(&state, &agent_id, None, focus.as_deref(), trigger).await;

    Ok(Json(json!({
        "blocks_created": result.blocks_created,
        "blocks_updated": result.blocks_updated,
        "summary":        result.summary,
        "duration_ms":    result.duration_ms,
    })))
}

/// GET /v1/agents/:id/reflection
pub async fn list_reflection(
    State(state): State<AppState>,
    Path(agent_id): Path<String>,
) -> Result<Json<Value>, (StatusCode, Json<Value>)> {
    let rows = sqlite::list_reflection_log(&state.db, &agent_id).map_err(|e: cade_store::error::Error| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({ "detail": e.to_string() })),
        )
    })?;
    Ok(Json(json!(rows)))
}

// endregion: --- Reflection endpoints

// region:    --- Support

fn truncate(s: &str, max: usize) -> &str {
    if s.len() <= max {
        return s;
    }
    &s[..s.ceil_char_boundary(max)]
}

// endregion: --- Support
