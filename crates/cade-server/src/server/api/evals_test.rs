use crate::server::api::evals::list_eval_tasks;
use crate::server::state::AppState;
use axum::http::StatusCode;
use std::sync::{Arc, Mutex};
use tokio::sync::RwLock;

#[tokio::test]
async fn test_db_lock_poisoning_yields_500() {
    // 1. Create a memory DB and poison it
    let conn = rusqlite::Connection::open_in_memory().unwrap();
    let db = Arc::new(Mutex::new(conn));

    // Force poison the DB lock
    let db_clone = db.clone();
    let _ = std::thread::spawn(move || {
        let _guard = db_clone.lock().unwrap();
        panic!("Intentionally poisoning the lock");
    })
    .join();

    assert!(db.lock().is_err(), "Lock should be poisoned");

    // 2. Set up AppState
    let _reqwest_client = reqwest::Client::new();
    let empty_path = std::path::PathBuf::new();
    let _settings = Arc::new(RwLock::new(
        cade_core::settings::SettingsManager::new(&empty_path).unwrap(),
    ));
    let config = Arc::new(crate::server::config::ServerConfig {
        addr: "127.0.0.1:0".parse().unwrap(),
        db_path: ":memory:".into(),
        llm_provider: crate::server::config::LlmProviderKind::Anthropic,
        default_model: "test".into(),
        anthropic_api_key: None,
        openai_api_key: None,
        google_api_key: None,
        ollama_base_url: String::new(),
        api_key: None,
    });

    let state = AppState {
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
        rate_limiter: crate::server::rate_limit::RateLimiter::from_env(),
        memory_cache: Arc::new(Mutex::new(std::collections::HashMap::new())),
        agent_activity: Arc::new(RwLock::new(std::collections::HashMap::new())),
    };

    // 3. Call the handler directly
    let result = list_eval_tasks(axum::extract::State(state)).await;

    // 4. Verify the poisoned lock resulted in a 500 error gracefully
    match result {
        Ok(_) => panic!("Expected error due to poisoned lock, but got Ok"),
        Err((status, body)) => {
            assert_eq!(status, StatusCode::INTERNAL_SERVER_ERROR);
            let body_str = serde_json::to_string(&body.0).unwrap_or_default();
            assert!(
                body_str.contains("db lock poisoned"),
                "Error response should indicate poisoned lock"
            );
        }
    }
}
