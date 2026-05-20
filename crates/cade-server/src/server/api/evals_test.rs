use crate::server::api::evals::list_eval_tasks;
use crate::server::state::AppState;
use axum::Json;
use std::sync::Arc;
use tokio::sync::RwLock;

/// Verify that `list_eval_tasks` returns an empty array (not an error) when
/// the DB is freshly initialised and contains no eval tasks.
///
/// This replaces the former `test_db_lock_poisoning_yields_500` test, which
/// was testing `std::sync::Mutex` poison semantics that no longer apply now
/// that `Db` uses `parking_lot::Mutex` (which never poisons).
#[tokio::test]
async fn list_eval_tasks_returns_empty_on_fresh_db() {
    let db = cade_store::sqlite::open(":memory:").unwrap();
    let config = Arc::new(crate::server::config::ServerConfig { max_tokens_per_turn: 64_000,
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

    let state = AppState {
        subagent_cancellations: std::sync::Arc::new(tokio::sync::RwLock::new(
            std::collections::HashMap::new(),
        )),
        db,
        llm: Arc::new(cade_ai::LlmRouter::build(&cade_ai::AiConfig {
            anthropic_api_key: None,
            openai_api_key: None,
            google_api_key: None,
            ollama_base_url: String::new(),
            llm_provider: String::new(),
        })),
        llm_router: Arc::new(RwLock::new(cade_ai::LlmRouter::build(&cade_ai::AiConfig {
            anthropic_api_key: None,
            openai_api_key: None,
            google_api_key: None,
            ollama_base_url: String::new(),
            llm_provider: String::new(),
        }))),
        config,
        mcp: Arc::new(crate::server::state::McpManager::empty()),
        rate_limiter: crate::server::rate_limit::RateLimiter::from_env(),
        memory_cache: Arc::new(parking_lot::Mutex::new(std::collections::HashMap::new())),
        agent_activity: Arc::new(RwLock::new(std::collections::HashMap::new())),
        agent_metrics: Arc::new(RwLock::new(std::collections::HashMap::new())),
        agent_context_telemetry: Arc::new(RwLock::new(std::collections::HashMap::new())),
        context_cache: Arc::new(parking_lot::Mutex::new(lru::LruCache::new(
            crate::server::state::CONTEXT_CACHE_CAPACITY,
        ))),
        all_skills: Arc::new(RwLock::new(Vec::new())),
        agent_skills: Arc::new(RwLock::new(std::collections::HashMap::new())),
        pending_subagent_results: Arc::new(RwLock::new(std::collections::HashMap::new())),
        subagent_semaphore: Arc::new(tokio::sync::Semaphore::new(4)),
        embedder: None,
    };

    let result = list_eval_tasks(axum::extract::State(state)).await;
    let Json(body) = result.expect("list_eval_tasks must succeed on a fresh DB");
    let tasks = body.as_array().expect("response body must be a JSON array");
    assert!(tasks.is_empty(), "fresh DB must return zero eval tasks");
}
