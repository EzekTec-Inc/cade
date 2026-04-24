//! `POST /v1/complete` — Stateless FIM code-completion endpoint.
//!
//! Accepts `{prefix, suffix, language}`, builds a minimal FIM prompt, streams
//! tokens back via SSE.  No DB interaction, no conversation state, no
//! consolidation — just a thin wrapper around the `LlmRouter`.
//!
//! Designed as the shared backend for both Neovim and VS Code inline
//! completion clients.

use axum::{
    extract::{Path, State},
    response::{sse::Event, IntoResponse, Response, Sse},
    Json,
};
use cade_ai::{catalogue, CompletionRequest, LlmMessage, StreamChunk};
use cade_store::sqlite;
use axum::http::StatusCode;
use serde_json::{json, Value};

use super::messages::err;
use crate::server::state::AppState;

/// System prompt for the FIM completion model.
const FIM_SYSTEM: &str = "\
You are a code completion engine. \
You receive the code before the cursor (<prefix>) and after the cursor (<suffix>). \
Output ONLY the code that should be inserted at the cursor position. \
Do not repeat any of the prefix or suffix. \
Do not add explanations, comments about your output, or markdown fences. \
Output nothing if no completion is appropriate.";

/// Maximum output tokens for completions — completions should be short.
const COMPLETE_MAX_TOKENS: u32 = 512;

/// `POST /v1/agents/:id/complete`
///
/// Body:
/// ```json
/// {
///   "prefix":   "string — code before cursor (required)",
///   "suffix":   "string — code after cursor (default: empty)",
///   "language":  "string — filetype, e.g. 'rust' (default: 'text')",
///   "model":     "string — model override (optional, falls back to agent's compaction_model → default model)",
///   "max_tokens": 512
/// }
/// ```
///
/// Returns an SSE stream identical in shape to `/v1/agents/:id/messages/stream`:
/// ```text
/// data: {"message_type":"stream_delta","content":"fn "}
/// data: {"message_type":"stream_delta","content":"main()"}
/// data: {"message_type":"stream_end"}
/// data: [DONE]
/// ```
pub async fn complete(
    State(state): State<AppState>,
    Path(agent_id): Path<String>,
    Json(body): Json<Value>,
) -> Response {
    // ── Parse request ────────────────────────────────────────────────────
    let prefix = body["prefix"].as_str().unwrap_or("");
    let suffix = body["suffix"].as_str().unwrap_or("");
    let language = body["language"].as_str().unwrap_or("text");

    if prefix.is_empty() && suffix.is_empty() {
        return err(StatusCode::BAD_REQUEST, "at least one of 'prefix' or 'suffix' must be non-empty");
    }

    // ── Resolve model ────────────────────────────────────────────────────
    // Priority: body.model → agent.compaction_model → agent.model
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
            Ok(None) => return err(StatusCode::NOT_FOUND, &format!("agent '{agent_id}' not found")),
            Err(e) => return err(StatusCode::INTERNAL_SERVER_ERROR, &e.to_string()),
        }
    };

    let max_tokens = body["max_tokens"]
        .as_u64()
        .map(|n| n as u32)
        .unwrap_or(COMPLETE_MAX_TOKENS)
        .min(catalogue::max_tokens_for_model(&model));

    // ── Build FIM prompt ─────────────────────────────────────────────────
    let user_prompt = format!(
        "# language: {language}\n\
         <prefix>\n\
         {prefix}\
         <suffix>\n\
         {suffix}\
         </suffix>"
    );

    let req = CompletionRequest {
        model: model.clone(),
        messages: vec![
            LlmMessage {
                role: "system".into(),
                content: FIM_SYSTEM.into(),
                tool_call_id: None,
                tool_calls: None,
                images: None,
            },
            LlmMessage {
                role: "user".into(),
                content: user_prompt,
                tool_call_id: None,
                tool_calls: None,
                images: None,
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
            tracing::error!("/v1/complete stream open failed for model '{model}': {err_msg}");
            let s = futures::stream::iter([
                Ok::<Event, std::convert::Infallible>(
                    Event::default().data(json!({"error": err_msg}).to_string()),
                ),
                Ok::<Event, std::convert::Infallible>(Event::default().data("[DONE]")),
            ]);
            return Sse::new(s).into_response();
        }
    };

    let sse_stream =
        futures::StreamExt::map(llm_stream, |chunk: cade_ai::Result<StreamChunk>| {
            let event = match chunk {
                Ok(StreamChunk::Text(text)) => {
                    Event::default()
                        .data(json!({"message_type": "stream_delta", "content": text}).to_string())
                }
                Ok(StreamChunk::Done) => {
                    Event::default()
                        .data(json!({"message_type": "stream_end"}).to_string())
                }
                // Completions don't use tool calls or reasoning — ignore gracefully.
                Ok(StreamChunk::Reasoning(_)) | Ok(StreamChunk::ToolCall(_)) => {
                    Event::default().comment("")
                }
                Ok(StreamChunk::Usage(u)) => {
                    Event::default().data(
                        json!({
                            "message_type": "usage_statistics",
                            "input_tokens": u.input_tokens,
                            "output_tokens": u.output_tokens,
                            "model": u.model,
                        })
                        .to_string(),
                    )
                }
                Ok(StreamChunk::FinishReason(_)) => Event::default().comment(""),
                Err(e) => {
                    Event::default().data(json!({"error": e.to_string()}).to_string())
                }
            };
            Ok::<Event, std::convert::Infallible>(event)
        });

    // Prepend a start event with the resolved model so clients can display it.
    let meta = futures::stream::once(async move {
        Ok::<Event, std::convert::Infallible>(
            Event::default().data(
                json!({"message_type": "stream_start", "model": model}).to_string(),
            ),
        )
    });

    Sse::new(futures::StreamExt::chain(meta, sse_stream)).into_response()
}

