//! Server adapter for guarded native plugin tools. Discovery is shared by model
//! context and the HTTP catalogue; execution remains inside ToolPipeline.

use cade_agent::mcp::McpManager;
use cade_agent::tools::runtime::{RuntimeToolResult, ToolExtension};
use cade_core::toolsets::Toolset;
use cade_plugin::{NativePluginEngine, PluginEngine, registry::ResolvedPluginTool};
use futures::FutureExt;
use serde_json::Value;
use std::collections::HashSet;
use std::hash::{Hash, Hasher};
use std::path::{Path, PathBuf};
use std::sync::{Arc, OnceLock};

fn reserved_native_names() -> &'static HashSet<String> {
    static NAMES: OnceLock<HashSet<String>> = OnceLock::new();
    NAMES.get_or_init(|| {
        let mut names = HashSet::new();
        for toolset in [Toolset::Default, Toolset::Codex, Toolset::Gemini] {
            names.extend(toolset.all_tool_names().into_iter().map(String::from));
            for schema in cade_agent::tools::manager::schemas_for_toolset(toolset, true) {
                if let Some(name) = schema["name"].as_str() {
                    names.insert(name.to_owned());
                    names.insert(cade_agent::tools::manager::canonical_name(name).to_owned());
                }
            }
        }
        for schema in cade_agent::tools::all_meta_schemas() {
            if let Some(name) = schema["name"].as_str() {
                names.insert(name.to_owned());
            }
        }
        names.extend(
            cade_core::tool_ids::META_TOOL_IDS
                .iter()
                .map(|name| (*name).to_owned()),
        );
        names.extend(
            [
                cade_core::tool_ids::TODO_WRITE,
                cade_core::tool_ids::WRITE_TODOS,
            ]
            .into_iter()
            .map(String::from),
        );
        names
    })
}

fn candidates(cwd: &Path) -> Vec<ResolvedPluginTool> {
    NativePluginEngine::from_default_dirs(cwd)
        .list_tools()
        .into_iter()
        .filter(|tool| !reserved_native_names().contains(&tool.name))
        .collect()
}

pub(crate) struct ReadyPluginCatalog {
    pub tools: Vec<ResolvedPluginTool>,
    /// Readiness, schema and winning package identity participate in context caching.
    pub digest: u64,
}

/// Native and MCP ownership win collisions even when a built-in is not selected
/// for this agent. A declaration without a handler never reaches this catalogue.
pub(crate) async fn ready_catalog(cwd: &Path, mcp: &McpManager) -> ReadyPluginCatalog {
    let mut tools = Vec::new();
    // A remote-only manager consumes all fallback calls itself; it cannot expose
    // local script capabilities through the agent runtime's unknown-tool route.
    if mcp.remote_client.is_none() {
        for tool in candidates(cwd) {
            if !mcp.owns_tool(&tool.name).await {
                tools.push(tool);
            }
        }
    }
    let mut hash = std::collections::hash_map::DefaultHasher::new();
    for tool in &tools {
        tool.name.hash(&mut hash);
        tool.schema.to_string().hash(&mut hash);
        tool.plugin_root.hash(&mut hash);
        tool.handler.hash(&mut hash);
        if let Some(handler) = &tool.handler
            && let Ok(metadata) = handler.metadata()
        {
            metadata.len().hash(&mut hash);
            metadata.modified().ok().hash(&mut hash);
        }
    }
    ReadyPluginCatalog {
        tools,
        digest: hash.finish(),
    }
}

pub(crate) struct ServerPluginTools {
    discovery_root: PathBuf,
    execution_root: PathBuf,
    mcp: Arc<McpManager>,
}

impl ServerPluginTools {
    pub(crate) fn new(cwd: PathBuf, mcp: Arc<McpManager>) -> Self {
        Self::for_workspace(cwd.clone(), cwd, mcp)
    }

    /// Isolated children discover the parent's packages but run in their own workspace.
    pub(crate) fn for_workspace(
        discovery_root: PathBuf,
        execution_root: PathBuf,
        mcp: Arc<McpManager>,
    ) -> Self {
        Self {
            discovery_root,
            execution_root,
            mcp,
        }
    }
}

