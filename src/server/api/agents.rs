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

/// Minimal fallback system prompt used only when the client doesn't supply one
/// (e.g. API calls outside the CLI). The CLI always sends BASE_SYSTEM_PROMPT.
const CADE_SYSTEM_PROMPT: &str = "\
You are CADE (Coding AI assistant with Desktop Extensions), a stateful AI coding agent. \
Use your tools to explore, edit, and run code on the user's machine. \
Be concise, accurate, and verify your changes after making them.";

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
        let desc = block["description"].as_str();
        let _ = sqlite::upsert_memory_block(&state.db, &id, label, value, desc);
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

#[derive(Debug, Deserialize)]
pub struct PatchAgentBody {
    pub name: Option<String>,
    pub model: Option<String>,
    pub system_prompt: Option<String>,
}

/// PATCH /v1/agents/:id — update model and/or system_prompt
pub async fn patch_agent(
    State(state): State<AppState>,
    Path(agent_id): Path<String>,
    Json(body): Json<PatchAgentBody>,
) -> Result<Json<Value>, (StatusCode, Json<Value>)> {
    // Verify agent exists
    let existing = sqlite::get_agent(&state.db, &agent_id)
        .map_err(|e| server_err(e.to_string()))?
        .ok_or_else(|| (StatusCode::NOT_FOUND, Json(json!({"detail": "Agent not found"}))))?;

    let mut updated_model = existing.model.clone();
    let mut updated_name  = existing.name.clone();

    if let Some(name) = &body.name {
        let name = name.trim();
        if name.is_empty() {
            return Err((StatusCode::BAD_REQUEST, Json(json!({ "detail": "name cannot be empty" }))));
        }
        sqlite::update_agent_name(&state.db, &agent_id, name)
            .map_err(|e| server_err(e.to_string()))?;
        updated_name = name.to_string();
        tracing::info!("Agent {agent_id}: name → {name}");
    }

    if let Some(model) = &body.model {
        // Validate the model is routable before persisting
        state.llm_router.read().await.validate_model(model).map_err(|e| {
            (StatusCode::BAD_REQUEST, Json(json!({ "detail": e.to_string() })))
        })?;
        sqlite::update_agent_model(&state.db, &agent_id, model)
            .map_err(|e| server_err(e.to_string()))?;
        updated_model = model.clone();
        tracing::info!("Agent {agent_id}: model → {model}");
    }

    Ok(Json(json!({
        "id": agent_id,
        "name": updated_name,
        "model": updated_model
    })))
}

// ── Memory endpoints ──────────────────────────────────────────────────────────

/// GET /v1/agents/:id/memory
pub async fn get_memory(
    State(state): State<AppState>,
    Path(agent_id): Path<String>,
) -> Result<Json<Value>, (StatusCode, Json<Value>)> {
    let blocks = sqlite::get_memory_blocks(&state.db, &agent_id)
        .map_err(|e| server_err(e.to_string()))?;
    let arr: Vec<Value> = blocks.into_iter()
        .map(|(label, value, description)| json!({ "label": label, "value": value, "description": description }))
        .collect();
    Ok(Json(json!({ "blocks": arr })))
}

/// DELETE /v1/agents/:id/memory/:label
pub async fn delete_memory(
    State(state): State<AppState>,
    Path((agent_id, label)): Path<(String, String)>,
) -> Result<StatusCode, (StatusCode, Json<Value>)> {
    let found = sqlite::delete_memory_block(&state.db, &agent_id, &label)
        .map_err(|e| server_err(e.to_string()))?;
    if found { Ok(StatusCode::NO_CONTENT) }
    else { Err((StatusCode::NOT_FOUND, Json(json!({"detail": format!("Memory block '{label}' not found")})))) }
}

/// PUT /v1/agents/:id/memory/:label
pub async fn upsert_memory(
    State(state): State<AppState>,
    Path((agent_id, label)): Path<(String, String)>,
    Json(body): Json<Value>,
) -> Result<StatusCode, (StatusCode, Json<Value>)> {
    let value       = body["value"].as_str().unwrap_or("").to_string();
    let description = body["description"].as_str();
    sqlite::upsert_memory_block(&state.db, &agent_id, &label, &value, description)
        .map_err(|e| server_err(e.to_string()))?;
    Ok(StatusCode::NO_CONTENT)
}

// ── Messages endpoints ────────────────────────────────────────────────────────

/// DELETE /v1/agents/:id/messages — clear context (default conversation)
pub async fn clear_messages_handler(
    State(state): State<AppState>,
    Path(agent_id): Path<String>,
    axum::extract::Query(params): axum::extract::Query<std::collections::HashMap<String, String>>,
) -> Result<Json<Value>, (StatusCode, Json<Value>)> {
    let conv_id = params.get("conversation_id").map(String::as_str);
    let n = sqlite::clear_messages(&state.db, &agent_id, conv_id)
        .map_err(|e| server_err(e.to_string()))?;
    Ok(Json(json!({ "deleted": n })))
}

