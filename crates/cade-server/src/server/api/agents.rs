use axum::{
    Json,
    extract::{Path, State},
    http::StatusCode,
};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use uuid::Uuid;

use crate::server::state::AppState;
use cade_store::sqlite::{self, AgentRow};

/// Minimal fallback system prompt used only when the client doesn't supply one
/// (e.g. API calls outside the CLI). The CLI always sends BASE_SYSTEM_PROMPT.
const CADE_SYSTEM_PROMPT: &str = "\
You are CADE (Coding AI assistant with Desktop Extensions), a stateful AI coding agent. \
Use your tools to explore, edit, and run code on the user's machine. \
Be concise, accurate, and verify your changes before and after making them. \
\n\nIMPORTANT: Never start a response with \"I am CADE\", your name, \
or any identity preamble. Answer questions directly and concisely. \
If explicitly asked who you are, answer in one brief sentence only.\
\n\nCRITICAL: User instructions regarding behavioral rules or execution modes \
(e.g., 'STRICT PROJECT EXECUTION MODE') are instructions for YOU, the LLM, to follow natively. \
Do NOT attempt to use MCP configuration tools (like set_config_value) to enforce them on the environment.\
Do not include filler phrases like \"Understood\" or \"I will adhere to the rules\". Just do the work.\
\n\nAfter every tool execution, always provide a plain-text response that explains \
the result, what you found, or what you are doing next. Never end a turn with only \
tool calls and no explanation.";

// -- Request / Response DTOs

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
    pub system_prompt: Option<String>,
    /// ISO-8601 creation timestamp (e.g. "2026-03-06T14:22:01Z").
    #[serde(skip_serializing_if = "Option::is_none")]
    pub created_at: Option<String>,
}

impl From<AgentRow> for AgentResponse {
    fn from(r: AgentRow) -> Self {
        let created_at = r.created_at.map(|ts| {
            use chrono::{DateTime, Utc};
            DateTime::<Utc>::from_timestamp(ts, 0)
                .map(|dt| dt.format("%Y-%m-%dT%H:%M:%SZ").to_string())
                .unwrap_or_else(|| ts.to_string())
        });
        Self {
            id: r.id,
            name: r.name,
            model: Some(r.model),
            description: r.description,
            system_prompt: r.system_prompt,
            created_at,
        }
    }
}

// -- Handlers

