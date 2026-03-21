use axum::response::sse::Event;
use axum::{
    Json,
    extract::{Path, Query, State},
    http::StatusCode,
    response::{IntoResponse, Response, Sse},
};
use serde_json::{Value, json};
use std::collections::HashMap;

use crate::server::{state::AppState, storage::sqlite};

fn err(status: StatusCode, msg: &str) -> Response {
    (status, Json(json!({ "detail": msg }))).into_response()
}

/// GET /v1/runs/:run_id — run status + last seq_id
pub async fn get_run(State(state): State<AppState>, Path(run_id): Path<String>) -> Response {
    match sqlite::get_run(&state.db, &run_id) {
        Ok(Some(r)) => {
            // Find last seq_id
            let last_seq: i64 = sqlite::run_events_after(&state.db, &run_id, -1)
                .ok()
                .and_then(|evs| evs.last().map(|(s, _)| *s))
                .unwrap_or(-1);
            Json(json!({
                "id":              r.id,
                "agent_id":        r.agent_id,
                "conversation_id": r.conversation_id,
                "status":          r.status,
                "last_seq_id":     last_seq,
                "created_at":      r.created_at,
                "updated_at":      r.updated_at,
            }))
            .into_response()
        }
        Ok(None) => err(StatusCode::NOT_FOUND, "run not found"),
        Err(e) => err(StatusCode::INTERNAL_SERVER_ERROR, &e.to_string()),
    }
}

/// GET /v1/runs/:run_id/stream?starting_after=<seq_id>
/// Replays stored events from seq_id+1, then if run is still 'running'
/// streams a [DONE] to let the client know to poll again.
pub async fn stream_run(
    State(state): State<AppState>,
    Path(run_id): Path<String>,
    Query(params): Query<HashMap<String, String>>,
) -> Response {
    let after_seq: i64 = params
        .get("starting_after")
        .and_then(|s| s.parse().ok())
        .unwrap_or(-1);

    let run = match sqlite::get_run(&state.db, &run_id) {
        Ok(Some(r)) => r,
        Ok(None) => return err(StatusCode::NOT_FOUND, "run not found"),
        Err(e) => return err(StatusCode::INTERNAL_SERVER_ERROR, &e.to_string()),
    };

    let events = match sqlite::run_events_after(&state.db, &run_id, after_seq) {
        Ok(e) => e,
        Err(e) => return err(StatusCode::INTERNAL_SERVER_ERROR, &e.to_string()),
    };

    let is_done = run.status == "completed" || run.status == "failed";

    // Build the replay stream
    let mut sse_events: Vec<Value> = events
        .into_iter()
        .map(|(seq, data)| {
            // Re-attach seq_id in case it's missing
            let mut v: Value = serde_json::from_str(&data).unwrap_or(Value::String(data));
            if let Some(obj) = v.as_object_mut() {
                obj.insert("seq_id".to_string(), seq.into());
                obj.insert("run_id".to_string(), run_id.clone().into());
            }
            v
        })
        .collect();

    if is_done {
        sse_events.push(json!({ "message_type": "run_done", "status": run.status }));
    }

    let stream = futures::stream::iter(
        sse_events
            .into_iter()
            .map(|v| Ok::<Event, std::convert::Infallible>(Event::default().data(v.to_string())))
            .chain(std::iter::once(Ok(Event::default().data("[DONE]")))),
    );

    Sse::new(stream).into_response()
}
