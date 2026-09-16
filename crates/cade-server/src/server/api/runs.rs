use axum::response::sse::Event;
use axum::{
    Json,
    extract::{Path, Query, State},
    http::StatusCode,
    response::{IntoResponse, Response, Sse, sse::KeepAlive},
};
use serde_json::{Value, json};
use std::collections::HashMap;

use crate::server::state::AppState;
use cade_store::sqlite;

fn err(status: StatusCode, msg: &str) -> Response {
    (status, Json(json!({ "detail": msg }))).into_response()
}

/// Statuses after which no further run events will be produced.
fn status_is_terminal(status: &str) -> bool {
    matches!(status, "done" | "error" | "cancelled")
}

/// Re-hydrate a persisted event with its `seq_id` and `run_id` so clients can
/// deduplicate replayed events after a reconnect.
fn attach_run_meta(run_id: &str, seq: i64, data: &str) -> Value {
    let mut v: Value = serde_json::from_str(data).unwrap_or(Value::String(data.to_string()));
    if let Some(obj) = v.as_object_mut() {
        obj.insert("seq_id".to_string(), seq.into());
        obj.insert("run_id".to_string(), run_id.to_string().into());
    }
    v
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

/// POST /v1/runs/:run_id/cancel — request durable cancellation for an active run.
pub async fn cancel_run(State(state): State<AppState>, Path(run_id): Path<String>) -> Response {
    match sqlite::request_run_cancellation(&state.db, &run_id) {
        Ok(true) => Json(json!({ "id": run_id, "status": "cancelling" })).into_response(),
        Ok(false) => match sqlite::get_run(&state.db, &run_id) {
            Ok(Some(run)) => Json(json!({ "id": run.id, "status": run.status })).into_response(),
            Ok(None) => err(StatusCode::NOT_FOUND, "run not found"),
            Err(error) => err(StatusCode::INTERNAL_SERVER_ERROR, &error.to_string()),
        },
        Err(error) => err(StatusCode::INTERNAL_SERVER_ERROR, &error.to_string()),
    }
}

/// GET /v1/runs/:run_id/stream?starting_after=<seq_id>
///
/// Replays stored events from `seq_id+1`; if the run is already finished it
/// immediately streams a `[DONE]` terminator.  When the run is still active the
/// endpoint *follows* it: it keeps polling the durable event log and forwards
/// new events until a terminal `run_done` envelope is persisted, and only then
/// emits `[DONE]`.  This is what lets a resuming client wait out a long-running
/// task without receiving a false "done" before the run actually completes.
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

    if status_is_terminal(&run.status) {
        return run_replay_sse(&state, &run_id, after_seq).await;
    }

    // Run is still active — replay outstanding events, then follow to completion.
    run_follow_stream(&state, &run_id, after_seq).await
}

/// Replay stored events after `after_seq`, then emit `[DONE]`.
async fn run_replay_sse(state: &AppState, run_id: &str, after_seq: i64) -> Response {
    let events = match sqlite::run_events_after(&state.db, run_id, after_seq) {
        Ok(e) => e,
        Err(e) => return err(StatusCode::INTERNAL_SERVER_ERROR, &e.to_string()),
    };

    let run_id_owned = run_id.to_string();
    let stream = futures::stream::iter(
        events
            .into_iter()
            .map(move |(seq, data)| {
                let v = attach_run_meta(&run_id_owned, seq, &data);
                Ok::<Event, std::convert::Infallible>(Event::default().data(v.to_string()))
            })
            .chain(std::iter::once(Ok(Event::default().data("[DONE]")))),
    );

    Sse::new(stream).into_response()
}

/// Live-follow session for an active run: replay `seq > after_seq`, then poll
/// the durable event log until the run finishes, then emit `[DONE]`.
async fn run_follow_stream(state: &AppState, run_id: &str, after_seq: i64) -> Response {
    let (tx, rx) = tokio::sync::mpsc::channel::<Result<Event, std::convert::Infallible>>(64);
    let db = state.db.clone();
    let run_id_owned = run_id.to_string();

    tokio::spawn(async move {
        let mut last_seq = after_seq;
        loop {
            if tx.is_closed() {
                return;
            }
            let new_events = match sqlite::run_events_after(&db, &run_id_owned, last_seq) {
                Ok(e) => e,
                Err(error) => {
                    tracing::warn!(%run_id_owned, %error, "follow stream: failed to read run events");
                    let _ = tx.send(Ok(Event::default().data("[DONE]"))).await;
                    return;
                }
            };

            let mut saw_final = false;
            let mut saw_terminal = false;
            for (seq, data) in &new_events {
                last_seq = last_seq.max(*seq);
                let v = attach_run_meta(&run_id_owned, *seq, data);
                if v.get("message_type").and_then(Value::as_str) == Some("run_done") {
                    saw_terminal = true;
                }
                saw_final = true;
                if tx
                    .send(Ok(Event::default().data(v.to_string())))
                    .await
                    .is_err()
                {
                    return;
                }
            }

            if saw_terminal {
                // Durable run_done envelope replayed — the run is over.
                let _ = tx.send(Ok(Event::default().data("[DONE]"))).await;
                return;
            }

            // If the run status has flipped to terminal but the run_done event
            // hasn't been persisted yet (ordering race), keep polling for it.
            let active = match sqlite::get_run(&db, &run_id_owned) {
                Ok(Some(r)) => !status_is_terminal(&r.status),
                _ => true,
            };
            if !active && !saw_final {
                // Status terminal, no new events on this tick and no pending
                // event to deliver — safe to finish the stream.
                let _ = tx.send(Ok(Event::default().data("[DONE]"))).await;
                return;
            }

            tokio::time::sleep(std::time::Duration::from_millis(200)).await;
        }
    });

    Sse::new(tokio_stream::wrappers::ReceiverStream::new(rx))
        .keep_alive(KeepAlive::new().interval(std::time::Duration::from_secs(15)))
        .into_response()
}
