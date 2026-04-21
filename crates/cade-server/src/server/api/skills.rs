//! Skill management API endpoints.
//!
//! - `GET  /v1/skills`                     — list all discovered skills
//! - `GET  /v1/agents/:id/skills`          — list skills loaded for an agent
//! - `POST /v1/agents/:id/skills/load`     — load (activate) a skill for an agent

use axum::{
    extract::{Path, State},
    http::StatusCode,
    response::{IntoResponse, Response},
    Json,
};
use serde_json::json;

use crate::server::state::AppState;

fn err(status: StatusCode, msg: &str) -> Response {
    (status, Json(json!({ "error": msg }))).into_response()
}

/// `GET /v1/skills` — list all discovered skills (global + project).
pub async fn list_all_skills(State(state): State<AppState>) -> Response {
    let skills = state.all_skills.read().await;
    let listing: Vec<serde_json::Value> = skills
        .iter()
        .map(|s| {
            json!({
                "id": s.id,
                "name": s.name,
                "description": s.description,
                "scope": format!("{:?}", s.scope),
                "tags": s.tags,
                "triggers": s.triggers,
                "body_chars": s.body.chars().count(),
                "scripts": s.scripts.iter().map(|sc| &sc.name).collect::<Vec<_>>(),
                "references": s.references.iter().map(|r| &r.name).collect::<Vec<_>>(),
            })
        })
        .collect();
    Json(json!({ "skills": listing, "count": listing.len() })).into_response()
}

/// `GET /v1/agents/:id/skills` — list skills loaded (activated) for this agent.
pub async fn list_agent_skills(
    State(state): State<AppState>,
    Path(agent_id): Path<String>,
) -> Response {
    let agent_skills = state.agent_skills.read().await;
    let loaded_ids = agent_skills.get(&agent_id).cloned().unwrap_or_default();

    let all = state.all_skills.read().await;
    let loaded: Vec<serde_json::Value> = loaded_ids
        .iter()
        .filter_map(|id| all.iter().find(|s| s.id == *id))
        .map(|s| {
            json!({
                "id": s.id,
                "name": s.name,
                "description": s.description,
                "body_chars": s.body.chars().count(),
                "body_tokens_approx": s.body.chars().count() / 3,
            })
        })
        .collect();

    Json(json!({
        "agent_id": agent_id,
        "loaded": loaded,
        "count": loaded.len(),
    }))
    .into_response()
}

/// `POST /v1/agents/:id/skills/load` — load a skill by ID for an agent.
///
/// Request body: `{ "id": "skill-id" }`
///
/// Returns the full skill body so the caller can inject it into context.
pub async fn load_skill(
    State(state): State<AppState>,
    Path(agent_id): Path<String>,
    Json(body): Json<serde_json::Value>,
) -> Response {
    let skill_id = match body["id"].as_str() {
        Some(id) => id.to_string(),
        None => return err(StatusCode::BAD_REQUEST, "Missing 'id' field"),
    };

    // Find the skill in all discovered skills
    let all = state.all_skills.read().await;
    let skill = match all.iter().find(|s| s.id == skill_id) {
        Some(s) => s.clone(),
        None => return err(StatusCode::NOT_FOUND, &format!("Skill '{skill_id}' not found")),
    };

    // Add to agent's loaded skills (deduplicate)
    {
        let mut agent_skills = state.agent_skills.write().await;
        let loaded = agent_skills.entry(agent_id.clone()).or_default();
        if !loaded.contains(&skill_id) {
            loaded.push(skill_id.clone());
        }
    }

    // Invalidate context cache for this agent so next build_context picks up the skill
    if let Ok(mut cache) = state.context_cache.lock() {
        let keys_to_remove: Vec<String> = cache
            .iter()
            .filter(|(k, _)| k.starts_with(&format!("{agent_id}:")))
            .map(|(k, _)| k.clone())
            .collect();
        for k in keys_to_remove {
            cache.pop(&k);
        }
    }

    Json(json!({
        "id": skill.id,
        "name": skill.name,
        "description": skill.description,
        "body": skill.body,
        "body_chars": skill.body.chars().count(),
    }))
    .into_response()
}
