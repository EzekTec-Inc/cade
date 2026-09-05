//! MCP management and execution API handlers.
//!
//! - `GET /v1/mcp` — list all MCP servers and their exposed tools.
//! - `POST /v1/mcp/reload` — reload MCP servers and return reload summary.
//! - `POST /v1/mcp/call` — execute an MCP tool via the server's McpManager.

use axum::{Json, extract::State, http::StatusCode};
use cade_core::settings::McpServerConfig;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::collections::HashMap;

use crate::server::state::AppState;

/// Request payload for `POST /v1/mcp/reload`.
#[derive(Debug, Deserialize, Default)]
pub struct ReloadMcpRequest {
    #[serde(default)]
    pub configs: Option<HashMap<String, McpServerConfig>>,
}

/// Request payload for `POST /v1/mcp/call`.
#[derive(Debug, Deserialize)]
pub struct CallMcpToolRequest {
    pub name: String,
    #[serde(default)]
    pub arguments: Value,
}

/// Response payload for `POST /v1/mcp/call`.
#[derive(Debug, Serialize, Deserialize)]
pub struct CallMcpToolResponse {
    pub output: String,
    pub is_error: bool,
    pub ui_resource_uri: Option<String>,
}

/// `GET /v1/mcp`
///
/// Returns every MCP server currently loaded by the server, with its connection
/// command, tool list, and enabled/disabled status.
///
/// ```json
/// {
///   "servers": [
///     {
///       "key": "desktop-commander",
///       "command": "npx @desktop-commander/mcp-server",
///       "tools": ["bash", "read_file", "write_file", ...],
///       "disabled": false
///     }
///   ]
/// }
/// ```
pub async fn list_mcp_servers(State(state): State<AppState>) -> Json<Value> {
    let servers = state.mcp.status().await;
    Json(json!({ "servers": servers }))
}

/// `POST /v1/mcp/reload`
///
/// Reloads MCP servers in `AppState::mcp`. If `configs` is provided in the body,
/// it reloads with those configs. Otherwise, it resolves settings from disk.
pub async fn reload_mcp_servers(
    State(state): State<AppState>,
    body: Option<Json<ReloadMcpRequest>>,
) -> Result<Json<Value>, (StatusCode, Json<Value>)> {
    let configs = match body.and_then(|Json(b)| b.configs) {
        Some(c) => c,
        None => {
            let cwd = std::env::current_dir().unwrap_or_else(|_| std::path::PathBuf::from("."));
            match cade_core::settings::SettingsManager::new(&cwd) {
                Ok(mgr) => mgr.merged_mcp_servers(),
                Err(e) => {
                    tracing::warn!("Failed to load settings from disk for MCP reload: {e}");
                    HashMap::new()
                }
            }
        }
    };

    let summary = state.mcp.reload(&configs, None).await;
    Ok(Json(json!({ "summary": summary })))
}

/// `POST /v1/mcp/call`
///
/// Executes an MCP tool through the server-hosted `McpManager`.
pub async fn call_mcp_tool(
    State(state): State<AppState>,
    Json(body): Json<CallMcpToolRequest>,
) -> Result<Json<CallMcpToolResponse>, (StatusCode, Json<Value>)> {
    match state.mcp.call_tool(&body.name, &body.arguments).await {
        Some(Ok((output, is_error, ui_resource_uri))) => Ok(Json(CallMcpToolResponse {
            output,
            is_error,
            ui_resource_uri,
        })),
        Some(Err(e)) => {
            let msg = e.to_string();
            let output = if msg.starts_with("Mcp error:") || msg.starts_with("MCP error:") {
                msg
            } else {
                format!("MCP error: {msg}")
            };
            Ok(Json(CallMcpToolResponse {
                output,
                is_error: true,
                ui_resource_uri: None,
            }))
        }
        None => Err((
            StatusCode::NOT_FOUND,
            Json(json!({
                "error": format!("Tool '{}' not found on any active MCP server", body.name)
            })),
        )),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::server::api::router;
    use axum::body::Body;
    use axum::http::{Method, Request, StatusCode};
    use std::sync::Arc;
    use tokio::sync::RwLock;
    use tower::ServiceExt;

    fn test_state() -> AppState {
        let db = cade_store::sqlite::open(":memory:").unwrap();
        let config = Arc::new(crate::server::config::ServerConfig {
            max_tokens_per_turn: Some(64_000),
            addr: "127.0.0.1:0".parse().unwrap(),
            db_path: ":memory:".into(),
            llm_provider: crate::server::config::LlmProviderKind::Anthropic,
            default_model: "test".into(),
            anthropic_api_key: None,
            openai_api_key: None,
            google_api_key: None,
            ollama_base_url: String::new(),
            api_key: Some("test_tok".to_string()),
            allowed_origin: None,
            max_context_budget: None,
        });

        AppState {
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
            agent_metrics: Arc::new(dashmap::DashMap::new()),
            agent_context_telemetry: Arc::new(RwLock::new(std::collections::HashMap::new())),
            context_cache: Arc::new(parking_lot::Mutex::new(
                crate::server::state::SafeLruCache::new(
                    crate::server::state::CONTEXT_CACHE_CAPACITY,
                ),
            )),
            all_skills: Arc::new(RwLock::new(Vec::new())),
            agent_skills: Arc::new(RwLock::new(std::collections::HashMap::new())),
            pending_subagent_results: Arc::new(RwLock::new(std::collections::HashMap::new())),
            subagent_cancellations: Arc::new(RwLock::new(std::collections::HashMap::new())),
            subagent_semaphore: Arc::new(tokio::sync::Semaphore::new(4)),
            embedder: None,
        }
    }

    #[tokio::test]
    async fn test_list_mcp_servers_endpoint() {
        let state = test_state();
        let app = router(state);

        let req = Request::builder()
            .method(Method::GET)
            .uri("/v1/mcp")
            .header("Authorization", "Bearer test_tok")
            .body(Body::empty())
            .unwrap();

        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let body_bytes = axum::body::to_bytes(resp.into_body(), usize::MAX)
            .await
            .unwrap();
        let json_val: Value = serde_json::from_slice(&body_bytes).unwrap();
        assert!(json_val.get("servers").unwrap().is_array());
    }

    #[tokio::test]
    async fn test_reload_mcp_servers_endpoint() {
        let state = test_state();
        let app = router(state);

        let req = Request::builder()
            .method(Method::POST)
            .uri("/v1/mcp/reload")
            .header("Authorization", "Bearer test_tok")
            .header("Content-Type", "application/json")
            .body(Body::from(r#"{"configs":{}}"#))
            .unwrap();

        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let body_bytes = axum::body::to_bytes(resp.into_body(), usize::MAX)
            .await
            .unwrap();
        let json_val: Value = serde_json::from_slice(&body_bytes).unwrap();
        assert!(json_val.get("summary").is_some());
    }

    #[tokio::test]
    async fn test_call_mcp_tool_not_found() {
        let state = test_state();
        let app = router(state);

        let req = Request::builder()
            .method(Method::POST)
            .uri("/v1/mcp/call")
            .header("Authorization", "Bearer test_tok")
            .header("Content-Type", "application/json")
            .body(Body::from(r#"{"name":"missing__tool","arguments":{}}"#))
            .unwrap();

        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::NOT_FOUND);
    }
}
