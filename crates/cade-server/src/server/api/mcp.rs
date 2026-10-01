//! MCP management and execution API handlers.
//!
//! - `GET /v1/mcp` — list all MCP servers and their exposed tools.
//! - `POST /v1/mcp/reload` — reload MCP servers and return reload summary.
//! - `POST /v1/mcp/call` — execute an MCP tool via the server's McpManager.

mod catalog;
pub(crate) use catalog::{execution_catalog, is_mcp_tool};
pub use catalog::{spawn_catalog_sync, sync_mcp_catalog};

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
    #[serde(default)]
    pub workspace_dir: Option<String>,
    #[serde(default)]
    pub generation: Option<String>,
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
    sync_mcp_catalog(&state.db, &state.mcp)
        .await
        .map_err(|error| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(json!({"error": format!("MCP catalog synchronization failed: {error}")})),
            )
        })?;
    Ok(Json(json!({ "summary": summary })))
}

/// `POST /v1/mcp/call`
///
/// Executes an MCP tool through the server-hosted `McpManager`.
pub async fn call_mcp_tool(
    State(state): State<AppState>,
    Json(body): Json<CallMcpToolRequest>,
) -> Result<Json<CallMcpToolResponse>, (StatusCode, Json<Value>)> {
    if body.name.trim().is_empty() {
        return Err((
            StatusCode::BAD_REQUEST,
            Json(json!({
                "error": "Tool name cannot be empty"
            })),
        ));
    }

    let cwd = match &body.workspace_dir {
        Some(dir) if !dir.trim().is_empty() => std::path::PathBuf::from(dir),
        _ => std::env::current_dir().unwrap_or_else(|_| std::path::PathBuf::from(".")),
    };
    let normalized_args =
        cade_agent::tools::normalize_mcp_arguments(&body.name, &body.arguments, &cwd);

    let result = match body.generation.as_deref() {
        Some(generation) => {
            state
                .mcp
                .call_tool_bound(&body.name, &normalized_args, generation)
                .await
        }
        None => state.mcp.call_tool(&body.name, &normalized_args).await,
    };
    match result {
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
            deepseek_api_key: None,
            ollama_base_url: String::new(),
            api_key: Some("test_tok".to_string()),
            allowed_origin: None,
            max_context_budget: None,
        });

        AppState {
            permission_sessions: Default::default(),
            db,
            llm: Arc::new(cade_ai::LlmRouter::build(&cade_ai::AiConfig {
                anthropic_api_key: None,
                openai_api_key: None,
                google_api_key: None,
                deepseek_api_key: None,
                ollama_base_url: String::new(),
                llm_provider: String::new(),
            })),
            llm_router: Arc::new(RwLock::new(cade_ai::LlmRouter::build(&cade_ai::AiConfig {
                anthropic_api_key: None,
                openai_api_key: None,
                google_api_key: None,
                deepseek_api_key: None,
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

    #[tokio::test]
    async fn test_call_mcp_tool_empty_name_returns_bad_request() {
        let state = test_state();
        let app = router(state);

        let req = Request::builder()
            .method(Method::POST)
            .uri("/v1/mcp/call")
            .header("Authorization", "Bearer test_tok")
            .header("Content-Type", "application/json")
            .body(Body::from(r#"{"name":"","arguments":{}}"#))
            .unwrap();

        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::BAD_REQUEST);

        let body_bytes = axum::body::to_bytes(resp.into_body(), usize::MAX)
            .await
            .unwrap();
        let json_val: Value = serde_json::from_slice(&body_bytes).unwrap();
        assert_eq!(json_val["error"], "Tool name cannot be empty");
    }

    #[tokio::test]
    async fn test_call_mcp_tool_whitespace_name_returns_bad_request() {
        let state = test_state();
        let app = router(state);

        let req = Request::builder()
            .method(Method::POST)
            .uri("/v1/mcp/call")
            .header("Authorization", "Bearer test_tok")
            .header("Content-Type", "application/json")
            .body(Body::from(r#"{"name":"   ","arguments":{}}"#))
            .unwrap();

        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
    }

    #[tokio::test]
    async fn test_call_mcp_tool_normalizes_relative_paths() {
        // -- Setup & Fixtures
        let state = test_state();
        let app = router(state);

        let req = Request::builder()
            .method(Method::POST)
            .uri("/v1/mcp/call")
            .header("Authorization", "Bearer test_tok")
            .header("Content-Type", "application/json")
            .body(Body::from(
                r#"{"name":"missing__tool","arguments":{"path":"."},"workspace_dir":"/custom/workspace"}"#,
            ))
            .unwrap();

        // -- Exec
        let resp = app.oneshot(req).await.unwrap();

        // -- Check
        assert_eq!(resp.status(), StatusCode::NOT_FOUND);
    }

    async fn fixture_server() -> (String, tokio::task::JoinHandle<()>) {
        let (url, task, _) = counted_fixture_server().await;
        (url, task)
    }

    async fn counted_fixture_server() -> (
        String,
        tokio::task::JoinHandle<()>,
        Arc<std::sync::atomic::AtomicUsize>,
    ) {
        let calls = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let handler_calls = calls.clone();
        let app = axum::Router::new().route("/mcp", axum::routing::post(move |headers: axum::http::HeaderMap, Json(request): Json<Value>| {
            let calls = handler_calls.clone();
            async move {
            use axum::response::IntoResponse;
            let Some(id) = request.get("id") else { return StatusCode::ACCEPTED.into_response(); };
            let description = headers.get("authorization").and_then(|value| value.to_str().ok())
                .map(|auth| format!("{auth};{}", headers.get("x-fixture-revision").and_then(|value| value.to_str().ok()).unwrap_or("")))
                .unwrap_or_else(|| "live schema".into());
            let result = match request["method"].as_str().unwrap_or_default() {
                "initialize" => json!({"protocolVersion": request["params"]["protocolVersion"], "capabilities": {"tools": {}}, "serverInfo": {"name": "catalog-fixture", "version": "1"}}),
                "tools/list" => json!({"tools": [{"name": "echo", "description": description, "inputSchema": {"type": "object", "properties": {"live": {"type": "string"}}, "required": ["live"]}, "annotations": {"readOnlyHint": true}}]}),
                "tools/call" => { calls.fetch_add(1, std::sync::atomic::Ordering::SeqCst); json!({"content": [{"type": "text", "text": "executed"}], "isError": false}) },
                _ => json!({}),
            };
            Json(json!({"jsonrpc": "2.0", "id": id, "result": result})).into_response()
        }}));
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("http://{}/mcp", listener.local_addr().unwrap());
        let task = tokio::spawn(async move {
            axum::serve(listener, app).await.unwrap();
        });
        (url, task, calls)
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn mcp_pipeline_rejects_reload_between_authorization_and_dispatch() {
        use cade_agent::tools::{AutoApprovalDelegate, ToolPipeline, ToolRuntime};
        use cade_core::{
            hooks::{HookEngine, LuaHookRunner},
            permissions::{PermissionManager, PermissionMode},
            settings::HooksConfig,
        };
        struct ReloadHook {
            mcp: Arc<crate::server::state::McpManager>,
            config: McpServerConfig,
        }
        impl LuaHookRunner for ReloadHook {
            fn run_hook(&self, event: &str, _: &Value) -> Option<String> {
                if event == "pre_tool_use" {
                    tokio::task::block_in_place(|| {
                        tokio::runtime::Handle::current().block_on(self.mcp.reload(
                            &HashMap::from([("fixture".into(), self.config.clone())]),
                            None,
                        ))
                    });
                }
                None
            }
        }
        let (url, server, calls) = counted_fixture_server().await;
        let workspace = tempfile::tempdir().unwrap();
        for (source, replacement_is_write) in [
            ("local", true),
            ("local", false),
            ("remote-manager", true),
            ("remote-storage", false),
        ] {
            let config = McpServerConfig {
                url: Some(url.clone()),
                ..Default::default()
            };
            let (manager, _) = crate::server::state::McpManager::start(
                &HashMap::from([("fixture".into(), config.clone())]),
                None,
            )
            .await;
            let mcp = Arc::new(manager);
            let mut replacement = config;
            replacement.core_server = true;
            if replacement_is_write {
                replacement.write_tools = vec!["echo".into()];
            }
            let mut state = test_state();
            state.mcp = mcp.clone();
            let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
            let api_url = format!("http://{}", listener.local_addr().unwrap());
            let api = tokio::spawn(async move {
                axum::serve(listener, router(state)).await.unwrap();
            });
            let client = Arc::new(
                cade_agent::agent::HttpTransport::new(api_url, "test_tok".into()).unwrap(),
            );
            let runtime_manager = match source {
                "remote-manager" => Arc::new(crate::server::state::McpManager::from_remote(
                    client.clone(),
                )),
                "remote-storage" => Arc::new(crate::server::state::McpManager::empty()),
                _ => mcp.clone(),
            };
            let runtime = Arc::new(ToolRuntime::new(
                client,
                runtime_manager,
                "agent".into(),
                workspace.path().into(),
            ));
            let hooks = HookEngine::new(
                HooksConfig::default(),
                workspace.path().into(),
                "session".into(),
            )
            .with_lua_runner(Arc::new(ReloadHook {
                mcp,
                config: replacement,
            }));
            let pipeline = ToolPipeline::new(
                runtime,
                PermissionManager::new(PermissionMode::Plan),
                Arc::new(hooks),
                Arc::new(AutoApprovalDelegate),
            );
            let result = pipeline
                .execute("call", "fixture__echo", &json!({"live": "value"}))
                .await
                .unwrap();
            assert!(
                result.is_error,
                "implementation replacement must require reauthorization: {}",
                result.output
            );
            assert!(
                result.output.contains("authorization"),
                "{source}: {}",
                result.output
            );
            assert_eq!(
                calls.load(std::sync::atomic::Ordering::SeqCst),
                0,
                "replacement was never authorized"
            );
            // A fresh permission pass binds the replacement and can execute it,
            // including across the real HTTP status and call endpoints.
            let fresh = ToolPipeline::new(
                pipeline.runtime().clone(),
                PermissionManager::new(PermissionMode::Default),
                Arc::new(HookEngine::new(
                    HooksConfig::default(),
                    workspace.path().into(),
                    "fresh".into(),
                )),
                Arc::new(AutoApprovalDelegate),
            );
            let result = fresh
                .execute("fresh", "fixture__echo", &json!({"live": "value"}))
                .await
                .unwrap();
            assert!(!result.is_error, "{source}: {}", result.output);
            assert_eq!(calls.swap(0, std::sync::atomic::Ordering::SeqCst), 1);
            api.abort();
        }
        server.abort();
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn withdrawn_mcp_does_not_fall_through_to_extension() {
        use cade_agent::tools::runtime::{RuntimeToolResult, ToolExtension};
        use cade_agent::tools::{AutoApprovalDelegate, ToolPipeline, ToolRuntime};
        use cade_core::{
            hooks::{HookEngine, LuaHookRunner},
            permissions::{PermissionManager, PermissionMode},
            settings::HooksConfig,
        };
        struct Withdraw(Arc<crate::server::state::McpManager>);
        impl LuaHookRunner for Withdraw {
            fn run_hook(&self, event: &str, _: &Value) -> Option<String> {
                if event == "pre_tool_use" {
                    tokio::task::block_in_place(|| {
                        tokio::runtime::Handle::current()
                            .block_on(self.0.reload(&HashMap::new(), None))
                    });
                }
                None
            }
        }
        struct Extension;
        #[async_trait::async_trait]
        impl ToolExtension for Extension {
            fn has_tool(&self, name: &str) -> bool {
                name == "fixture__echo"
            }
            async fn execute(&self, _: &str, _: &str, _: &Value) -> RuntimeToolResult {
                panic!("extension was not authorized")
            }
        }
        let (url, server) = fixture_server().await;
        let (manager, _) = crate::server::state::McpManager::start(
            &HashMap::from([(
                "fixture".into(),
                McpServerConfig {
                    url: Some(url),
                    ..Default::default()
                },
            )]),
            None,
        )
        .await;
        let mcp = Arc::new(manager);
        let workspace = tempfile::tempdir().unwrap();
        let runtime = Arc::new(
            ToolRuntime::new(
                Arc::new(
                    cade_agent::agent::HttpTransport::new(
                        "http://127.0.0.1:0".into(),
                        "unused".into(),
                    )
                    .unwrap(),
                ),
                mcp.clone(),
                "agent".into(),
                workspace.path().into(),
            )
            .with_extension(Arc::new(Extension)),
        );
        let hooks = HookEngine::new(
            HooksConfig::default(),
            workspace.path().into(),
            "session".into(),
        )
        .with_lua_runner(Arc::new(Withdraw(mcp)));
        let pipeline = ToolPipeline::new(
            runtime,
            PermissionManager::new(PermissionMode::Plan),
            Arc::new(hooks),
            Arc::new(AutoApprovalDelegate),
        );
        let result = pipeline
            .execute("call", "fixture__echo", &json!({"live": "value"}))
            .await
            .unwrap();
        assert!(result.is_error);
        assert!(
            result.output.contains("fresh authorization"),
            "{}",
            result.output
        );
        server.abort();
    }

    fn seed_catalog_rows(state: &AppState) {
        for (id, name, tags) in [
            ("legacy-live-id", "fixture__echo", vec!["cade", "mcp"]),
            ("legacy-removed-id", "removed__echo", vec!["cade", "mcp"]),
            ("native-id", "bash", vec!["cade"]),
        ] {
            cade_store::sqlite::upsert_tool(&state.db, &cade_store::sqlite::ToolRow {
                id: id.into(), name: name.into(), description: Some("obsolete".into()), source_code: None,
                json_schema: Some(json!({"name": name, "description": "obsolete", "parameters": {"type": "object", "properties": {"obsolete": {"type": "string"}}}})),
                tags: tags.into_iter().map(String::from).collect(),
            }).unwrap();
        }
    }

    #[tokio::test]
    async fn mcp_context_replaces_stale_schema_and_withdraws_removed_tools() {
        let mut state = test_state();
        let (url, server) = fixture_server().await;
        let configs = HashMap::from([(
            "fixture".into(),
            McpServerConfig {
                url: Some(url),
                ..Default::default()
            },
        )]);
        let (manager, _) = crate::server::state::McpManager::start(&configs, None).await;
        state.mcp = Arc::new(manager);
        seed_catalog_rows(&state);
        cade_store::sqlite::create_agent(
            &state.db,
            &cade_store::sqlite::AgentRow {
                id: "catalog-agent".into(),
                name: "catalog-agent".into(),
                model: "openai/gpt-6-sol".into(),
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
        let (_, _, tools) = crate::server::api::messages::context::build_context(
            state.clone(),
            "catalog-agent".into(),
            None,
            false,
        )
        .await
        .unwrap();
        assert!(tools.iter().any(|tool| tool["name"] == "bash"));
        assert!(!tools.iter().any(|tool| tool["name"] == "removed__echo"));
        let live = tools
            .iter()
            .find(|tool| tool["name"] == "fixture__echo")
            .unwrap();
        assert_eq!(live["description"], "live schema");
        assert_eq!(live["parameters"]["required"], json!(["live"]));
        state.mcp.reload(&HashMap::new(), None).await;
        let (_, _, tools) = crate::server::api::messages::context::build_context(
            state,
            "catalog-agent".into(),
            None,
            false,
        )
        .await
        .unwrap();
        assert!(!tools.iter().any(|tool| tool["name"] == "fixture__echo"));
        server.abort();
    }

    #[tokio::test]
    async fn mcp_reload_endpoint_reconciles_mirror_and_policy_metadata() {
        let state = test_state();
        seed_catalog_rows(&state);
        let (url, server) = fixture_server().await;
        let mut cfg = McpServerConfig {
            url: Some(url),
            ..Default::default()
        };
        for is_write in [false, true] {
            cfg.core_server = is_write;
            cfg.write_tools = if is_write {
                vec!["echo".into()]
            } else {
                vec![]
            };
            let _ = reload_mcp_servers(
                State(state.clone()),
                Some(Json(ReloadMcpRequest {
                    configs: Some(HashMap::from([("fixture".into(), cfg.clone())])),
                })),
            )
            .await
            .unwrap();
            let rows = cade_store::sqlite::list_tools(&state.db).unwrap();
            assert!(!rows.iter().any(|row| row.name == "removed__echo"));
            assert!(
                rows.iter()
                    .any(|row| row.name == "bash" && row.id == "native-id")
            );
            let live = rows.iter().find(|row| row.name == "fixture__echo").unwrap();
            assert_eq!(live.id, "legacy-live-id");
            assert_eq!(live.description.as_deref(), Some("live schema"));
            assert_eq!(live.tags.iter().any(|tag| tag == "core_mcp"), is_write);
            assert_eq!(state.mcp.is_write_tool("fixture__echo").await, is_write);
            let call = state
                .mcp
                .call_tool("fixture__echo", &json!({"live": "value"}))
                .await
                .unwrap()
                .unwrap();
            assert_eq!(call.0, "executed");
        }
        let _ = reload_mcp_servers(
            State(state.clone()),
            Some(Json(ReloadMcpRequest {
                configs: Some(HashMap::new()),
            })),
        )
        .await
        .unwrap();
        let rows = cade_store::sqlite::list_tools(&state.db).unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].name, "bash");
        server.abort();
    }

    #[tokio::test]
    async fn mcp_http_auth_reload_and_background_mirror_follow_live_catalog() {
        let mut state = test_state();
        let (url, server) = fixture_server().await;
        let mut cfg = McpServerConfig {
            url: Some(url),
            auth_token: Some("token-v1".into()),
            ..Default::default()
        };
        let (manager, _) = crate::server::state::McpManager::start(
            &HashMap::from([("fixture".into(), cfg.clone())]),
            None,
        )
        .await;
        state.mcp = Arc::new(manager);
        seed_catalog_rows(&state);
        let mirror = spawn_catalog_sync(state.db.clone(), state.mcp.clone());
        for (token, header) in [
            ("token-v1", ""),
            ("token-v2", ""),
            ("token-v2", "new-header"),
        ] {
            cfg.auth_token = Some(token.into());
            cfg.headers = Some(HashMap::from([(
                "x-fixture-revision".into(),
                header.into(),
            )]));
            state
                .mcp
                .reload(&HashMap::from([("fixture".into(), cfg.clone())]), None)
                .await;
            let expected = format!("Bearer {token};{header}");
            tokio::time::timeout(std::time::Duration::from_secs(5), async {
                loop {
                    let rows = cade_store::sqlite::list_tools(&state.db).unwrap();
                    if let Some(row) = rows.iter().find(|row| row.name == "fixture__echo")
                        && row.description.as_deref() == Some(expected.as_str())
                    {
                        assert_eq!(
                            row.id, "legacy-live-id",
                            "in-flight reload must not delete tool attachments"
                        );
                        assert!(!rows.iter().any(|row| row.name == "removed__echo"));
                        break;
                    }
                    tokio::time::sleep(std::time::Duration::from_millis(10)).await;
                }
            })
            .await
            .unwrap();
        }
        state.mcp.reload(&HashMap::new(), None).await;
        tokio::time::timeout(std::time::Duration::from_secs(5), async {
            loop {
                let rows = cade_store::sqlite::list_tools(&state.db).unwrap();
                if rows.len() == 1 && rows[0].name == "bash" {
                    break;
                }
                tokio::time::sleep(std::time::Duration::from_millis(10)).await;
            }
        })
        .await
        .unwrap();
        mirror.abort();
        server.abort();
    }
}
