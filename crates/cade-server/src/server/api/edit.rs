//! `POST /v1/agents/:id/edit` — Stateless edit endpoint.
//!
//! Accepts `{prefix, selected_text, suffix, instruction, language}`, builds a prompt for the model
//! to rewrite `selected_text` according to `instruction`, considering `prefix` and `suffix` context.
//! Streams tokens back via SSE.
//!
//! Designed as the shared backend for Neovim and other editor clients wanting an interactive
//! AI edit/refactor capability.

use axum::http::StatusCode;
use axum::{
    Json,
    extract::{Path, State},
    response::{IntoResponse, Response, Sse, sse::Event},
};
use cade_ai::{CompletionRequest, LlmMessage, StreamChunk, catalogue};
use cade_store::sqlite;
use serde_json::{Value, json};

use super::messages::err;
use crate::server::state::AppState;

/// System prompt for the edit model.
const EDIT_SYSTEM: &str = "\
You are a code editor engine. \
You receive the code before the cursor (<prefix>), the code currently selected (<selected_text>), \
and the code after the cursor (<suffix>). You also receive an <instruction> from the user. \
Rewrite the <selected_text> to fulfill the <instruction>. \
Output ONLY the replacement code. Do not repeat the prefix or suffix. \
Do not add explanations, comments about your output, or markdown fences. \
Output nothing if no replacement is appropriate.";

/// Maximum output tokens for edits — edits can be longer than completions.
const EDIT_MAX_TOKENS: u32 = 4096;