#[async_trait::async_trait]
impl ToolExtension for ServerPluginTools {
    fn has_tool(&self, name: &str) -> bool {
        if reserved_native_names().contains(name) || self.mcp.remote_client.is_some() {
            return false;
        }
        // The extension trait is synchronous. Poll cached local ownership without
        // blocking the executor. Under contention, keep a ready script classified
        // as mutating; native/MCP dispatch still wins and execute rechecks ownership.
        if self.mcp.owns_tool(name).now_or_never() == Some(true) {
            return false;
        }
        candidates(&self.discovery_root)
            .iter()
            .any(|tool| tool.name == name)
    }

    async fn execute(&self, call_id: &str, name: &str, args: &Value) -> RuntimeToolResult {
        let result = if reserved_native_names().contains(name)
            || self.mcp.remote_client.is_some()
            || self.mcp.owns_tool(name).await
        {
            Err(cade_plugin::Error::custom(format!(
                "Plugin tool '{name}' is shadowed by native/MCP ownership"
            )))
        } else {
            // Re-resolve at invocation so removal never leaves a cached callable handler.
            NativePluginEngine::from_default_dirs(&self.discovery_root)
                .with_working_directory(self.execution_root.clone())
                .dispatch(name, args)
                .await
        };
        let (output, is_error) = match result {
            Ok(output) => (output, false),
            Err(error) => (error.to_string(), true),
        };
        RuntimeToolResult {
            tool_call_id: call_id.to_owned(),
            tool_name: name.to_owned(),
            output,
            is_error,
            ui_resource_uri: None,
        }
    }
}

#[cfg(all(test, unix))]
mod tests {
    use super::super::runtime::{
        RunExecutionOptions, RunHandle, RunRequest, ServerAgentRuntime, in_execution_scope,
    };
    use super::*;
    use crate::server::api::{messages, plugins, tools};
    use crate::server::state::AppState;
    use axum::{
        Json,
        extract::{Path as ApiPath, State},
        http::StatusCode,
    };
    use cade_ai::{CompletionRequest, CompletionResponse, LlmProvider, LlmToolCall, StreamChunk};
    use cade_store::sqlite;
    use serde_json::json;
    use std::os::unix::fs::PermissionsExt;
    use std::pin::Pin;
    use std::sync::atomic::{AtomicUsize, Ordering};

    const TOOL: &str = "candidate7_script";
    struct ScriptedProvider {
        calls: AtomicUsize,
        requests: parking_lot::Mutex<Vec<CompletionRequest>>,
    }
    impl ScriptedProvider {
        fn new() -> Self {
            Self {
                calls: AtomicUsize::new(0),
                requests: Default::default(),
            }
        }
    }
    #[async_trait::async_trait]
    impl LlmProvider for ScriptedProvider {
        async fn complete(&self, _: &CompletionRequest) -> cade_ai::Result<CompletionResponse> {
            Err(cade_ai::Error::custom(
                "Tests use the canonical streaming runtime",
            ))
        }
        async fn stream(
            &self,
            request: &CompletionRequest,
        ) -> cade_ai::Result<
            Pin<Box<dyn futures::Stream<Item = cade_ai::Result<StreamChunk>> + Send>>,
        > {
            self.requests.lock().push(request.clone());
            let chunk = if self.calls.fetch_add(1, Ordering::SeqCst) == 0 {
                StreamChunk::ToolCall(LlmToolCall {
                    id: "plugin-call".into(),
                    name: TOOL.into(),
                    arguments: json!({"value":"actual script input"}),
                    thought_signature: None,
                })
            } else {
                StreamChunk::Text("finished".into())
            };
            Ok(Box::pin(futures::stream::iter(vec![
                Ok(chunk),
                Ok(StreamChunk::Done),
            ])))
        }
    }

