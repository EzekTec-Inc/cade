//! HTTP and webhook adapters for the same canonical workflow engine.

use crate::server::state::AppState;
pub use crate::server::workflows::WorkflowConfig;
use crate::server::workflows::{WorkflowEngine, run_summary};
use axum::{
    Json,
    extract::{Path, State},
    http::StatusCode,
    response::{
        IntoResponse, Response,
        sse::{Event, KeepAlive, Sse},
    },
};
use cade_store::sqlite::get_workflow_run;
use serde_json::{Value, json};
use tokio_stream::{StreamExt, wrappers::BroadcastStream};

fn error(status: StatusCode, message: impl Into<String>) -> Response {
    (status, Json(json!({ "error": message.into() }))).into_response()
}

pub async fn list_workflows_handler(State(state): State<AppState>) -> Response {
    match WorkflowEngine::new(state).list_workflows().await {
        Ok(workflows) => Json(json!({ "workflows": workflows })).into_response(),
        Err(message) => error(StatusCode::INTERNAL_SERVER_ERROR, message),
    }
}

pub async fn get_workflow_handler(
    Path(name): Path<String>,
    State(state): State<AppState>,
) -> Response {
    match WorkflowEngine::new(state).definition(&name) {
        Ok(Some(definition)) => Json(json!(definition)).into_response(),
        Ok(None) => error(
            StatusCode::NOT_FOUND,
            format!("Workflow '{name}' not found"),
        ),
        Err(message) => error(StatusCode::BAD_REQUEST, message),
    }
}

pub async fn run_workflow_handler(
    Path(name): Path<String>,
    State(state): State<AppState>,
    Json(params): Json<Value>,
) -> Response {
    dispatch_named(state, name, params, false).await
}

/// Webhooks use the same dependency ordering, run records, outcomes and cancellation.
pub async fn dispatch_workflow(
    Path(name): Path<String>,
    State(state): State<AppState>,
    Json(payload): Json<Value>,
) -> Response {
    dispatch_named(state, name, payload, true).await
}

async fn dispatch_named(state: AppState, name: String, params: Value, webhook: bool) -> Response {
    let engine = WorkflowEngine::new(state);
    let definition = match engine.definition(&name) {
        Ok(Some(definition)) => definition,
        Ok(None) => {
            return error(
                StatusCode::NOT_FOUND,
                format!("Workflow '{name}' not found"),
            );
        }
        Err(message) => return error(StatusCode::BAD_REQUEST, message),
    };
    if let Err(message) = engine.prepare_legacy_agent(&name) {
        return error(StatusCode::BAD_REQUEST, message);
    }
    match engine.dispatch_with_execution(definition, params).await {
        Ok((accepted, _events)) => {
            let mut body = json!({
                "run_id": accepted.run_id, "execution_id": accepted.execution_id,
                "agent_id": accepted.agent_id, "workflow": name,
                "status": if webhook { "triggered" } else { "running" },
            });
            if webhook {
                body["message"] =
                    json!("Workflow spawned and running asynchronously in background.");
            }
            (StatusCode::ACCEPTED, Json(body)).into_response()
        }
        Err(message) => error(StatusCode::BAD_REQUEST, message),
    }
}

pub async fn get_workflow_run_handler(
    Path(run_id): Path<String>,
    State(state): State<AppState>,
) -> Response {
    match get_workflow_run(&state.db, &run_id) {
        Ok(Some(record)) => Json(json!(run_summary(record))).into_response(),
        Ok(None) => error(StatusCode::NOT_FOUND, "Workflow run not found"),
        Err(message) => error(StatusCode::INTERNAL_SERVER_ERROR, message.to_string()),
    }
}

pub async fn stream_workflow_run_handler(
    Path(run_id): Path<String>,
    State(state): State<AppState>,
) -> Result<Sse<impl futures::Stream<Item = Result<Event, std::convert::Infallible>>>, StatusCode> {
    let receiver = WorkflowEngine::new(state)
        .subscribe_events(&run_id)
        .await
        .ok_or(StatusCode::NOT_FOUND)?;
    let stream = BroadcastStream::new(receiver).filter_map(|result| match result {
        Ok(event) => Some(Ok(Event::default().data(json!(event).to_string()))),
        Err(_) => None,
    });
    Ok(Sse::new(stream).keep_alive(KeepAlive::default()))
}

pub async fn cancel_workflow_run_handler(
    Path(run_id): Path<String>,
    State(state): State<AppState>,
) -> Response {
    match WorkflowEngine::new(state).cancel(&run_id).await {
        Ok(true) => Json(json!({ "status": "cancelling", "run_id": run_id })).into_response(),
        Ok(false) => error(
            StatusCode::NOT_FOUND,
            "Active workflow run not found or already completed",
        ),
        Err(message) => error(StatusCode::INTERNAL_SERVER_ERROR, message),
    }
}
