//! Wasm-only HTTP client: thin `gloo-net` adapter that delegates all
//! logic to the pure `api` and `sse` modules.
//!
//! This file compiles only for `wasm32`.  It contains **no behaviour**
//! beyond:
//!   1. Issuing HTTP requests via `gloo-net::http::Request`.
//!   2. Reading response status + body (text or streaming).
//!   3. Calling the corresponding pure parser functions.
//!
//! Native `cargo test` therefore has nothing to execute here — every
//! branch is covered by `api::tests::*` and `sse::tests::*`.  If you are
//! tempted to add logic here (e.g. retry, caching, header shaping),
//! push it down into `api` or `sse` so it stays testable on native.

#![cfg(target_arch = "wasm32")]

use cade_api_types::{AgentInfo, ChatMessage, HealthInfo};
use gloo_net::http::Request;
use wasm_bindgen::JsCast as _;
use wasm_bindgen_futures::JsFuture;
use web_sys::js_sys;
use web_sys::js_sys::Uint8Array;

use crate::api::{self, ApiError};
use crate::sse::{SseFrame, SseParser};

// ── One-shot JSON endpoints ─────────────────────────────────────────────

/// `GET /v1/health` — returns the parsed health envelope or a typed error.
///
/// `base_url` must be the scheme + host (+ optional port) of the
/// cade-server instance; trailing slashes are tolerated.
pub async fn get_health(base_url: &str, token: &str) -> Result<HealthInfo, ApiError> {
    let url = api::build_url(base_url, "/v1/health");
    let (status, body) = send_text(&url, token).await?;
    api::parse_health(status, &body)
}

/// `GET /v1/agents` — returns the parsed agent list or a typed error.
pub async fn get_agents(base_url: &str, token: &str) -> Result<Vec<AgentInfo>, ApiError> {
    let url = api::build_url(base_url, "/v1/agents");
    let (status, body) = send_text(&url, token).await?;
    api::parse_agents(status, &body)
}

/// `GET /v1/agents/:id/messages` — returns the parsed message list or a typed error.
pub async fn get_messages(
    base_url: &str,
    token: &str,
    agent_id: &str,
) -> Result<Vec<ChatMessage>, ApiError> {
    let path = format!("/v1/agents/{agent_id}/messages");
    let url = api::build_url(base_url, &path);
    let (status, body) = send_text(&url, token).await?;
    api::parse_messages(status, &body)
}

/// `POST /v1/agents/:id/messages/stream` — send a user message and stream
/// the assistant's response via SSE.
///
/// `on_chunk` is called with each assistant-text fragment.
/// The future resolves when the stream ends or an error occurs.
pub async fn send_message_stream(
    base_url: &str,
    token: &str,
    agent_id: &str,
    input: &str,
    mut on_chunk: impl FnMut(&str),
) -> Result<(), ApiError> {
    let path = format!("/v1/agents/{agent_id}/messages/stream");
    let url = api::build_url(base_url, &path);

    let body = serde_json::json!({ "input": input });

    let resp = Request::post(&url)
        .header("Authorization", &api::bearer_header(token))
        .header("Content-Type", "application/json")
        .body(body.to_string())
        .map_err(|e| ApiError::Transport {
            message: e.to_string(),
        })?
        .send()
        .await
        .map_err(|e| ApiError::Transport {
            message: e.to_string(),
        })?;

    let status = resp.status();
    if status == 401 {
        return Err(ApiError::Unauthorized);
    }
    if !(200..300).contains(&(status as u16)) {
        return Err(ApiError::Server {
            status: status as u16,
        });
    }

    let resp_body = resp.body().ok_or_else(|| ApiError::Transport {
        message: "response has no body".to_string(),
    })?;

    let reader =
        web_sys::ReadableStreamDefaultReader::new(&resp_body).map_err(|e| ApiError::Transport {
            message: format!("failed to get reader: {e:?}"),
        })?;

    let mut parser = SseParser::new();

    loop {
        let result = JsFuture::from(reader.read())
            .await
            .map_err(|e| ApiError::Transport {
                message: format!("read error: {e:?}"),
            })?;

        let done = js_sys::Reflect::get(&result, &"done".into())
            .unwrap_or(true.into())
            .as_bool()
            .unwrap_or(true);

        if !done {
            if let Ok(value) = js_sys::Reflect::get(&result, &"value".into()) {
                if let Ok(arr) = value.dyn_into::<Uint8Array>() {
                    let bytes = arr.to_vec();
                    parser.feed(&bytes);

                    while let Some(frame) = parser.pop() {
                        match &frame {
                            SseFrame::Json(v) => {
                                if v.get("message_type").and_then(|m| m.as_str())
                                    == Some("assistant_message")
                                {
                                    if let Some(text) = v.get("content").and_then(|c| c.as_str()) {
                                        on_chunk(text);
                                    }
                                }
                            }
                            SseFrame::Done => {
                                reader.release_lock();
                                return Ok(());
                            }
                            SseFrame::ParseError(_) => {
                                // Skip malformed frames.
                            }
                        }
                    }
                }
            }
        }

        if done {
            // Drain remaining buffered frames.
            while let Some(frame) = parser.pop() {
                if let SseFrame::Json(v) = &frame {
                    if v.get("message_type").and_then(|m| m.as_str())
                        == Some("assistant_message")
                    {
                        if let Some(text) = v.get("content").and_then(|c| c.as_str()) {
                            on_chunk(text);
                        }
                    }
                }
            }
            break;
        }
    }

    reader.release_lock();
    Ok(())
}

