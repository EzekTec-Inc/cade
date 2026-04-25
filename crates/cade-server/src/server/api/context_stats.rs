//! Phase 4: per-agent context-window telemetry endpoint.
//!
//! `GET /v1/agents/:id/context_stats` returns the most-recent
//! `ContextTelemetry` recorded by `build_context`.  Useful for the GUI's
//! /context overlay and for proving every defence layer is firing.
//!
//! Returns 404 if no telemetry has been captured yet (i.e. the agent has
//! not yet had a `build_context` call complete).

use axum::{
    Json,
    extract::{Path, State},
    http::StatusCode,
};
use serde_json::{Value, json};

use crate::server::state::AppState;

pub async fn get_context_stats(
    State(state): State<AppState>,
    Path(agent_id): Path<String>,
) -> Result<Json<Value>, (StatusCode, Json<Value>)> {
    let map = state.agent_context_telemetry.read().await;
    match map.get(&agent_id) {
        Some(t) => Ok(Json(json!(t))),
        None => Err((
            StatusCode::NOT_FOUND,
            Json(json!({ "error": "no telemetry yet for this agent" })),
        )),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::server::state::ContextTelemetry;

    fn dummy_state() -> AppState {
        let db = cade_store::sqlite::open(":memory:").unwrap();
        struct NoopLlm;
        #[async_trait::async_trait]
        impl cade_ai::LlmProvider for NoopLlm {
            async fn complete(
                &self,
                _r: &cade_ai::CompletionRequest,
            ) -> cade_ai::Result<cade_ai::CompletionResponse> {
                Ok(cade_ai::CompletionResponse {
                    content: Some(String::new()),
                    tool_calls: vec![],
                    finish_reason: "stop".into(),
                })
            }
            async fn stream(
                &self,
                _r: &cade_ai::CompletionRequest,
            ) -> cade_ai::Result<
                std::pin::Pin<
                    Box<
                        dyn tokio_stream::Stream<Item = cade_ai::Result<cade_ai::StreamChunk>>
                            + Send,
                    >,
                >,
            > {
                unimplemented!()
            }
        }
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
            llm: std::sync::Arc::new(NoopLlm),
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
            memory_cache: std::sync::Arc::new(std::sync::Mutex::new(
                std::collections::HashMap::new(),
            )),
            agent_activity: std::sync::Arc::new(tokio::sync::RwLock::new(
                std::collections::HashMap::new(),
            )),
            agent_metrics: std::sync::Arc::new(tokio::sync::RwLock::new(
                std::collections::HashMap::new(),
            )),
            agent_context_telemetry: std::sync::Arc::new(tokio::sync::RwLock::new(
                std::collections::HashMap::new(),
            )),
            context_cache: std::sync::Arc::new(std::sync::Mutex::new(lru::LruCache::new(
                std::num::NonZeroUsize::new(20).unwrap(),
            ))),
            all_skills: std::sync::Arc::new(tokio::sync::RwLock::new(Vec::new())),
            agent_skills: std::sync::Arc::new(tokio::sync::RwLock::new(
                std::collections::HashMap::new(),
            )),
            pending_subagent_results: std::sync::Arc::new(tokio::sync::RwLock::new(
                std::collections::HashMap::new(),
            )),
            subagent_semaphore: std::sync::Arc::new(tokio::sync::Semaphore::new(4)),
        }
    }

    #[tokio::test]
    async fn returns_404_when_no_telemetry_recorded_yet() {
        let state = dummy_state();
        let res = get_context_stats(State(state), Path("missing".into())).await;
        assert!(res.is_err(), "must 404 when no telemetry exists");
        let (code, body) = res.err().unwrap();
        assert_eq!(code, StatusCode::NOT_FOUND);
        assert!(body.0["error"].is_string());
    }

    #[tokio::test]
    async fn returns_recorded_telemetry_when_present() {
        let state = dummy_state();
        let agent_id = "agent_with_telemetry";
        let t = ContextTelemetry {
            model: "anthropic/claude-sonnet-4-5".into(),
            window_tokens: 200_000,
            input_budget_chars: 510_000,
            system_overhead_chars: 12_345,
            system_tokens: 4_115,
            message_budget_chars: 497_655,
            history_chars: 12_000,
            turns_selected: 3,
            turns_omitted: 0,
            system_msg_count: 2,
            skills_full: 1,
            skills_summary: 0,
            fits_budget: true,
            build_micros: 950,
        };
        state
            .agent_context_telemetry
            .write()
            .await
            .insert(agent_id.into(), t);

        let res = get_context_stats(State(state), Path(agent_id.into()))
            .await
            .expect("must succeed");
        let body = res.0;
        assert_eq!(body["model"], "anthropic/claude-sonnet-4-5");
        assert_eq!(body["window_tokens"], 200_000);
        assert_eq!(body["fits_budget"], true);
        assert_eq!(body["turns_selected"], 3);
        assert_eq!(body["skills_full"], 1);
    }
}