// ── Tests ────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fim_system_prompt_is_concise() {
        // The system prompt should be under 500 chars to minimise input tokens
        assert!(
            FIM_SYSTEM.len() < 500,
            "FIM system prompt is {} chars — keep it under 500",
            FIM_SYSTEM.len()
        );
    }

    #[test]
    fn complete_max_tokens_is_reasonable() {
        #[allow(clippy::assertions_on_constants)]
        {
            assert!(COMPLETE_MAX_TOKENS <= 1024);
            assert!(COMPLETE_MAX_TOKENS >= 128);
        }
    }

    #[test]
    fn fim_prompt_format_includes_language() {
        let language = "rust";
        let prefix = "fn main() {\n    ";
        let suffix = "\n}\n";
        let prompt = format!(
            "# language: {language}\n<prefix>\n{prefix}<suffix>\n{suffix}</suffix>"
        );
        assert!(prompt.contains("# language: rust"));
        assert!(prompt.contains("<prefix>"));
        assert!(prompt.contains("fn main()"));
        assert!(prompt.contains("<suffix>"));
    }

    fn make_test_state() -> AppState {
        let db = cade_store::sqlite::open(":memory:").unwrap();
        let config = std::sync::Arc::new(crate::server::config::ServerConfig {
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
            db,
            llm: std::sync::Arc::new(cade_ai::LlmRouter::build(&cade_ai::AiConfig {
                anthropic_api_key: None,
                openai_api_key: None,
                google_api_key: None,
                ollama_base_url: String::new(),
                llm_provider: String::new(),
            })),
            llm_router: std::sync::Arc::new(tokio::sync::RwLock::new(
                cade_ai::LlmRouter::build(&cade_ai::AiConfig {
                    anthropic_api_key: None,
                    openai_api_key: None,
                    google_api_key: None,
                    ollama_base_url: String::new(),
                    llm_provider: String::new(),
                }),
            )),
            config,
            mcp: std::sync::Arc::new(crate::server::state::McpManager::empty()),
            rate_limiter: crate::server::rate_limit::RateLimiter::from_env(),
            memory_cache: std::sync::Arc::new(std::sync::Mutex::new(
                std::collections::HashMap::new(),
            )),
            agent_activity: std::sync::Arc::new(tokio::sync::RwLock::new(
                std::collections::HashMap::new(),
            )),
            agent_metrics: std::sync::Arc::new(tokio::sync::RwLock::new(
                std::collections::HashMap::new(),
            )),
            context_cache: std::sync::Arc::new(std::sync::Mutex::new(lru::LruCache::new(std::num::NonZeroUsize::new(20).unwrap()))),
            all_skills: std::sync::Arc::new(tokio::sync::RwLock::new(Vec::new())),
            agent_skills: std::sync::Arc::new(tokio::sync::RwLock::new(std::collections::HashMap::new())),
            pending_subagent_results: std::sync::Arc::new(tokio::sync::RwLock::new(std::collections::HashMap::new())),
            subagent_semaphore: std::sync::Arc::new(tokio::sync::Semaphore::new(4)),
        }
    }

    #[tokio::test]
    async fn rejects_empty_prefix_and_suffix() {
        let state = make_test_state();
        let res = complete(
            State(state),
            Path("agent1".into()),
            Json(json!({})),
        )
        .await;
        let (parts, body) = res.into_parts();
        let bytes = axum::body::to_bytes(body, usize::MAX).await.unwrap();
        assert_eq!(parts.status, StatusCode::BAD_REQUEST);
        let text = String::from_utf8_lossy(&bytes);
        assert!(text.contains("prefix"), "error should mention prefix: {text}");
    }

    #[tokio::test]
    async fn returns_not_found_for_missing_agent() {
        let state = make_test_state();
        let res = complete(
            State(state),
            Path("nonexistent".into()),
            Json(json!({"prefix": "fn "})),
        )
        .await;
        let (parts, body) = res.into_parts();
        let bytes = axum::body::to_bytes(body, usize::MAX).await.unwrap();
        assert_eq!(parts.status, StatusCode::NOT_FOUND);
        let text = String::from_utf8_lossy(&bytes);
        assert!(text.contains("nonexistent"), "error should name the agent: {text}");
    }

    #[tokio::test]
    async fn with_model_override_bypasses_agent_lookup() {
        // When body.model is specified, the handler skips DB lookup entirely,
        // so a missing agent should NOT produce a 404.
        let state = make_test_state();
        let res = complete(
            State(state),
            Path("doesnt_exist".into()),
            Json(json!({
                "prefix": "fn main() {",
                "suffix": "}",
                "language": "rust",
                "model": "test-model"
            })),
        )
        .await;
        let (parts, _body) = res.into_parts();
        // Should return 200 (SSE) — the LLM call will fail but that's an SSE error event,
        // not a 404.
        assert_eq!(parts.status, StatusCode::OK, "model override should skip agent lookup");
    }

    #[tokio::test]
    async fn no_messages_persisted_to_db() {
        let state = make_test_state();

        // Create the agent so model resolution works
        cade_store::sqlite::create_agent(
            &state.db,
            &cade_store::sqlite::AgentRow {
                id: "a1".into(),
                name: "test".into(),
                model: "test-model".into(),
                description: None,
                system_prompt: None,
                created_at: None,
                compaction_model: None,
                theme: None,
            },
        )
        .unwrap();

        // Make a completion request
        let _ = complete(
            State(state.clone()),
            Path("a1".into()),
            Json(json!({"prefix": "fn ", "language": "rust"})),
        )
        .await;

        // Verify zero messages were persisted
        let count: i64 = state
            .db
            .lock()
            .unwrap()
            .query_row("SELECT COUNT(*) FROM messages", [], |r| r.get(0))
            .unwrap();
        assert_eq!(count, 0, "completion endpoint must not persist any messages");
    }

    #[tokio::test]
    async fn no_consolidation_triggered() {
        let state = make_test_state();

        cade_store::sqlite::create_agent(
            &state.db,
            &cade_store::sqlite::AgentRow {
                id: "a2".into(),
                name: "test".into(),
                model: "test-model".into(),
                description: None,
                system_prompt: None,
                created_at: None,
                compaction_model: None,
                theme: None,
            },
        )
        .unwrap();

        let _ = complete(
            State(state.clone()),
            Path("a2".into()),
            Json(json!({"prefix": "let x = "})),
        )
        .await;

        let activity = state.agent_activity.read().await;
        assert!(
            activity.get("a2").is_none(),
            "completion endpoint must not touch agent_activity"
        );
    }
}
