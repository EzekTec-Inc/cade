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

/// `GET /v1/agents/:id/messages?limit=<limit>&offset=<offset>` — paged messages.
pub async fn get_messages_paged(
    base_url: &str,
    token: &str,
    agent_id: &str,
    limit: usize,
    offset: usize,
    conversation_id: Option<&str>,
) -> Result<(Vec<ChatMessage>, bool), ApiError> {
    let mut path = format!("/v1/agents/{agent_id}/messages?limit={limit}&offset={offset}");
    if let Some(cid) = conversation_id {
        path.push_str(&format!("&conversation_id={cid}"));
    }
    let url = api::build_url(base_url, &path);
    let (status, body) = send_text(&url, token).await?;
    api::parse_messages_paged(status, &body)
}

/// `GET /v1/agents/:id/messages?conversation_id=<cid>` — messages for one conversation.
pub async fn get_messages_for_conversation(
    base_url: &str,
    token: &str,
    agent_id: &str,
    conversation_id: &str,
) -> Result<Vec<ChatMessage>, ApiError> {
    let path = format!("/v1/agents/{agent_id}/messages?conversation_id={conversation_id}");
    let url = api::build_url(base_url, &path);
    let (status, body) = send_text(&url, token).await?;
    api::parse_messages(status, &body)
}

pub async fn search_messages(
    base_url: &str,
    token: &str,
    agent_id: &str,
    query: &str,
) -> Result<Vec<serde_json::Value>, ApiError> {
    let path = format!("/v1/agents/{agent_id}/messages?q={}", urlencoding::encode(query));
    let url = api::build_url(base_url, &path);
    let (status, body) = send_text(&url, token).await?;
    if status != 200 {
        return Err(ApiError::Server { status });
    }
    let v: serde_json::Value = serde_json::from_str(&body).map_err(|e| ApiError::Decode {
        message: e.to_string(),
    })?;
    Ok(v["messages"].as_array().cloned().unwrap_or_default())
}

pub async fn search_memory(
    base_url: &str,
    token: &str,
    agent_id: &str,
    query: &str,
) -> Result<Vec<serde_json::Value>, ApiError> {
    let path = format!("/v1/agents/{agent_id}/memory?q={}", urlencoding::encode(query));
    let url = api::build_url(base_url, &path);
    let (status, body) = send_text(&url, token).await?;
    if status != 200 {
        return Err(ApiError::Server { status });
    }
    let v: serde_json::Value = serde_json::from_str(&body).map_err(|e| ApiError::Decode {
        message: e.to_string(),
    })?;
    Ok(v["blocks"].as_array().cloned().unwrap_or_default())
}

/// `GET /v1/agents/:id/conversations` — list conversations for an agent.
pub async fn get_conversations(
    base_url: &str,
    token: &str,
    agent_id: &str,
) -> Result<Vec<api::ConversationInfo>, ApiError> {
    let url = api::conversations_url(base_url, agent_id);
    let (status, body) = send_text(&url, token).await?;
    api::parse_conversations(status, &body)
}

/// `POST /v1/agents/:id/conversations` — create a new conversation.
#[allow(dead_code)]
pub async fn create_conversation(
    base_url: &str,
    token: &str,
    agent_id: &str,
    title: &str,
) -> Result<api::ConversationInfo, ApiError> {
    let url = api::conversations_url(base_url, agent_id);
    let body_json = serde_json::json!({ "title": title });
    let resp = Request::post(&url)
        .header("Authorization", &api::bearer_header(token))
        .header("Content-Type", "application/json")
        .body(body_json.to_string())
        .map_err(|e| ApiError::Transport {
            message: format!("{e:?}"),
        })?
        .send()
        .await
        .map_err(|e| ApiError::Transport {
            message: e.to_string(),
        })?;
    let status = resp.status();
    let text = resp.text().await.map_err(|e| ApiError::Transport {
        message: e.to_string(),
    })?;
    let info: api::ConversationInfo = crate::api::decode_conversations_single(status, &text)?;
    Ok(info)
}

