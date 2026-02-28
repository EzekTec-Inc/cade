use axum::{
    extract::{Path, State},
    http::StatusCode,
    Json,
};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use uuid::Uuid;

use crate::server::{
    state::AppState,
    storage::sqlite::{self, AgentRow},
};

const CADE_SYSTEM_PROMPT: &str = "You are CADE, a coding AI assistant with desktop extensions. \
You have access to tools for reading/writing files, running shell commands, searching code, \
and controlling the desktop. Work step-by-step, always verify your changes with the available tools. \
Be concise and accurate.";

// ── Request / Response DTOs ───────────────────────────────────────────────────

#[derive(Debug, Deserialize)]
pub struct CreateAgentBody {
    pub name: Option<String>,
    pub model: String,
    pub description: Option<String>,
    pub system_prompt: Option<String>,
    #[serde(default)]
    pub memory_blocks: Vec<Value>,
    #[serde(default)]
    pub tool_ids: Vec<String>,
}

#[derive(Debug, Serialize)]
pub struct AgentResponse {
    pub id: String,
    pub name: String,
    pub model: Option<String>,
    pub description: Option<String>,
}

impl From<AgentRow> for AgentResponse {
    fn from(r: AgentRow) -> Self {
        Self { id: r.id, name: r.name, model: Some(r.model), description: r.description }
    }
}

// ── Handlers ──────────────────────────────────────────────────────────────────

pub async fn create_agent(
    State(state): State<AppState>,
    Json(body): Json<CreateAgentBody>,
) -> Result<Json<Value>, (StatusCode, Json<Value>)> {
    let id = format!("agent-{}", Uuid::new_v4());
    let name = body.name.unwrap_or_else(|| format!("CADE-{}", &id[6..14]));
    let system_prompt = body.system_prompt
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| CADE_SYSTEM_PROMPT.to_string());

    let row = AgentRow {
        id: id.clone(),
        name: name.clone(),
        model: body.model.clone(),
        description: body.description.clone(),
        system_prompt: Some(system_prompt),
    };

    sqlite::create_agent(&state.db, &row).map_err(|e| server_err(e.to_string()))?;

    // Handle memory blocks
    for block in &body.memory_blocks {
        let label = block["label"].as_str().unwrap_or("memory");
        let value = block["value"].as_str().unwrap_or("");
        let _ = sqlite::upsert_memory_block(&state.db, &id, label, value);
    }

    tracing::info!("Created agent: {name} ({id}) model={}", body.model);
    Ok(Json(json!(AgentResponse::from(row))))
}

pub async fn get_agent(
    State(state): State<AppState>,
    Path(agent_id): Path<String>,
) -> Result<Json<Value>, (StatusCode, Json<Value>)> {
    match sqlite::get_agent(&state.db, &agent_id).map_err(|e| server_err(e.to_string()))? {
        Some(row) => Ok(Json(json!(AgentResponse::from(row)))),
        None => Err((StatusCode::NOT_FOUND, Json(json!({"detail": format!("Agent '{agent_id}' not found")})))),
    }
}

pub async fn list_agents(
    State(state): State<AppState>,
) -> Result<Json<Value>, (StatusCode, Json<Value>)> {
    let rows = sqlite::list_agents(&state.db).map_err(|e| server_err(e.to_string()))?;
    let agents: Vec<AgentResponse> = rows.into_iter().map(AgentResponse::from).collect();
    Ok(Json(json!(agents)))
}

pub async fn delete_agent(
    State(state): State<AppState>,
    Path(agent_id): Path<String>,
) -> Result<StatusCode, (StatusCode, Json<Value>)> {
    let deleted = sqlite::delete_agent(&state.db, &agent_id).map_err(|e| server_err(e.to_string()))?;
    if deleted { Ok(StatusCode::NO_CONTENT) } else {
        Err((StatusCode::NOT_FOUND, Json(json!({"detail": "Agent not found"}))))
    }
}

fn server_err(msg: String) -> (StatusCode, Json<Value>) {
    (StatusCode::INTERNAL_SERVER_ERROR, Json(json!({"detail": msg})))
}