// ── SSE streaming endpoint ──────────────────────────────────────────────

/// Stream SSE frames from an authenticated GET endpoint.
///
/// Opens a `fetch()` request to `url` with `Authorization: Bearer <token>`,
/// then reads the `ReadableStream` body chunk-by-chunk, feeding each chunk
/// into [`SseParser`] and invoking `on_frame` for every complete frame.
///
/// The loop ends when:
///   * The stream closes (reader returns `done: true`).
///   * `on_frame` returns `false` (caller wants to stop early).
///   * A transport error occurs.
///
/// # Why fetch + ReadableStream instead of EventSource?
///
/// `EventSource` does not support custom headers — the browser API has no
/// way to attach `Authorization: Bearer <token>`.  Every cade-server
/// streaming endpoint (except `/v1/health`) requires auth, so we must use
/// the Fetch API with a streaming body reader.
///
/// All SSE parsing logic lives in the pure `sse` module; this function is
/// nothing but I/O glue.
pub async fn stream_sse(
    url: &str,
    token: &str,
    mut on_frame: impl FnMut(SseFrame) -> bool,
) -> Result<(), ApiError> {
    let resp = Request::get(url)
        .header("Authorization", &api::bearer_header(token))
        .send()
        .await
        .map_err(|e| ApiError::Transport {
            message: e.to_string(),
        })?;

    // Check status before attempting to stream the body.
    let status = resp.status();
    if status == 401 {
        return Err(ApiError::Unauthorized);
    }
    if !(200..300).contains(&(status as u16)) {
        return Err(ApiError::Server {
            status: status as u16,
        });
    }

    let body = resp.body().ok_or_else(|| ApiError::Transport {
        message: "response has no body".to_string(),
    })?;

    let reader = web_sys::ReadableStreamDefaultReader::new(&body).map_err(|e| {
        ApiError::Transport {
            message: format!("failed to get reader: {e:?}"),
        }
    })?;

    let mut parser = SseParser::new();

    loop {
        let result = JsFuture::from(reader.read())
            .await
            .map_err(|e| ApiError::Transport {
                message: format!("read error: {e:?}"),
            })?;

        // The read() promise resolves to `{ value: Uint8Array | undefined, done: bool }`.
        let done = js_sys::Reflect::get(&result, &"done".into())
            .unwrap_or(true.into())
            .as_bool()
            .unwrap_or(true);

        if !done {
            if let Ok(value) = js_sys::Reflect::get(&result, &"value".into()) {
                if let Ok(arr) = value.dyn_into::<Uint8Array>() {
                    let bytes = arr.to_vec();
                    parser.feed(&bytes);

                    while let Some(frame) = parser.pop() {
                        if !on_frame(frame) {
                            // Caller signalled stop — release the reader lock.
                            reader.release_lock();
                            return Ok(());
                        }
                    }
                }
            }
        }

        if done {
            // Drain any final frames that were buffered.
            while let Some(frame) = parser.pop() {
                if !on_frame(frame) {
                    break;
                }
            }
            break;
        }
    }

    reader.release_lock();
    Ok(())
}

// ── Internal helpers ────────────────────────────────────────────────────

/// One GET round-trip, returning `(status, body_text)` or a
/// `Transport` error on any `gloo-net` failure.
async fn send_text(url: &str, token: &str) -> Result<(u16, String), ApiError> {
    let req = Request::get(url)
        .header("Authorization", &api::bearer_header(token))
        .build()
        .map_err(|e| ApiError::Transport {
            message: e.to_string(),
        })?;

    let resp = req.send().await.map_err(|e| ApiError::Transport {
        message: e.to_string(),
    })?;

    let status = resp.status();
    let body = resp.text().await.map_err(|e| ApiError::Transport {
        message: e.to_string(),
    })?;
    Ok((status, body))
}
