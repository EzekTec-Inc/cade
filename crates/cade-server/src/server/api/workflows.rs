//! Automated Webhook Workflow Router & Dispatcher.
//!
//! Exposes stateless HTTP endpoints designed to receive third-party webhooks
//! (CI/CD triggers, GitHub/GitLab Actions, Slack slashes) and asynchronously
//! spawn, inject payload parameters, and run CADE's automated backend workflows.

use crate::server::api::messages::persist::persist;
use crate::server::api::run::run_agent_loop;
use crate::server::state::AppState;
use axum::{
    Json,
    extract::{Path, State},
    http::StatusCode,
    response::IntoResponse,
};
use serde_json::{Value, json};

use cade_store::sqlite::{self, AgentRow};

/// Workflow configuration structure loaded from `.cade/workflows/{name}.json`.
#[derive(Debug, serde::Deserialize, serde::Serialize)]
pub struct WorkflowConfig {
    pub name: String,
    pub agent: String,
    pub model: String,
    pub prompt: String,
}

/// Webhook entrypoint to dispatch automated, headless CADE workflow sessions.
pub async fn dispatch_workflow(
    Path(workflow_name): Path<String>,
    State(state): State<AppState>,
    Json(payload): Json<Value>,
) -> impl IntoResponse {
    // 1. Validate workflow name: no path traversal, alphanumeric + hyphens/underscores only
    if workflow_name.is_empty()
        || workflow_name.contains('/')
        || workflow_name.contains('\\')
        || workflow_name.contains("..")
        || !workflow_name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
    {
        return (
            StatusCode::BAD_REQUEST,
            Json(json!({
                "error": "Invalid workflow name. Only alphanumeric characters, hyphens, and underscores are allowed."
            })),
        )
            .into_response();
    }

    // Standardize structured trace logging for observability
    tracing::info!(
        "Workflow Dispatch Webhook Received: '{}' with payload: {}",
        workflow_name,
        serde_json::to_string(&payload).unwrap_or_default()
    );

    // 2. Locate the `.cade/workflows/{workflow_name}.json` file
    let path = std::path::Path::new(".cade/workflows").join(format!("{}.json", workflow_name));
    if !path.exists() {
        return (
            StatusCode::NOT_FOUND,
            Json(json!({
                "error": format!("Workflow '{}' not found on disk.", workflow_name)
            })),
        )
            .into_response();
    }

    // 3. Load and deserialize it into a WorkflowConfig struct
    let content = match std::fs::read_to_string(&path) {
        Ok(c) => c,
        Err(e) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(json!({
                    "error": format!("Failed to read workflow file: {}", e)
                })),
            )
                .into_response();
        }
    };

    let config: WorkflowConfig = match serde_json::from_str(&content) {
        Ok(cfg) => cfg,
        Err(e) => {
            return (
                StatusCode::BAD_REQUEST,
                Json(json!({
                    "error": format!("Malformed workflow JSON config: {}", e)
                })),
            )
                .into_response();
        }
    };

    // 4. Resolve or create Agent
    let agent_id = format!("agent-workflow-{}", workflow_name);
    let agent_exists = matches!(sqlite::get_agent(&state.db, &agent_id), Ok(Some(_)));

    if !agent_exists {
        let row = AgentRow {
            id: agent_id.clone(),
            name: config.agent.clone(),
            model: config.model.clone(),
            description: Some(format!("Automated workflow agent for '{}'", workflow_name)),
            system_prompt: Some(config.prompt.clone()),
            created_at: None,
            compaction_model: None,
            theme: None,
            active_plan_json: None,
            parent_id: None,
        };
        if let Err(e) = sqlite::create_agent(&state.db, &row) {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(json!({
                    "error": format!("Failed to create agent for workflow: {}", e)
                })),
            )
                .into_response();
        }
    }

    // 5. Create a new conversation for this agent run
    let conv_title = format!("Workflow Run: {}", workflow_name);
    let conv = match sqlite::create_conversation(&state.db, &agent_id, &conv_title) {
        Ok(c) => c,
        Err(e) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(json!({
                    "error": format!("Failed to create conversation for workflow: {}", e)
                })),
            )
                .into_response();
        }
    };
    let conv_id = conv.id;

    // 6. Format and persist the initial trigger message containing the webhook payload
    let initial_message = format!(
        "Execute Workflow: '{}'. Input Payload: {}",
        workflow_name,
        serde_json::to_string_pretty(&payload).unwrap_or_default()
    );
    persist(
        &state,
        &agent_id,
        Some(&conv_id),
        "user",
        json!({ "content": initial_message }),
    );

    // 7. Create a run record in the database
    let run = match sqlite::create_run(&state.db, &agent_id, Some(&conv_id)) {
        Ok(r) => r,
        Err(e) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(json!({
                    "error": format!("Failed to create run record in DB: {}", e)
                })),
            )
                .into_response();
        }
    };
    let run_id = run.id;

    // 8. Spawn run_agent_loop asynchronously in a background tokio task
    let state_clone = state.clone();
    let agent_id_clone = agent_id.clone();
    let conv_id_clone = Some(conv_id.clone());
    let run_id_clone = run_id.clone();

    let (tx, mut rx) = tokio::sync::mpsc::channel(128);

    let payload_for_loop = payload.clone();
    tokio::spawn(async move {
        // Run standard agent loop
        let _ = run_agent_loop(
            state_clone,
            agent_id_clone,
            conv_id_clone,
            run_id_clone,
            None, // No specific theme_cmd for automated background runs
            tx,
            payload_for_loop.to_string(),
        )
        .await;
    });

    // Drain and log incoming stream events in the background so the loop executes fully
    tokio::spawn(async move {
        while let Some(event) = rx.recv().await {
            match event {
                Ok(e) => {
                    tracing::debug!(target: "cade::workflow", "Workflow Run Event: {:?}", e);
                }
                Err(_) => break,
            }
        }
    });

    // 9. Return the real run ID as execution_id
    let response_body = json!({
        "status": "accepted",
        "workflow": workflow_name,
        "config": config,
        "payload": payload,
        "execution_id": run_id,
    });

    (StatusCode::ACCEPTED, Json(response_body)).into_response()
}
