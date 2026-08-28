//! Automated Webhook Workflow Router & Dispatcher (PRD #99 / Issue #101).

use crate::server::api::run::run_agent_loop;
use crate::server::state::AppState;
use crate::server::workflows::{WorkflowDef, WorkflowEngine};
use axum::{
    Json,
    extract::{Path, State},
    http::StatusCode,
    response::{
        IntoResponse,
        sse::{Event, Sse},
    },
};
use cade_api_types::WorkflowStatus;
use serde_json::{Value, json};
use tokio_stream::StreamExt;
use tokio_stream::wrappers::BroadcastStream;

use cade_store::sqlite::{self, AgentRow, get_workflow_run};

/// Workflow configuration structure loaded from `.cade/workflows/{name}.json`.
#[derive(Debug, serde::Deserialize, serde::Serialize)]
pub struct WorkflowConfig {
    pub name: String,
    pub agent: String,
    pub model: String,
    pub prompt: String,
}

/// GET /v1/workflows — List all registered workflow summaries.
pub async fn list_workflows_handler(State(state): State<AppState>) -> impl IntoResponse {
    let engine = WorkflowEngine::new(state.db.clone());
    let workflows = engine.list_workflows().await;
    (StatusCode::OK, Json(json!({ "workflows": workflows }))).into_response()
}

/// POST /v1/workflows/:workflow_name/run — Dispatch a workflow run.
pub async fn run_workflow_handler(
    Path(workflow_name): Path<String>,
    State(state): State<AppState>,
    Json(params): Json<Value>,
) -> impl IntoResponse {
    let engine = WorkflowEngine::new(state.db.clone());
    let builtins = WorkflowEngine::builtin_workflows();

    let def = if let Some(found) = builtins.into_iter().find(|w| w.name == workflow_name) {
        found
    } else {
        WorkflowDef {
            name: workflow_name.clone(),
            description: format!("Custom workflow pipeline: {workflow_name}"),
            steps: vec![cade_api_types::WorkflowStepDef {
                name: "default-step".to_string(),
                agent: Some("worker".to_string()),
                prompt: format!("Execute workflow {workflow_name}"),
                depends_on: vec![],
            }],
        }
    };

    let (run_id, _rx) = engine.dispatch(def, params).await;
    (
        StatusCode::ACCEPTED,
        Json(json!({
            "run_id": run_id,
            "status": "running"
        })),
    )
        .into_response()
}

/// GET /v1/workflows/runs/:run_id — Query workflow run summary.
pub async fn get_workflow_run_handler(
    Path(run_id): Path<String>,
    State(state): State<AppState>,
) -> impl IntoResponse {
    match get_workflow_run(&state.db, &run_id) {
        Ok(Some(record)) => {
            let summary = cade_api_types::WorkflowRunSummary {
                run_id: record.run_id,
                workflow_name: record.workflow_name,
                status: match record.status.as_str() {
                    "running" => WorkflowStatus::Running,
                    "succeeded" => WorkflowStatus::Succeeded,
                    "failed" => WorkflowStatus::Failed,
                    "cancelled" => WorkflowStatus::Cancelled,
                    "skipped" => WorkflowStatus::Skipped,
                    _ => WorkflowStatus::Pending,
                },
                created_at: record.created_at,
                completed_at: record.completed_at,
                current_step: record.current_step,
                total_steps: record.total_steps,
                error: record.error,
            };
            (StatusCode::OK, Json(json!(summary))).into_response()
        }
        Ok(None) => (
            StatusCode::NOT_FOUND,
            Json(json!({ "error": "Workflow run not found" })),
        )
            .into_response(),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({ "error": e.to_string() })),
        )
            .into_response(),
    }
}

/// GET /v1/workflows/runs/:run_id/stream — Stream live workflow step events over SSE.
pub async fn stream_workflow_run_handler(
    Path(run_id): Path<String>,
    State(state): State<AppState>,
) -> Result<Sse<impl futures::Stream<Item = Result<Event, std::convert::Infallible>>>, StatusCode> {
    let engine = WorkflowEngine::new(state.db.clone());
    let rx = engine.subscribe_events(&run_id).await.ok_or(StatusCode::NOT_FOUND)?;

    let stream = BroadcastStream::new(rx).filter_map(|res| match res {
        Ok(ev) => {
            let json_data = serde_json::to_string(&ev).unwrap_or_default();
            Some(Ok(Event::default().data(json_data)))
        }
        Err(_) => None,
    });

    Ok(Sse::new(stream))
}

/// POST /v1/workflows/runs/:run_id/cancel — Cancel an in-flight workflow run.
pub async fn cancel_workflow_run_handler(
    Path(run_id): Path<String>,
    State(state): State<AppState>,
) -> impl IntoResponse {
    let engine = WorkflowEngine::new(state.db.clone());
    let cancelled = engine.cancel(&run_id).await;

    if cancelled {
        (StatusCode::OK, Json(json!({ "status": "cancelled", "run_id": run_id }))).into_response()
    } else {
        (
            StatusCode::NOT_FOUND,
            Json(json!({ "error": "Active workflow run not found or already completed" })),
        )
            .into_response()
    }
}

/// Webhook entrypoint to dispatch automated, headless CADE workflow sessions.
pub async fn dispatch_workflow(
    Path(workflow_name): Path<String>,
    State(state): State<AppState>,
    Json(payload): Json<Value>,
) -> impl IntoResponse {
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

    tracing::info!(
        "Workflow Dispatch Webhook Received: '{}' with payload: {}",
        workflow_name,
        serde_json::to_string(&payload).unwrap_or_default()
    );

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

    let prompt_input = serde_json::to_string_pretty(&payload).unwrap_or_default();
    let prompt = format!(
        "Automated Webhook Payload Received for Workflow '{}':\n\n```json\n{}\n```\nExecute your designated prompt instructions.",
        workflow_name, prompt_input
    );

    let run_row = match sqlite::create_run(&state.db, &agent_id, None) {
        Ok(r) => r,
        Err(e) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(json!({
                    "error": format!("Failed to create run record: {}", e)
                })),
            )
                .into_response();
        }
    };
    let execution_id = run_row.id.clone();

    let (tx, _rx) = tokio::sync::mpsc::channel(100);
    let loop_run_id = execution_id.clone();
    let loop_state = state.clone();
    let loop_agent_id = agent_id.clone();

    tokio::spawn(async move {
        run_agent_loop(
            loop_state.clone(),
            loop_agent_id.clone(),
            None,
            loop_run_id,
            None,
            tx,
            prompt,
        )
        .await;
    });

    (
        StatusCode::ACCEPTED,
        Json(json!({
            "status": "triggered",
            "execution_id": execution_id,
            "workflow": workflow_name,
            "agent_id": agent_id,
            "message": "Workflow spawned and running asynchronously in background."
        })),
    )
        .into_response()
}
