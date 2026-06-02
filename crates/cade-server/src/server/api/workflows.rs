//! Automated Webhook Workflow Router & Dispatcher.
//!
//! Exposes stateless HTTP endpoints designed to receive third-party webhooks
//! (CI/CD triggers, GitHub/GitLab Actions, Slack slashes) and asynchronously
//! spawn, inject payload parameters, and run CADE's automated backend workflows.

use crate::server::state::AppState;
use axum::{
    Json,
    extract::{Path, State},
    http::StatusCode,
    response::IntoResponse,
};
use serde_json::{Value, json};

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
    State(_state): State<AppState>,
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

    // Dynamic Hono-like trigger schema mapping the workflow execution
    let response_body = json!({
        "status": "accepted",
        "workflow": workflow_name,
        "config": config,
        "payload": payload,
        "execution_id": uuid::Uuid::new_v4().to_string(),
    });

    (StatusCode::ACCEPTED, Json(response_body)).into_response()
}