pub async fn create_agent(
    State(state): State<AppState>,
    Json(body): Json<CreateAgentBody>,
) -> Result<Json<Value>, (StatusCode, Json<Value>)> {
    let id = format!("agent-{}", Uuid::new_v4());
    let name = body.name.unwrap_or_else(|| format!("CADE-{}", &id[6..14]));
    let system_prompt = body
        .system_prompt
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| CADE_SYSTEM_PROMPT.to_string());

    let row = AgentRow {
        id: id.clone(),
        name: name.clone(),
        model: body.model.clone(),
        description: body.description.clone(),
        system_prompt: Some(system_prompt),
        created_at: None, // populated by DB via now_ts()
        compaction_model: None,
    };

    sqlite::create_agent(&state.db, &row).map_err(|e| server_err(e.to_string()))?;

    // Tool wiring: attach only the tools explicitly provided by the client.
    //
    // The CLI calls register_and_attach_filtered() immediately after this
    // endpoint and passes exactly the right toolset-specific IDs.
    //
    // The old "fall back to all registered tools" behaviour caused every new
    // agent to inherit every tool ever registered on the server — including
    // stale MCP tools from past sessions — inflating the tool-schema section
    // of every prompt with schemas for tools that may not even be running.
    if !body.tool_ids.is_empty() {
        let _ = sqlite::attach_tools_to_agent(&state.db, &id, &body.tool_ids);
        tracing::info!("Wired {} tool(s) to new agent {id}", body.tool_ids.len());
    }

    // Handle memory blocks
    for block in &body.memory_blocks {
        let label = block["label"].as_str().unwrap_or("memory");
        let value = block["value"].as_str().unwrap_or("");
        let desc = block["description"].as_str();
        let max_chars = block["max_chars"].as_u64().map(|n| n as usize);
        let _ = sqlite::upsert_memory_block(&state.db, &id, label, value, desc, max_chars);
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
        None => Err((
            StatusCode::NOT_FOUND,
            Json(json!({"detail": format!("Agent '{agent_id}' not found")})),
        )),
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
    let deleted =
        sqlite::delete_agent(&state.db, &agent_id).map_err(|e| server_err(e.to_string()))?;
    if deleted {
        Ok(StatusCode::NO_CONTENT)
    } else {
        Err((
            StatusCode::NOT_FOUND,
            Json(json!({"detail": "Agent not found"})),
        ))
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
        .ok_or_else(|| {
            (
                StatusCode::NOT_FOUND,
                Json(json!({"detail": "Agent not found"})),
            )
        })?;

    let mut updated_model = existing.model.clone();
    let mut updated_name = existing.name.clone();

    if let Some(name) = &body.name {
        let name = name.trim();
        if name.is_empty() {
            return Err((
                StatusCode::BAD_REQUEST,
                Json(json!({ "detail": "name cannot be empty" })),
            ));
        }
        sqlite::update_agent_name(&state.db, &agent_id, name)
            .map_err(|e| server_err(e.to_string()))?;
        updated_name = name.to_string();
        tracing::info!("Agent {agent_id}: name → {name}");
    }

    if let Some(model) = &body.model {
        // Validate the model is routable before persisting
        state
            .llm_router
            .read()
            .await
            .validate_model(model)
            .map_err(|e| {
                (
                    StatusCode::BAD_REQUEST,
                    Json(json!({ "detail": e.to_string() })),
                )
            })?;
        let updated = sqlite::update_agent_model(&state.db, &agent_id, model)
            .map_err(|e| server_err(e.to_string()))?;
        if !updated {
            return Err((
                StatusCode::NOT_FOUND,
                Json(json!({"detail": format!("Agent '{agent_id}' not found")})),
            ));
        }
        updated_model = model.clone();
        tracing::info!("Agent {agent_id}: model → {model}");
    }

    if let Some(prompt) = &body.system_prompt {
        let prompt = prompt.trim();
        sqlite::update_agent_system_prompt(&state.db, &agent_id, prompt)
            .map_err(|e| server_err(e.to_string()))?;
        tracing::info!(
            "Agent {agent_id}: system_prompt updated ({} chars)",
            prompt.len()
        );
    }

    Ok(Json(json!({
        "id": agent_id,
        "name": updated_name,
        "model": updated_model
    })))
}

// -- Memory endpoints

/// GET /v1/agents/:id/memory
pub async fn get_memory(
    State(state): State<AppState>,
    Path(agent_id): Path<String>,
) -> Result<Json<Value>, (StatusCode, Json<Value>)> {
    let blocks = sqlite::get_memory_blocks_full(&state.db, &agent_id)
        .map_err(|e| server_err(e.to_string()))?;
    let arr: Vec<Value> = blocks
        .into_iter()
        .map(|(label, value, description, tier)| {
            json!({
                "label": label, "value": value, "description": description, "tier": tier
            })
        })
        .collect();
    Ok(Json(json!({ "blocks": arr })))
}

/// PUT /v1/agents/:id/memory/:label/tier — set tier ('short'|'long'|'pinned')
pub async fn set_memory_tier_handler(
    State(state): State<AppState>,
    Path((agent_id, label)): Path<(String, String)>,
    Json(body): Json<Value>,
) -> Result<StatusCode, (StatusCode, Json<Value>)> {
    let tier = body["tier"].as_str().unwrap_or("short");
    if !matches!(tier, "short" | "long" | "pinned") {
        return Err((
            StatusCode::BAD_REQUEST,
            Json(json!({ "detail": "tier must be 'short', 'long', or 'pinned'" })),
        ));
    }
    let reset_turn = tier != "long"; // reactivation resets last_turn; demotion does not
    let found = sqlite::set_memory_tier(&state.db, &agent_id, &label, tier, reset_turn)
        .map_err(|e| server_err(e.to_string()))?;
    if found {
        Ok(StatusCode::NO_CONTENT)
    } else {
        Err((
            StatusCode::NOT_FOUND,
            Json(json!({ "detail": format!("Block '{label}' not found") })),
        ))
    }
}

/// DELETE /v1/agents/:id/memory/:label
pub async fn delete_memory(
    State(state): State<AppState>,
    Path((agent_id, label)): Path<(String, String)>,
) -> Result<StatusCode, (StatusCode, Json<Value>)> {
    let found = sqlite::delete_memory_block(&state.db, &agent_id, &label)
        .map_err(|e| server_err(e.to_string()))?;
    if found {
        Ok(StatusCode::NO_CONTENT)
    } else {
        Err((
            StatusCode::NOT_FOUND,
            Json(json!({"detail": format!("Memory block '{label}' not found")})),
        ))
    }
}

/// PUT /v1/agents/:id/memory/:label
pub async fn upsert_memory(
    State(state): State<AppState>,
    Path((agent_id, label)): Path<(String, String)>,
    Json(body): Json<Value>,
) -> Result<StatusCode, (StatusCode, Json<Value>)> {
    let value = body["value"].as_str().unwrap_or("").to_string();
    let description = body["description"].as_str();
    let max_chars = body["max_chars"].as_u64().map(|n| n as usize);
    sqlite::upsert_memory_block(&state.db, &agent_id, &label, &value, description, max_chars)
        .map_err(|e| server_err(e.to_string()))?;
    Ok(StatusCode::NO_CONTENT)
}

/// GET /v1/agents/:id/memory/:label/history?limit=5
pub async fn get_memory_history(
    State(state): State<AppState>,
    Path((agent_id, label)): Path<(String, String)>,
    axum::extract::Query(params): axum::extract::Query<std::collections::HashMap<String, String>>,
) -> Result<Json<Value>, (StatusCode, Json<Value>)> {
    let limit = params
        .get("limit")
        .and_then(|v| v.parse::<usize>().ok())
        .unwrap_or(5)
        .min(20);
    let history = sqlite::get_memory_history(&state.db, &agent_id, &label, limit)
        .map_err(|e| server_err(e.to_string()))?;
    let items: Vec<Value> = history
        .iter()
        .map(|(id, val, ts)| {
            json!({
                "id": id, "value": val, "updated_at": ts
            })
        })
        .collect();
    Ok(Json(json!(items)))
}

/// PUT /v1/agents/:id/memory/:label/restore/:rev_id
pub async fn restore_memory_revision(
    State(state): State<AppState>,
    Path((agent_id, label, rev_id)): Path<(String, String, String)>,
) -> Result<StatusCode, (StatusCode, Json<Value>)> {
    let found = sqlite::restore_memory_from_history(&state.db, &agent_id, &label, &rev_id)
        .map_err(|e| server_err(e.to_string()))?;
    if found {
        Ok(StatusCode::NO_CONTENT)
    } else {
        Err((
            StatusCode::NOT_FOUND,
            Json(json!({"detail": "revision not found"})),
        ))
    }
}

// -- Messages endpoints

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

/// GET /v1/agents/:id/messages?q=<query>&conversation_id=<id>&limit=<limit> — search or list message history
pub async fn search_messages_handler(
    State(state): State<AppState>,
    Path(agent_id): Path<String>,
    axum::extract::Query(params): axum::extract::Query<std::collections::HashMap<String, String>>,
) -> Result<Json<Value>, (StatusCode, Json<Value>)> {
    let query = params.get("q").map(String::as_str).unwrap_or("");
    let conv_id = params.get("conversation_id").map(String::as_str);

    if query.is_empty() {
        let limit = params
            .get("limit")
            .and_then(|s| s.parse::<usize>().ok())
            .unwrap_or(100);
        let rows = sqlite::list_messages(&state.db, &agent_id, conv_id, limit)
            .map_err(|e| server_err(e.to_string()))?;
        let messages: Vec<Value> = rows
            .into_iter()
            .map(|r| {
                json!({
                    "id":              r.id,
                    "role":            r.role,
                    "content":         r.content,
                    "conversation_id": r.conversation_id,
                    "score":           0.0,
                    "snippet":         Value::Null,
                })
            })
            .collect();
        return Ok(Json(json!({ "messages": messages, "query": query })));
    }

    let rows = sqlite::search_messages(&state.db, &agent_id, query, conv_id)
        .map_err(|e| server_err(e.to_string()))?;
    let messages: Vec<Value> = rows
        .into_iter()
        .map(|r| {
            json!({
                "id":              r.id,
                "role":            r.role,
                "content":         r.content,
                "conversation_id": r.conversation_id,
                "score":           r.score,
                "snippet":         r.snippet,
            })
        })
        .collect();
    Ok(Json(json!({ "messages": messages, "query": query })))
}

/// GET /v1/agents/:id/messages/latest — fetch the most recent assistant turn
pub async fn latest_assistant_message(
    State(state): State<AppState>,
    Path(agent_id): Path<String>,
    axum::extract::Query(params): axum::extract::Query<std::collections::HashMap<String, String>>,
) -> Result<Json<Value>, (StatusCode, Json<Value>)> {
    let conv_id = params.get("conversation_id").map(String::as_str);
    match sqlite::last_assistant_message(&state.db, &agent_id, conv_id) {
        Ok(Some(row)) => Ok(Json(json!({ "message": row }))),
        Ok(None) => Err((
            StatusCode::NOT_FOUND,
            Json(json!({"detail": "no assistant messages"})),
        )),
        Err(e) => Err(server_err(e.to_string())),
    }
}

// -- Conversation endpoints

use cade_store::sqlite::ConversationRow;

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
    let rows =
        sqlite::list_conversations(&state.db, &agent_id).map_err(|e| server_err(e.to_string()))?;
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
            return Err((
                StatusCode::FORBIDDEN,
                Json(json!({"detail": "conversation not owned by this agent"})),
            ));
        }
        Ok(None) => {
            return Err((
                StatusCode::NOT_FOUND,
                Json(json!({"detail": "conversation not found"})),
            ));
        }
        Err(e) => return Err(server_err(e.to_string())),
        Ok(Some(_)) => {}
    }
    let deleted =
        sqlite::delete_conversation(&state.db, &conv_id).map_err(|e| server_err(e.to_string()))?;
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