/// `DELETE /v1/agents/:id/conversations/:conv_id` — delete a conversation.
pub async fn delete_conversation(
    base_url: &str,
    token: &str,
    agent_id: &str,
    conv_id: &str,
) -> Result<(), ApiError> {
    let url = api::conversation_url(base_url, agent_id, conv_id);
    let resp = Request::delete(&url)
        .header("Authorization", &api::bearer_header(token))
        .send()
        .await
        .map_err(|e| ApiError::Transport {
            message: e.to_string(),
        })?;
    api::classify_upsert(resp.status())
}

/// `POST /v1/agents/:id/run` — send a user message and run the full
/// server-side agentic loop (tool execution included), streaming all events
/// (text, tool calls, tool results, finish) back via SSE.
///
/// `on_event` is called for each parsed [`api::StreamEvent`].
/// Returns `Ok(())` when the stream ends normally.
pub async fn send_message_stream(
    base_url: &str,
    token: &str,
    agent_id: &str,
    input: &str,
    conversation_id: Option<&str>,
    mut on_event: impl FnMut(api::StreamEvent),
) -> Result<(), ApiError> {
    let path = format!("/v1/agents/{agent_id}/run");
    let url = api::build_url(base_url, &path);

    let mut body = serde_json::json!({ "input": input });
    if let Some(cid) = conversation_id {
        body["conversation_id"] = serde_json::Value::String(cid.to_string());
    }

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
                                if let Some(evt) = api::parse_stream_event(v) {
                                    on_event(evt);
                                }
                            }
                            SseFrame::Done => {
                                reader.release_lock();
                                return Ok(());
                            }
                            SseFrame::ParseError(_) => {}
                        }
                    }
                }
            }
        }

        if done {
            while let Some(frame) = parser.pop() {
                if let SseFrame::Json(v) = &frame {
                    if let Some(evt) = api::parse_stream_event(v) {
                        on_event(evt);
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

    let reader =
        web_sys::ReadableStreamDefaultReader::new(&body).map_err(|e| ApiError::Transport {
            message: format!("failed to get reader: {e:?}"),
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

/// `GET /v1/agents/:id/memory` — returns all memory blocks or a typed error.
pub async fn get_memory(
    base_url: &str,
    token: &str,
    agent_id: &str,
) -> Result<Vec<api::MemoryBlock>, ApiError> {
    let url = api::memory_url(base_url, agent_id);
    let (status, body) = send_text(&url, token).await?;
    api::parse_memory(status, &body)
}

/// `PUT /v1/agents/:id/memory/:label` — upsert a memory block's value.
pub async fn put_memory_block(
    base_url: &str,
    token: &str,
    agent_id: &str,
    label: &str,
    value: &str,
    description: Option<&str>,
) -> Result<(), ApiError> {
    let url = api::memory_block_url(base_url, agent_id, label);
    let body = api::upsert_memory_body(value, description);
    let resp = Request::put(&url)
        .header("Authorization", &api::bearer_header(token))
        .header("Content-Type", "application/json")
        .body(body)
        .map_err(|e| ApiError::Transport {
            message: format!("{e:?}"),
        })?
        .send()
        .await
        .map_err(|e| ApiError::Transport {
            message: e.to_string(),
        })?;
    api::classify_upsert(resp.status())
}

/// `PATCH /v1/agents/:id` — update the agent's model (only field we
/// currently expose).  Returns `Ok(())` on 2xx, a typed error otherwise.
pub async fn patch_agent_model(
    base_url: &str,
    token: &str,
    agent_id: &str,
    model: &str,
) -> Result<(), ApiError> {
    let url = api::agent_url(base_url, agent_id);
    let body = api::patch_agent_model_body(model);
    let resp = Request::patch(&url)
        .header("Authorization", &api::bearer_header(token))
        .header("Content-Type", "application/json")
        .body(body)
        .map_err(|e| ApiError::Transport {
            message: format!("{e:?}"),
        })?
        .send()
        .await
        .map_err(|e| ApiError::Transport {
            message: e.to_string(),
        })?;
    api::classify_upsert(resp.status())
}

/// `PATCH /v1/agents/:id` — update the agent's compaction (summarisation)
/// model.  Pass an empty string to clear the override and fall back to the
/// auto-cheapest resolver server-side.  Returns `Ok(())` on 2xx.
pub async fn patch_agent_compaction_model(
    base_url: &str,
    token: &str,
    agent_id: &str,
    model: &str,
) -> Result<(), ApiError> {
    let url = api::agent_url(base_url, agent_id);
    let body = api::patch_agent_compaction_model_body(model);
    let resp = Request::patch(&url)
        .header("Authorization", &api::bearer_header(token))
        .header("Content-Type", "application/json")
        .body(body)
        .map_err(|e| ApiError::Transport {
            message: format!("{e:?}"),
        })?
        .send()
        .await
        .map_err(|e| ApiError::Transport {
            message: e.to_string(),
        })?;
    api::classify_upsert(resp.status())
}

// ── Checkpoints ────────────────────────────────────────────────────────

/// `GET /v1/agents/:id/checkpoints` — list checkpoints for an agent.
pub async fn get_checkpoints(
    base_url: &str,
    token: &str,
    agent_id: &str,
) -> Result<Vec<api::CheckpointRow>, ApiError> {
    let url = api::checkpoints_url(base_url, agent_id);
    let (status, body) = send_text(&url, token).await?;
    api::parse_checkpoints(status, &body)
}

/// `POST /v1/agents/:id/checkpoints` — create a new checkpoint.
///
/// Returns `Ok(())` on 2xx; errors are surfaced as [`ApiError`].  The
/// caller is expected to refresh the list after creation.
pub async fn create_checkpoint(
    base_url: &str,
    token: &str,
    agent_id: &str,
    label: Option<&str>,
    description: Option<&str>,
    conversation_id: Option<&str>,
) -> Result<(), ApiError> {
    let url = api::checkpoints_url(base_url, agent_id);
    let body = api::create_checkpoint_body(label, description, conversation_id);
    let resp = Request::post(&url)
        .header("Authorization", &api::bearer_header(token))
        .header("Content-Type", "application/json")
        .body(body)
        .map_err(|e| ApiError::Transport {
            message: format!("{e:?}"),
        })?
        .send()
        .await
        .map_err(|e| ApiError::Transport {
            message: e.to_string(),
        })?;
    api::classify_upsert(resp.status())
}

/// `DELETE /v1/agents/:id/checkpoints/:cp_id` — remove a checkpoint.
pub async fn delete_checkpoint(
    base_url: &str,
    token: &str,
    agent_id: &str,
    cp_id: &str,
) -> Result<(), ApiError> {
    let url = api::checkpoint_url(base_url, agent_id, cp_id);
    let resp = Request::delete(&url)
        .header("Authorization", &api::bearer_header(token))
        .send()
        .await
        .map_err(|e| ApiError::Transport {
            message: e.to_string(),
        })?;
    api::classify_upsert(resp.status())
}

/// `POST /v1/agents/:id/checkpoints/:cp_id/restore` — restore a checkpoint.
pub async fn restore_checkpoint(
    base_url: &str,
    token: &str,
    agent_id: &str,
    cp_id: &str,
) -> Result<(), ApiError> {
    let url = api::checkpoint_restore_url(base_url, agent_id, cp_id);
    let resp = Request::post(&url)
        .header("Authorization", &api::bearer_header(token))
        .send()
        .await
        .map_err(|e| ApiError::Transport {
            message: e.to_string(),
        })?;
    api::classify_upsert(resp.status())
}

// ── Artifacts ──────────────────────────────────────────────────────────

/// `GET /v1/agents/:id/artifacts` — list artifact summaries.
pub async fn get_artifacts(
    base_url: &str,
    token: &str,
    agent_id: &str,
) -> Result<Vec<api::ArtifactInfo>, ApiError> {
    let url = api::artifacts_url(base_url, agent_id);
    let (status, body) = send_text(&url, token).await?;
    api::parse_artifacts(status, &body)
}

/// `GET /v1/agents/:id/artifacts/:art_id` — fetch full artifact detail.
pub async fn get_artifact(
    base_url: &str,
    token: &str,
    agent_id: &str,
    art_id: &str,
) -> Result<api::ArtifactDetail, ApiError> {
    let url = api::artifact_url(base_url, agent_id, art_id);
    let (status, body) = send_text(&url, token).await?;
    api::parse_artifact(status, &body)
}

/// `DELETE /v1/agents/:id/artifacts/:art_id` — remove an artifact.
pub async fn delete_artifact(
    base_url: &str,
    token: &str,
    agent_id: &str,
    art_id: &str,
) -> Result<(), ApiError> {
    let url = api::artifact_url(base_url, agent_id, art_id);
    let resp = Request::delete(&url)
        .header("Authorization", &api::bearer_header(token))
        .send()
        .await
        .map_err(|e| ApiError::Transport {
            message: e.to_string(),
        })?;
    api::classify_upsert(resp.status())
}

// ── Tools (MCP / skills panel) ─────────────────────────────────────────

/// `GET /v1/agents/:id/tools` — list MCP tools registered with the agent.
pub async fn get_tools(
    base_url: &str,
    token: &str,
    agent_id: &str,
) -> Result<Vec<api::AgentTool>, ApiError> {
    let url = api::tools_url(base_url, agent_id);
    let (status, body) = send_text(&url, token).await?;
    api::parse_tools(status, &body)
}

// ── Metrics + context stats ────────────────────────────────────────────

/// `GET /v1/agents/:id/metrics`
pub async fn get_metrics(
    base_url: &str,
    token: &str,
    agent_id: &str,
) -> Result<api::AgentMetrics, ApiError> {
    let url = api::metrics_url(base_url, agent_id);
    let (status, body) = send_text(&url, token).await?;
    api::parse_metrics(status, &body)
}

/// `GET /v1/agents/:id/context`
pub async fn get_context_stats(
    base_url: &str,
    token: &str,
    agent_id: &str,
) -> Result<api::ContextStats, ApiError> {
    let url = api::context_url(base_url, agent_id);
    let (status, body) = send_text(&url, token).await?;
    api::parse_context_stats(status, &body)
}

/// `GET /v1/agents/:id/context-breakdown` — per-category context breakdown.
pub async fn get_context_breakdown(
    base_url: &str,
    token: &str,
    agent_id: &str,
) -> Result<api::ContextBreakdown, ApiError> {
    let url = api::context_breakdown_url(base_url, agent_id);
    let (status, body) = send_text(&url, token).await?;
    api::parse_context_breakdown(status, &body)
}

/// `GET /v1/models` — list all available models from all providers.
pub async fn get_models(
    base_url: &str,
    token: &str,
) -> Result<(Vec<api::ModelInfo>, Vec<String>), ApiError> {
    let url = api::models_url(base_url);
    let (status, body) = send_text(&url, token).await?;
    api::parse_models(status, &body)
}

/// `GET /v1/mcp` — list all MCP servers loaded by the server.
pub async fn get_mcp_status(
    base_url: &str,
    token: &str,
) -> Result<Vec<api::McpServerInfo>, ApiError> {
    let url = api::mcp_url(base_url);
    let (status, body) = send_text(&url, token).await?;
    api::parse_mcp_status(status, &body)
}

// ── Providers ───────────────────────────────────────────────────────────

/// `GET /v1/providers` — list configured providers.
pub async fn get_providers(
    base_url: &str,
    token: &str,
) -> Result<Vec<serde_json::Value>, ApiError> {
    let url = api::build_url(base_url, "/v1/providers");
    let (status, body) = send_text(&url, token).await?;
    if status != 200 {
        return Err(ApiError::Server { status });
    }
    let v: serde_json::Value = serde_json::from_str(&body).map_err(|e| ApiError::Decode {
        message: e.to_string(),
    })?;
    Ok(v["providers"].as_array().cloned().unwrap_or_default())
}

// ── Skills ──────────────────────────────────────────────────────────────

/// `GET /v1/skills` — list all discovered skills.
pub async fn get_all_skills(base_url: &str, token: &str) -> Result<Vec<api::SkillEntry>, ApiError> {
    let url = api::build_url(base_url, "/v1/skills");
    let (status, body) = send_text(&url, token).await?;
    if status != 200 {
        return Err(ApiError::Server { status });
    }
    let v: serde_json::Value = serde_json::from_str(&body).map_err(|e| ApiError::Decode {
        message: e.to_string(),
    })?;
    let skills: Vec<api::SkillEntry> = v["skills"]
        .as_array()
        .map(|arr| {
            arr.iter()
                .filter_map(|s| serde_json::from_value::<api::SkillEntry>(s.clone()).ok())
                .collect()
        })
        .unwrap_or_default();
    Ok(skills)
}

/// `GET /v1/agents/:id/skills` — list loaded skill IDs for an agent.
pub async fn get_agent_skills(
    base_url: &str,
    token: &str,
    agent_id: &str,
) -> Result<Vec<String>, ApiError> {
    let url = api::build_url(base_url, &format!("/v1/agents/{agent_id}/skills"));
    let (status, body) = send_text(&url, token).await?;
    if status != 200 {
        return Err(ApiError::Server { status });
    }
    let v: serde_json::Value = serde_json::from_str(&body).map_err(|e| ApiError::Decode {
        message: e.to_string(),
    })?;
    Ok(v["loaded_skill_ids"]
        .as_array()
        .map(|a| {
            a.iter()
                .filter_map(|s| s.as_str().map(String::from))
                .collect::<Vec<String>>()
        })
        .unwrap_or_default())
}

/// `POST /v1/agents/:id/skills/load` — load a skill by ID.
pub async fn post_load_skill(
    base_url: &str,
    token: &str,
    agent_id: &str,
    skill_id: &str,
) -> Result<(), ApiError> {
    let url = api::build_url(base_url, &format!("/v1/agents/{agent_id}/skills/load"));
    let body_str = serde_json::json!({ "id": skill_id }).to_string();
    let resp = Request::post(&url)
        .header("Authorization", &api::bearer_header(token))
        .header("Content-Type", "application/json")
        .body(body_str)
        .map_err(|e| ApiError::Transport {
            message: format!("{e:?}"),
        })?
        .send()
        .await
        .map_err(|e| ApiError::Transport {
            message: e.to_string(),
        })?;
    api::classify_upsert(resp.status())
}

/// `POST /v1/agents/:id/skills/unload` — unload a skill by ID.
pub async fn post_unload_skill(
    base_url: &str,
    token: &str,
    agent_id: &str,
    skill_id: &str,
) -> Result<(), ApiError> {
    let url = api::build_url(base_url, &format!("/v1/agents/{agent_id}/skills/unload"));
    let body_str = serde_json::json!({ "id": skill_id }).to_string();
    let resp = Request::post(&url)
        .header("Authorization", &api::bearer_header(token))
        .header("Content-Type", "application/json")
        .body(body_str)
        .map_err(|e| ApiError::Transport {
            message: format!("{e:?}"),
        })?
        .send()
        .await
        .map_err(|e| ApiError::Transport {
            message: e.to_string(),
        })?;
    api::classify_upsert(resp.status())
}

/// `PATCH /v1/agents/:id` — update reasoning effort.
pub async fn patch_agent_reasoning(
    base_url: &str,
    token: &str,
    agent_id: &str,
    effort: &str,
) -> Result<(), ApiError> {
    let url = api::agent_url(base_url, agent_id);
    let body_str = serde_json::json!({ "reasoning_effort": effort }).to_string();
    let resp = Request::patch(&url)
        .header("Authorization", &api::bearer_header(token))
        .header("Content-Type", "application/json")
        .body(body_str)
        .map_err(|e| ApiError::Transport {
            message: format!("{e:?}"),
        })?
        .send()
        .await
        .map_err(|e| ApiError::Transport {
            message: e.to_string(),
        })?;
    api::classify_upsert(resp.status())
}

/// `POST /v1/agents/:id/compact` — synchronously trigger session-summary
/// consolidation. Returns the size (chars) of the resulting
/// session_summary block, or 0 if there was nothing to consolidate.
pub async fn compact(base_url: &str, token: &str, agent_id: &str) -> Result<usize, ApiError> {
    let url = api::compact_url(base_url, agent_id);
    let resp = Request::post(&url)
        .header("Authorization", &api::bearer_header(token))
        .header("Content-Type", "application/json")
        .body("{}")
        .map_err(|e| ApiError::Transport {
            message: format!("{e:?}"),
        })?
        .send()
        .await
        .map_err(|e| ApiError::Transport {
            message: e.to_string(),
        })?;
    if resp.status() < 200 || resp.status() >= 300 {
        return Err(ApiError::Server {
            status: resp.status(),
        });
    }
    let txt = resp.text().await.map_err(|e| ApiError::Transport {
        message: e.to_string(),
    })?;
    let v: serde_json::Value = serde_json::from_str(&txt).map_err(|e| ApiError::Decode {
        message: e.to_string(),
    })?;
    Ok(v["session_summary_chars"].as_u64().unwrap_or(0) as usize)
}