/// `POST /v1/agents/:id/edit`
///
/// Body:
/// ```json
/// {
///   "prefix":        "string — code before cursor (default: empty)",
///   "selected_text": "string — code selected by the user (default: empty)",
///   "suffix":        "string — code after cursor (default: empty)",
///   "instruction":   "string — user prompt/instruction (required)",
///   "language":      "string — filetype, e.g. 'rust' (default: 'text')",
///   "model":         "string — model override (optional)",
///   "max_tokens":    4096
/// }
/// ```
pub async fn edit(
    State(state): State<AppState>,
    Path(agent_id): Path<String>,
    Json(body): Json<Value>,
) -> Response {
    // ── Parse request ────────────────────────────────────────────────────
    let prefix = body["prefix"].as_str().unwrap_or("");
    let selected_text = body["selected_text"].as_str().unwrap_or("");
    let suffix = body["suffix"].as_str().unwrap_or("");
    let instruction = body["instruction"].as_str().unwrap_or("");
    let language = body["language"].as_str().unwrap_or("text");

    if instruction.is_empty() {
        return err(StatusCode::BAD_REQUEST, "'instruction' must be provided");
    }

    // ── Resolve model ────────────────────────────────────────────────────
    let model = if let Some(m) = body["model"].as_str().filter(|s| !s.is_empty()) {
        m.to_string()
    } else {
        match sqlite::get_agent(&state.db, &agent_id) {
            Ok(Some(agent)) => agent
                .compaction_model
                .as_deref()
                .filter(|m| !m.is_empty())
                .unwrap_or(&agent.model)
                .to_string(),
            Ok(None) => {
                return err(
                    StatusCode::NOT_FOUND,
                    &format!("agent '{agent_id}' not found"),
                );
            }
            Err(e) => return err(StatusCode::INTERNAL_SERVER_ERROR, &e.to_string()),
        }
    };

    let max_tokens = body["max_tokens"]
        .as_u64()
        .map(|n| n as u32)
        .unwrap_or(EDIT_MAX_TOKENS)
        .min(catalogue::max_tokens_for_model(&model));

    // ── Build EDIT prompt ────────────────────────────────────────────────
    let user_prompt = format!(
        "# language: {language}\n\
         <instruction>\n{instruction}\n</instruction>\n\
         <prefix>\n{prefix}</prefix>\n\
         <selected_text>\n{selected_text}\n</selected_text>\n\
         <suffix>\n{suffix}</suffix>"
    );

    let req = CompletionRequest {
        model: model.clone(),
        messages: vec![
            LlmMessage {
                role: "system".into(),
                content: EDIT_SYSTEM.into(),
                tool_call_id: None,
                tool_calls: None,
                images: None, cache_control: None,
            },
            LlmMessage {
                role: "user".into(),
                content: user_prompt,
                tool_call_id: None,
                tool_calls: None,
                images: None, cache_control: None,
            },
        ],
        tools: vec![],
        max_tokens,
        reasoning_effort: None,
    };

    // ── Stream response ──────────────────────────────────────────────────
    let llm_stream = match state.llm.stream(&req).await {
        Ok(s) => s,
        Err(e) => {
            let err_msg = e.to_string();
            tracing::error!("/v1/edit stream open failed for model '{model}': {err_msg}");
            let s = futures::stream::iter([
                Ok::<Event, std::convert::Infallible>(
                    Event::default()
                        .data(json!({"message_type": "error", "error": err_msg}).to_string()),
                ),
                Ok::<Event, std::convert::Infallible>(Event::default().data("[DONE]")),
            ]);
            return Sse::new(s).into_response();
        }
    };

    let sse_stream = futures::StreamExt::map(llm_stream, |chunk: cade_ai::Result<StreamChunk>| {
        let event = match chunk {
            Ok(StreamChunk::Text(text)) => Event::default()
                .data(json!({"message_type": "stream_delta", "content": text}).to_string()),
            Ok(StreamChunk::Done) => {
                Event::default().data(json!({"message_type": "stream_end"}).to_string())
            }
            Ok(StreamChunk::Reasoning(_)) | Ok(StreamChunk::ToolCall(_)) => {
                Event::default().comment("")
            }
            Ok(StreamChunk::Usage(u)) => Event::default().data(
                json!({
                    "message_type": "usage_statistics",
                    "input_tokens": u.input_tokens,
                    "output_tokens": u.output_tokens,
                    "model": u.model,
                })
                .to_string(),
            ),
            Ok(StreamChunk::FinishReason(_)) => Event::default().comment(""),
            Err(e) => Event::default()
                .data(json!({"message_type": "error", "error": e.to_string()}).to_string()),
        };
        Ok::<Event, std::convert::Infallible>(event)
    });

    let meta = futures::stream::once(async move {
        Ok::<Event, std::convert::Infallible>(
            Event::default()
                .data(json!({"message_type": "stream_start", "model": model}).to_string()),
        )
    });

    Sse::new(futures::StreamExt::chain(meta, sse_stream)).into_response()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn edit_system_prompt_is_concise() {
        assert!(
            EDIT_SYSTEM.len() < 500,
            "EDIT system prompt is {} chars — keep it under 500",
            EDIT_SYSTEM.len()
        );
    }

    #[test]
    fn edit_prompt_format_includes_all_parts() {
        let language = "rust";
        let prefix = "fn main() {\n    ";
        let selected_text = "println!(\"hello\");";
        let suffix = "\n}\n";
        let instruction = "Change to hello world";

        let prompt = format!(
            "# language: {language}\n\
             <instruction>\n{instruction}\n</instruction>\n\
             <prefix>\n{prefix}</prefix>\n\
             <selected_text>\n{selected_text}\n</selected_text>\n\
             <suffix>\n{suffix}</suffix>"
        );

        assert!(prompt.contains("# language: rust"));
        assert!(prompt.contains("<instruction>\nChange to hello world"));
        assert!(prompt.contains("<prefix>\nfn main()"));
        assert!(prompt.contains("<selected_text>\nprintln!(\"hello\");"));
        assert!(prompt.contains("<suffix>\n\n}\n"));
    }

    fn make_test_state() -> AppState {
        let db = cade_store::sqlite::open(":memory:").unwrap();
        let config = std::sync::Arc::new(crate::server::config::ServerConfig {
            max_tokens_per_turn: Some(64_000),
            addr: "127.0.0.1:0".parse().unwrap(),
            db_path: ":memory:".into(),
            llm_provider: crate::server::config::LlmProviderKind::Anthropic,
            default_model: "test".into(),
            anthropic_api_key: None,
            openai_api_key: None,
            google_api_key: None,
            ollama_base_url: String::new(),
            api_key: None,

            allowed_origin: None,
            max_context_budget: None,
        });
        AppState {
            subagent_cancellations: std::sync::Arc::new(tokio::sync::RwLock::new(
                std::collections::HashMap::new(),
            )),
            db,
            llm: std::sync::Arc::new(cade_ai::LlmRouter::build(&cade_ai::AiConfig {
                anthropic_api_key: None,
                openai_api_key: None,
                google_api_key: None,
                ollama_base_url: String::new(),
                llm_provider: String::new(),
            })),
            llm_router: std::sync::Arc::new(tokio::sync::RwLock::new(cade_ai::LlmRouter::build(
                &cade_ai::AiConfig {
                    anthropic_api_key: None,
                    openai_api_key: None,
                    google_api_key: None,
                    ollama_base_url: String::new(),
                    llm_provider: String::new(),
                },
            ))),
            config,
            mcp: std::sync::Arc::new(crate::server::state::McpManager::empty()),
            rate_limiter: crate::server::rate_limit::RateLimiter::from_env(),
            memory_cache: std::sync::Arc::new(parking_lot::Mutex::new(
                std::collections::HashMap::new(),
            )),
            agent_activity: std::sync::Arc::new(tokio::sync::RwLock::new(
                std::collections::HashMap::new(),
            )),
            agent_metrics: std::sync::Arc::new(dashmap::DashMap::new()),
            agent_context_telemetry: std::sync::Arc::new(tokio::sync::RwLock::new(
                std::collections::HashMap::new(),
            )),
            context_cache: std::sync::Arc::new(parking_lot::Mutex::new(lru::LruCache::new(
                crate::server::state::CONTEXT_CACHE_CAPACITY,
            ))),
            all_skills: std::sync::Arc::new(tokio::sync::RwLock::new(Vec::new())),
            agent_skills: std::sync::Arc::new(tokio::sync::RwLock::new(
                std::collections::HashMap::new(),
            )),
            pending_subagent_results: std::sync::Arc::new(tokio::sync::RwLock::new(
                std::collections::HashMap::new(),
            )),
            subagent_semaphore: std::sync::Arc::new(tokio::sync::Semaphore::new(4)),
            embedder: None,
        }
    }

    #[tokio::test]
    async fn test_edit_endpoint_returns_sse_stream() {
        let state = make_test_state();
        let agent_id = "test-agent".to_string();

        cade_store::sqlite::create_agent(
            &state.db,
            &cade_store::sqlite::AgentRow {
                id: agent_id.clone(),
                name: "Test Agent".into(),
                model: "test-model".into(),
                compaction_model: None,
                system_prompt: Some("sys".into()),
                description: Some("desc".into()),
                theme: None,
                active_plan_json: None,
                created_at: Some(chrono::Utc::now().timestamp()),
                parent_id: None,
            },
        )
        .unwrap();

        let body = json!({
            "prefix": "fn ",
            "selected_text": "foo",
            "suffix": "() {}",
            "instruction": "rename to bar",
            "language": "rust"
        });

        let response = edit(
            axum::extract::State(state),
            axum::extract::Path(agent_id),
            axum::Json(body),
        )
        .await;

        let status = response.status();
        assert_eq!(status, StatusCode::OK, "Expected 200 OK from edit endpoint");

        let content_type = response
            .headers()
            .get("content-type")
            .unwrap()
            .to_str()
            .unwrap();
        assert!(content_type.starts_with("text/event-stream"));
    }
}
