//! Phase 3: `/compact` slash command handler.
//!
//! Manually triggers the same `consolidate_agent` flow that the Sleeptime
//! background task runs after 20 s of agent inactivity (or that the P1-3
//! recovery loop runs synchronously on a context-overflow error).  Lets
//! the user proactively roll dropped turns into the pinned
//! `session_summary` block before issuing a large request.

use axum::{
    Json,
    extract::{Path, Query, State},
    http::StatusCode,
};
use serde_json::{Value, json};
use std::collections::HashMap;

use crate::server::state::AppState;

/// `POST /v1/agents/:id/compact?conversation_id=<id>`
///
/// Synchronously triggers `consolidate_agent` for the agent (and, when
/// supplied, the specific conversation).  Returns a JSON envelope
/// describing whether a `session_summary` block now exists and how large
/// it is — clients can show this in a toast.
pub async fn compact_handler(
    State(state): State<AppState>,
    Path(agent_id): Path<String>,
    Query(params): Query<HashMap<String, String>>,
) -> Result<Json<Value>, (StatusCode, Json<Value>)> {
    let conversation_id = params.get("conversation_id").map(String::as_str);

    let compacted_chars =
        crate::server::consolidation::consolidate_agent(state.clone(), agent_id.clone(), conversation_id.map(String::from), None)
            .await;

    Ok(Json(json!({
        "agent_id":             agent_id,
        "conversation_id":      conversation_id,
        "session_summary_chars": compacted_chars.unwrap_or(0),
        "ok":                   true,
    })))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn build_state(db: cade_store::sqlite::Db) -> AppState {
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
                unreachable!("stream() is not exercised by this mock")
            }
        }

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
    async fn compact_handler_returns_ok_for_empty_agent() {
        let db = cade_store::sqlite::open(":memory:").unwrap();
        let agent_id = "compact_agent_empty";
        cade_store::sqlite::create_agent(
            &db,
            &cade_store::sqlite::AgentRow {
                id: agent_id.to_string(),
                name: "A".to_string(),
                model: "anthropic/claude-sonnet-4-5-20250929".to_string(),
                description: None,
                system_prompt: None,
                created_at: None,
                compaction_model: None,
                theme: None,
                active_plan_json: None,
                parent_id: None,
            },
        )
        .unwrap();

        let state = build_state(db);
        let res = compact_handler(
            State(state),
            Path(agent_id.to_string()),
            Query(HashMap::new()),
        )
        .await
        .expect("compact_handler must succeed");

        let body: Value = res.0;
        assert_eq!(body["agent_id"], agent_id);
        assert_eq!(body["ok"], true);
        assert!(body["session_summary_chars"].is_number());
    }
}
