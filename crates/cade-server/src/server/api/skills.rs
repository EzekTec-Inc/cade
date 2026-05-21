//! Skill management API endpoints.
//!
//! - `GET  /v1/skills`                     — list all discovered skills
//! - `GET  /v1/agents/:id/skills`          — list skills loaded for an agent
//! - `POST /v1/agents/:id/skills/load`     — load (activate) a skill for an agent

use axum::{
    Json,
    extract::{Path, State},
    http::StatusCode,
    response::{IntoResponse, Response},
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
        None => {
            return err(
                StatusCode::NOT_FOUND,
                &format!("Skill '{skill_id}' not found"),
            );
        }
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
    {
        let mut cache = state.context_cache.lock();
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

/// `POST /v1/agents/:id/skills/unload` — unload (deactivate) a skill for an agent.
///
/// Request body: `{ "id": "skill-id" }`
///
/// Removes the skill from the agent's active set. The skill body will no longer
/// be injected into the system prompt on the next `build_context` call.
/// Does **not** invalidate the context cache — the stale cache entry naturally
/// expires when message history changes on the next turn.
pub async fn unload_skill(
    State(state): State<AppState>,
    Path(agent_id): Path<String>,
    Json(body): Json<serde_json::Value>,
) -> Response {
    let skill_id = match body["id"].as_str() {
        Some(id) => id.to_string(),
        None => return err(StatusCode::BAD_REQUEST, "Missing 'id' field"),
    };

    let removed = {
        let mut agent_skills = state.agent_skills.write().await;
        if let Some(loaded) = agent_skills.get_mut(&agent_id) {
            let before = loaded.len();
            loaded.retain(|id| id != &skill_id);
            before != loaded.len()
        } else {
            false
        }
    };

    if removed {
        Json(json!({
            "id": skill_id,
            "status": "unloaded",
            "message": format!("Skill '{}' removed from active context. Takes effect on next turn.", skill_id),
        }))
        .into_response()
    } else {
        err(
            StatusCode::NOT_FOUND,
            &format!("Skill '{skill_id}' is not loaded for agent '{agent_id}'"),
        )
    }
}

/// `POST /v1/agents/:id/skills/disable` — add a skill to the agent's blacklist.
///
/// Request body: `{ "id": "skill-id" }`
///
/// The skill continues to be *discovered* but will be filtered out of
/// `build_context` / the system prompt for this agent.
pub async fn disable_skill(
    State(state): State<AppState>,
    Path(agent_id): Path<String>,
    Json(body): Json<serde_json::Value>,
) -> Response {
    let skill_id = match body["id"].as_str() {
        Some(id) => id.to_string(),
        None => return err(StatusCode::BAD_REQUEST, "Missing 'id' field"),
    };
    match cade_store::sqlite::skills::disable_skill(&state.db, &agent_id, &skill_id) {
        Ok(()) => Json(json!({
            "id": skill_id,
            "status": "disabled",
            "message": format!("Skill '{}' disabled for agent '{}'. Takes effect on next turn.", skill_id, agent_id),
        }))
        .into_response(),
        Err(e) => err(StatusCode::INTERNAL_SERVER_ERROR, &e.to_string()),
    }
}

/// `POST /v1/agents/:id/skills/enable` — remove a skill from the agent's blacklist.
///
/// Request body: `{ "id": "skill-id" }`
pub async fn enable_skill(
    State(state): State<AppState>,
    Path(agent_id): Path<String>,
    Json(body): Json<serde_json::Value>,
) -> Response {
    let skill_id = match body["id"].as_str() {
        Some(id) => id.to_string(),
        None => return err(StatusCode::BAD_REQUEST, "Missing 'id' field"),
    };
    match cade_store::sqlite::skills::enable_skill(&state.db, &agent_id, &skill_id) {
        Ok(()) => Json(json!({
            "id": skill_id,
            "status": "enabled",
            "message": format!("Skill '{}' re-enabled for agent '{}'.", skill_id, agent_id),
        }))
        .into_response(),
        Err(e) => err(StatusCode::INTERNAL_SERVER_ERROR, &e.to_string()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::{
        body::Body,
        http::{self, Request},
    };
    use std::sync::Arc;
    use tokio::sync::RwLock;
    use tower::ServiceExt;

    fn test_state() -> AppState {
        // Use cade_store::sqlite::open so apply_schema runs (creates all tables).
        let db = cade_store::sqlite::open(":memory:").unwrap();
        let config = Arc::new(crate::server::config::ServerConfig {
            max_tokens_per_turn: Some(64_000),
            addr: "127.0.0.1:0".parse().unwrap(),
            db_path: ":memory:".into(),
            llm_provider: crate::server::config::LlmProviderKind::Anthropic,
            default_model: "test".into(),
            anthropic_api_key: None,
            openai_api_key: None,
            google_api_key: None,
            ollama_base_url: String::new(),
            api_key: Some("testkey".into()),
            allowed_origin: None,
            max_context_budget: None,
        });
        AppState {
            subagent_cancellations: std::sync::Arc::new(tokio::sync::RwLock::new(
                std::collections::HashMap::new(),
            )),
            db,
            llm: Arc::new(cade_ai::LlmRouter::build(&cade_ai::AiConfig {
                anthropic_api_key: None,
                openai_api_key: None,
                google_api_key: None,
                ollama_base_url: String::new(),
                llm_provider: String::new(),
            })),
            llm_router: Arc::new(RwLock::new(cade_ai::LlmRouter::build(&cade_ai::AiConfig {
                anthropic_api_key: None,
                openai_api_key: None,
                google_api_key: None,
                ollama_base_url: String::new(),
                llm_provider: String::new(),
            }))),
            config,
            mcp: Arc::new(crate::server::state::McpManager::empty()),
            rate_limiter: crate::server::rate_limit::RateLimiter::from_env(),
            memory_cache: Arc::new(parking_lot::Mutex::new(std::collections::HashMap::new())),
            agent_activity: Arc::new(RwLock::new(std::collections::HashMap::new())),
            agent_metrics: Arc::new(dashmap::DashMap::new()),
            agent_context_telemetry: Arc::new(RwLock::new(std::collections::HashMap::new())),
            context_cache: Arc::new(parking_lot::Mutex::new(lru::LruCache::new(
                crate::server::state::CONTEXT_CACHE_CAPACITY,
            ))),
            all_skills: Arc::new(RwLock::new(Vec::new())),
            agent_skills: Arc::new(RwLock::new(std::collections::HashMap::new())),
            pending_subagent_results: Arc::new(RwLock::new(std::collections::HashMap::new())),
            subagent_semaphore: Arc::new(tokio::sync::Semaphore::new(4)),
            embedder: None,
        }
    }

    async fn post_json(
        state: AppState,
        uri: &str,
        body: serde_json::Value,
    ) -> axum::response::Response {
        let router = crate::server::api::router(state);
        router
            .oneshot(
                Request::builder()
                    .method(http::Method::POST)
                    .uri(uri)
                    .header("content-type", "application/json")
                    .header("Authorization", "Bearer testkey")
                    .body(Body::from(serde_json::to_string(&body).unwrap()))
                    .unwrap(),
            )
            .await
            .unwrap()
    }

    fn mk_agent(id: &str) -> cade_store::sqlite::AgentRow {
        cade_store::sqlite::AgentRow {
            id: id.into(),
            name: id.into(),
            model: "t".into(),
            description: None,
            system_prompt: None,
            created_at: None,
            compaction_model: None,
            theme: None,
            active_plan_json: None,
        }
    }

    /// Phase B3: POST /v1/agents/:id/skills/disable must return 200 and
    /// persist the blacklist row.
    #[tokio::test]
    async fn disable_skill_returns_200_and_persists() {
        let state = test_state();
        cade_store::sqlite::create_agent(&state.db, &mk_agent("ag1")).unwrap();

        let resp = post_json(
            state.clone(),
            "/v1/agents/ag1/skills/disable",
            serde_json::json!({"id": "rust"}),
        )
        .await;

        assert_eq!(resp.status(), 200, "expect 200 OK");
        assert!(
            cade_store::sqlite::skills::is_skill_disabled(&state.db, "ag1", "rust").unwrap(),
            "skill must be in blacklist after disable"
        );
    }

    /// Phase B3: POST /v1/agents/:id/skills/enable must return 200 and
    /// remove the blacklist row.
    #[tokio::test]
    async fn enable_skill_returns_200_and_removes_blacklist() {
        let state = test_state();
        cade_store::sqlite::create_agent(&state.db, &mk_agent("ag2")).unwrap();
        cade_store::sqlite::skills::disable_skill(&state.db, "ag2", "rust").unwrap();

        let resp = post_json(
            state.clone(),
            "/v1/agents/ag2/skills/enable",
            serde_json::json!({"id": "rust"}),
        )
        .await;

        assert_eq!(resp.status(), 200, "expect 200 OK");
        assert!(
            !cade_store::sqlite::skills::is_skill_disabled(&state.db, "ag2", "rust").unwrap(),
            "skill must be removed from blacklist after enable"
        );
    }

    /// Phase B3: missing 'id' field must return 400.
    #[tokio::test]
    async fn disable_skill_missing_id_returns_400() {
        let state = test_state();
        let resp = post_json(
            state,
            "/v1/agents/any/skills/disable",
            serde_json::json!({}),
        )
        .await;
        assert_eq!(resp.status(), 400);
    }
}