    fn state(provider: Arc<dyn LlmProvider>) -> AppState {
        let state = super::super::tests::build_state_with_llm(provider);
        sqlite::create_agent(
            &state.db,
            &sqlite::AgentRow {
                id: "plugin-parent".into(),
                name: "Plugin runtime fixture".into(),
                model: state.config.default_model.clone(),
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
        state
    }
    fn request() -> RunRequest {
        RunRequest {
            agent_id: "plugin-parent".into(),
            conversation_id: None,
            input: "Use the plugin fixture".into(),
            permission_mode: None,
        }
    }
    fn options(cwd: &Path, mode: &str) -> RunExecutionOptions {
        RunExecutionOptions {
            cwd: Some(cwd.to_path_buf()),
            permission_mode: Some(mode.into()),
            permissions: Some(Default::default()),
            execution: Some(Default::default()),
            ..Default::default()
        }
    }

    fn package(workspace: &Path, names: &[&str]) -> cade_plugin::PackedPlugin {
        let root = workspace.join("fixture-source");
        std::fs::create_dir_all(&root).unwrap();
        let definitions: Vec<_> = names.iter().enumerate().map(|(index, name)| {
            let filename = format!("schema-{index}.json");
            std::fs::write(root.join(&filename), json!({
                "name": name, "description":"Actual native plugin", "parameters":{"type":"object","properties":{"value":{"type":"string"}}}
            }).to_string()).unwrap();
            json!({"schema":filename,"handler":"handler.sh"})
        }).collect();
        std::fs::write(
            root.join("cade-plugin.json"),
            json!({"name":"Candidate7 fixture","version":"1.0.0","tools":definitions}).to_string(),
        )
        .unwrap();
        std::fs::write(
            root.join("handler.sh"),
            b"#!/bin/sh\ncat\nprintf executed > executed.txt\n",
        )
        .unwrap();
        std::fs::set_permissions(
            root.join("handler.sh"),
            std::fs::Permissions::from_mode(0o755),
        )
        .unwrap();
        cade_plugin::pack_plugin(&root, Some(&workspace.join("fixture.tar.gz"))).unwrap()
    }
    async fn serve_archive(path: &Path) -> String {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        let bytes = std::fs::read(path).unwrap();
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        tokio::spawn(async move {
            let (mut stream, _) = listener.accept().await.unwrap();
            let mut request = [0; 4096];
            let bytes_read = stream.read(&mut request).await.unwrap();
            assert!(
                bytes_read > 0,
                "HTTP fixture must receive a request before responding"
            );
            stream
                .write_all(
                    format!(
                        "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                        bytes.len()
                    )
                    .as_bytes(),
                )
                .await
                .unwrap();
            stream.write_all(&bytes).await.unwrap();
        });
        format!("http://{address}/fixture.tar.gz")
    }
    async fn install(workspace: &Path) {
        let packed = package(workspace, &[TOOL]);
        NativePluginEngine::from_default_dirs(workspace)
            .install_with_checksum(
                &serve_archive(&packed.archive_path).await,
                "candidate7-fixture",
                Some(&packed.sha256),
            )
            .await
            .unwrap();
    }
    async fn drain(handle: &mut RunHandle) -> Vec<Value> {
        tokio::time::timeout(std::time::Duration::from_secs(10), async {
            let mut events = Vec::new();
            while let Some(Ok(event)) = handle.events.recv().await {
                if event.data != "[DONE]" {
                    events.push(serde_json::from_str(&event.data).unwrap());
                }
            }
            events
        })
        .await
        .expect("Canonical runtime must finish")
    }

    #[tokio::test]
    async fn plan_denies_plugin_mutation_and_bypass_executes_the_real_script_in_workspace() {
        for (mode, executes) in [("plan", false), ("bypass", true)] {
            let workspace = tempfile::tempdir().unwrap();
            install(workspace.path()).await;
            let provider = Arc::new(ScriptedProvider::new());
            let state = state(provider.clone());
            let mut handle = ServerAgentRuntime::new(state.clone())
                .start_with_options(request(), options(workspace.path(), mode))
                .await
                .unwrap();
            let events = drain(&mut handle).await;
            let result = events
                .iter()
                .find(|event| event["message_type"] == "tool_result_message")
                .unwrap();
            assert_eq!(result["tool_result"]["is_error"], !executes, "{events:?}");
            if executes {
                assert_eq!(
                    result["tool_result"]["output"],
                    json!({"value":"actual script input"}).to_string()
                );
                assert_eq!(
                    std::fs::read_to_string(workspace.path().join("executed.txt")).unwrap(),
                    "executed"
                );
            } else {
                assert!(
                    result["tool_result"]["output"]
                        .as_str()
                        .unwrap()
                        .contains("Plan Mode")
                );
                assert!(!workspace.path().join("executed.txt").exists());
            }
            assert!(
                provider.requests.lock()[0]
                    .tools
                    .iter()
                    .any(|schema| schema["name"] == TOOL)
            );
            assert!(
                sqlite::list_messages(&state.db, "plugin-parent", None, 20)
                    .unwrap()
                    .iter()
                    .any(|message| message.role == "tool"
                        && message.content["is_error"] == !executes)
            );
            assert_eq!(
                sqlite::get_run(&state.db, &handle.run_id)
                    .unwrap()
                    .unwrap()
                    .status,
                "done"
            );
        }
    }

    #[tokio::test]
    async fn readonly_backend_never_executes_native_plugins_even_in_bypass_mode() {
        let workspace = tempfile::tempdir().unwrap();
        install(workspace.path()).await;
        let state = state(Arc::new(ScriptedProvider::new()));
        let mut options = options(workspace.path(), "bypass");
        options.execution = Some(cade_core::settings::ExecutionProfile {
            backend: cade_core::settings::ExecutionBackendKind::ReadOnly,
            ..Default::default()
        });
        let mut handle = ServerAgentRuntime::new(state)
            .start_with_options(request(), options)
            .await
            .unwrap();
        let events = drain(&mut handle).await;
        let result = events
            .iter()
            .find(|event| event["message_type"] == "tool_result_message")
            .unwrap();
        assert_eq!(result["tool_result"]["is_error"], true);
        assert!(
            result["tool_result"]["output"]
                .as_str()
                .unwrap()
                .contains("writable local backend")
        );
        assert!(!workspace.path().join("executed.txt").exists());
    }

    #[tokio::test]
    async fn install_and_delete_refresh_context_cache_catalogue_and_existing_runtime() {
        let workspace = tempfile::tempdir().unwrap();
        let packed = package(workspace.path(), &[TOOL]);
        let state = state(Arc::new(ScriptedProvider::new()));
        // A stored plugin declaration must not become a phantom ready capability.
        sqlite::upsert_tool(
            &state.db,
            &sqlite::ToolRow {
                id: "tool-plugin-stale".into(),
                name: TOOL.into(),
                description: Some("Stale declaration".into()),
                source_code: None,
                json_schema: Some(json!({"name":TOOL,"parameters":{"type":"object"}})),
                tags: vec!["plugin".into()],
            },
        )
        .unwrap();
        messages::persist_checked(
            &state,
            "plugin-parent",
            None,
            "user",
            json!({"content":"Unchanged timeline"}),
        )
        .unwrap();
        let resolved = options(workspace.path(), "bypass")
            .resolve(&state, &request())
            .unwrap();
        let runtime = resolved.runtime.clone();
        in_execution_scope(Some(resolved), async {
            let key = messages::context_cache_key("plugin-parent", None);
            let (_, _, before) =
                messages::build_context(state.clone(), "plugin-parent".into(), None, true)
                    .await
                    .unwrap();
            assert!(!before.iter().any(|schema| schema["name"] == TOOL));
            assert!(!runtime.extension_is_write(TOOL));
            let before_hash = state.context_cache.lock().get(&key).unwrap().0;
            let _ = messages::build_context(state.clone(), "plugin-parent".into(), None, true)
                .await
                .unwrap();
            assert_eq!(
                state.context_cache.lock().get(&key).unwrap().0,
                before_hash,
                "Unchanged context can use the same cache entry"
            );
            let url = serve_archive(&packed.archive_path).await;
            let response = plugins::install_plugin_handler(
                State(state.clone()),
                Json(plugins::InstallPluginPayload {
                    url,
                    plugin_id: Some("candidate7-fixture".into()),
                    agent_id: None,
                    sha256: Some(packed.sha256),
                }),
            )
            .await;
            assert_eq!(response.status(), StatusCode::OK);
            let (_, _, installed) =
                messages::build_context(state.clone(), "plugin-parent".into(), None, true)
                    .await
                    .unwrap();
            assert_eq!(
                installed
                    .iter()
                    .filter(|schema| schema["name"] == TOOL)
                    .count(),
                1
            );
            assert_ne!(
                state.context_cache.lock().get(&key).unwrap().0,
                before_hash,
                "Readiness invalidates cached schemas without a new message"
            );
            assert!(runtime.extension_is_write(TOOL));
            let Json(catalog) = tools::list_tools(State(state.clone())).await.unwrap();
            let ready = catalog
                .as_array()
                .unwrap()
                .iter()
                .find(|tool| tool["name"] == TOOL)
                .unwrap();
            assert_eq!(ready["source"], "plugin");
            assert_eq!(ready["status"], "ready");
            assert_eq!(
                ready["json_schema"]["parameters"]["properties"]["value"]["type"],
                "string"
            );
            let response = plugins::uninstall_plugin_handler(
                State(state.clone()),
                ApiPath("candidate7-fixture".into()),
            )
            .await;
            assert_eq!(response.status(), StatusCode::OK);
            let (_, _, removed) =
                messages::build_context(state.clone(), "plugin-parent".into(), None, true)
                    .await
                    .unwrap();
            assert!(!removed.iter().any(|schema| schema["name"] == TOOL));
            assert!(!runtime.extension_is_write(TOOL));
            let Json(catalog) = tools::list_tools(State(state.clone())).await.unwrap();
            assert!(
                !catalog
                    .as_array()
                    .unwrap()
                    .iter()
                    .any(|tool| tool["name"] == TOOL)
            );
            let stale = runtime
                .execute("removed-call".into(), TOOL, &json!({}))
                .await
                .unwrap();
            assert!(stale.is_error);
            assert!(!workspace.path().join("executed.txt").exists());
        })
        .await;
    }

    #[tokio::test]
    async fn native_and_alias_collisions_are_not_advertised_or_claimed_by_extension() {
        let workspace = tempfile::tempdir().unwrap();
        let packed = package(
            workspace.path(),
            &[TOOL, "read_file", "RunShellCommand", "TodoWrite"],
        );
        NativePluginEngine::from_default_dirs(workspace.path())
            .install(
                &serve_archive(&packed.archive_path).await,
                "candidate7-fixture",
            )
            .await
            .unwrap();
        let mcp = Arc::new(McpManager::empty());
        let catalog = ready_catalog(workspace.path(), &mcp).await;
        assert!(catalog.tools.iter().any(|tool| tool.name == TOOL));
        let extension = ServerPluginTools::new(workspace.path().into(), mcp);
        for name in ["read_file", "RunShellCommand", "TodoWrite"] {
            assert!(!catalog.tools.iter().any(|tool| tool.name == name));
            assert!(!extension.has_tool(name));
            assert!(
                extension
                    .execute("collision", name, &json!({}))
                    .await
                    .is_error
            );
        }
        assert!(!workspace.path().join("executed.txt").exists());
    }

    #[tokio::test]
    async fn child_adapter_discovers_parent_packages_but_executes_only_in_child_workspace() {
        let parent = tempfile::tempdir().unwrap();
        let child = tempfile::tempdir().unwrap();
        install(parent.path()).await;
        let extension = ServerPluginTools::for_workspace(
            parent.path().into(),
            child.path().into(),
            Arc::new(McpManager::empty()),
        );
        assert!(extension.has_tool(TOOL));
        let result = extension
            .execute("child-call", TOOL, &json!({"value":"child input"}))
            .await;
        assert!(!result.is_error, "{}", result.output);
        assert!(child.path().join("executed.txt").is_file());
        assert!(!parent.path().join("executed.txt").exists());
    }
}