/// GET /v1/agents/:id/tools — list tools attached to an agent
pub async fn get_agent_tools(
    State(state): State<AppState>,
    Path(agent_id): Path<String>,
) -> Result<Json<Value>, (StatusCode, Json<Value>)> {
    let tools = sqlite::get_agent_tools_with_names(&state.db, &agent_id)
        .map_err(|e| server_err(e.to_string()))?;
    let list: Vec<Value> = tools
        .into_iter()
        .map(|(id, name)| json!({ "id": id, "name": name }))
        .collect();
    Ok(Json(json!(list)))
}

/// DELETE /v1/agents/:id/tools — detach all tools from an agent
pub async fn detach_tools(
    State(state): State<AppState>,
    Path(agent_id): Path<String>,
) -> Result<Json<Value>, (StatusCode, Json<Value>)> {
    let n = sqlite::detach_all_tools_from_agent(&state.db, &agent_id)
        .map_err(|e| server_err(e.to_string()))?;
    Ok(Json(json!({ "detached": n })))
}

fn server_err(msg: String) -> (StatusCode, Json<Value>) {
    tracing::error!("500 Internal Server Error: {msg}");
    (
        StatusCode::INTERNAL_SERVER_ERROR,
        Json(json!({"detail": msg})),
    )
}

/// GET /v1/agents/:id/memory?q=<query>  — search memory blocks by label or value.
/// Returns ranked matches with contextual snippets.
pub async fn search_memory_handler(
    State(state): State<AppState>,
    Path(agent_id): Path<String>,
    axum::extract::Query(params): axum::extract::Query<std::collections::HashMap<String, String>>,
) -> Result<Json<Value>, (StatusCode, Json<Value>)> {
    let query = params.get("q").map(String::as_str).unwrap_or("");
    if query.is_empty() {
        // No ?q= → delegate to get_memory_full (list all with tier)
        let blocks = sqlite::get_memory_blocks_full(&state.db, &agent_id)
            .map_err(|e| server_err(e.to_string()))?;
        let out: Vec<Value> = blocks
            .iter()
            .map(|(l, v, d, t)| {
                json!({
                    "label": l, "value": v, "description": d, "tier": t
                })
            })
            .collect();
        return Ok(Json(json!({ "blocks": out })));
    }

    let rows = sqlite::search_memory(&state.db, &agent_id, query)
        .map_err(|e| server_err(e.to_string()))?;

    // Boost confidence for every block returned by search — relevance weighting.
    for (label, _value, _snippet) in &rows {
        let _ = sqlite::boost_confidence(&state.db, &agent_id, label);
    }

    // Auto-reactivate any long-term blocks returned by search — they're clearly
    // relevant to the current task, so promote back to short-term for 20 turns.
    let full = sqlite::get_memory_blocks_full(&state.db, &agent_id).unwrap_or_default();
    for (label, _value, _snippet) in &rows {
        if let Some((_, _, _, tier)) = full.iter().find(|(l, _, _, _)| l == label)
            && tier == "long"
        {
            let _ = sqlite::set_memory_tier(&state.db, &agent_id, label, "short", true);
        }
    }

    let blocks: Vec<Value> = rows
        .iter()
        .map(|(label, value, snippet)| {
            json!({
                "label":   label,
                "value":   value,
                "snippet": snippet,
            })
        })
        .collect();
    Ok(Json(json!({ "blocks": blocks, "query": query })))
}