/// GET /v1/agents/:id/messages?q=<query>&conversation_id=<id> — search message history
pub async fn search_messages_handler(
    State(state): State<AppState>,
    Path(agent_id): Path<String>,
    axum::extract::Query(params): axum::extract::Query<std::collections::HashMap<String, String>>,
) -> Result<Json<Value>, (StatusCode, Json<Value>)> {
    let query = params.get("q").map(String::as_str).unwrap_or("");
    if query.is_empty() {
        return Err((StatusCode::BAD_REQUEST, Json(json!({"detail": "missing ?q= parameter"}))));
    }
    let conv_id = params.get("conversation_id").map(String::as_str);
    let rows = sqlite::search_messages(&state.db, &agent_id, query, conv_id)
        .map_err(|e| server_err(e.to_string()))?;
    let messages: Vec<Value> = rows.into_iter().map(|r| json!({
        "id": r.id,
        "role": r.role,
        "content": r.content,
        "conversation_id": r.conversation_id,
    })).collect();
    Ok(Json(json!({ "messages": messages })))
}

// ── Conversation endpoints ────────────────────────────────────────────────────

use crate::server::storage::sqlite::ConversationRow;

fn conv_to_json(c: &ConversationRow) -> Value {
    json!({
        "id":            c.id,
        "agent_id":      c.agent_id,
        "title":         c.title,
        "created_at":    c.created_at,
        "updated_at":    c.updated_at,
        "message_count": c.message_count,
    })
}

/// GET /v1/agents/:id/conversations
pub async fn list_conversations(
    State(state): State<AppState>,
    Path(agent_id): Path<String>,
) -> Result<Json<Value>, (StatusCode, Json<Value>)> {
    let rows = sqlite::list_conversations(&state.db, &agent_id)
        .map_err(|e| server_err(e.to_string()))?;
    let convs: Vec<Value> = rows.iter().map(conv_to_json).collect();
    Ok(Json(json!({ "conversations": convs })))
}

/// POST /v1/agents/:id/conversations — create a new conversation
pub async fn create_conversation(
    State(state): State<AppState>,
    Path(agent_id): Path<String>,
    Json(body): Json<Value>,
) -> Result<Json<Value>, (StatusCode, Json<Value>)> {
    let title = body["title"].as_str().unwrap_or("").to_string();
    let row = sqlite::create_conversation(&state.db, &agent_id, &title)
        .map_err(|e| server_err(e.to_string()))?;
    Ok(Json(conv_to_json(&row)))
}

/// DELETE /v1/agents/:id/conversations/:conv_id
pub async fn delete_conversation(
    State(state): State<AppState>,
    Path((agent_id, conv_id)): Path<(String, String)>,
) -> Result<Json<Value>, (StatusCode, Json<Value>)> {
    // Verify ownership
    match sqlite::get_conversation(&state.db, &conv_id) {
        Ok(Some(c)) if c.agent_id != agent_id => {
            return Err((StatusCode::FORBIDDEN, Json(json!({"detail": "conversation not owned by this agent"}))));
        }
        Ok(None) => {
            return Err((StatusCode::NOT_FOUND, Json(json!({"detail": "conversation not found"}))));
        }
        Err(e) => return Err(server_err(e.to_string())),
        Ok(Some(_)) => {}
    }
    let deleted = sqlite::delete_conversation(&state.db, &conv_id)
        .map_err(|e| server_err(e.to_string()))?;
    Ok(Json(json!({ "deleted": deleted })))
}

/// POST /v1/agents/:id/tools — attach tool IDs to an agent
pub async fn attach_tools(
    State(state): State<AppState>,
    Path(agent_id): Path<String>,
    Json(body): Json<serde_json::Value>,
) -> Result<StatusCode, (StatusCode, Json<Value>)> {
    let ids: Vec<String> = body["tool_ids"]
        .as_array()
        .unwrap_or(&vec![])
        .iter()
        .filter_map(|v| v.as_str().map(String::from))
        .collect();
    sqlite::attach_tools_to_agent(&state.db, &agent_id, &ids)
        .map_err(|e| server_err(e.to_string()))?;
    Ok(StatusCode::NO_CONTENT)
}

fn server_err(msg: String) -> (StatusCode, Json<Value>) {
    tracing::error!("500 Internal Server Error: {msg}");
    (StatusCode::INTERNAL_SERVER_ERROR, Json(json!({"detail": msg})))
}