#[derive(serde::Deserialize)]
pub struct InsertArchivalReq {
    pub content: String,
    pub tags: Vec<String>,
}

/// POST /v1/agents/:id/archival
pub async fn insert_archival_memory_handler(
    State(state): State<AppState>,
    Path(agent_id): Path<String>,
    Json(payload): Json<InsertArchivalReq>,
) -> Result<Json<Value>, (StatusCode, Json<Value>)> {
    let id = sqlite::insert_archival_memory(&state.db, &agent_id, &payload.content, &payload.tags)
        .map_err(|e| server_err(e.to_string()))?;
    Ok(Json(json!({ "id": id, "status": "success" })))
}

/// GET /v1/agents/:id/archival/search?q=<query>&limit=<limit>
pub async fn search_archival_memory_handler(
    State(state): State<AppState>,
    Path(agent_id): Path<String>,
    axum::extract::Query(params): axum::extract::Query<std::collections::HashMap<String, String>>,
) -> Result<Json<Value>, (StatusCode, Json<Value>)> {
    let query = params.get("q").map(String::as_str).unwrap_or("");
    let limit = params
        .get("limit")
        .and_then(|s| s.parse::<usize>().ok())
        .unwrap_or(10);

    if query.is_empty() {
        return Ok(Json(json!({ "results": [] })));
    }

    let records = sqlite::search_archival_memory(&state.db, &agent_id, query, limit)
        .map_err(|e| server_err(e.to_string()))?;

    Ok(Json(json!({ "results": records })))
}
