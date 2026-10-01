//! Production-runtime contracts. No process-wide cwd or environment mutation.
use super::*;
use cade_agent::backends::{BashOutput, DirEntry, ExecutionBackend};
use cade_ai::{CompletionResponse, LlmProvider};
use runtime::{RunExecutionOptions, RunHandle, RunRequest, ServerAgentRuntime};
use std::{
    path::{Path, PathBuf},
    pin::Pin,
    sync::{
        Arc,
        atomic::{AtomicBool, AtomicUsize, Ordering},
    },
};

type ModelStream = Pin<Box<dyn futures::Stream<Item = cade_ai::Result<StreamChunk>> + Send>>;

struct Workspace(PathBuf);
impl Workspace {
    fn new() -> Self {
        let path = std::env::temp_dir().join(format!("cade-runtime-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&path).unwrap();
        Self(path.canonicalize().unwrap())
    }
}
impl Drop for Workspace {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn state(provider: Arc<dyn LlmProvider>) -> AppState {
    let state = super::tests::build_state_with_llm(provider);
    sqlite::create_agent(
        &state.db,
        &sqlite::AgentRow {
            id: "parent".into(),
            name: "Runtime contract".into(),
            model: "test".into(),
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
        agent_id: "parent".into(),
        conversation_id: None,
        input: "work".into(),
        permission_mode: None,
    }
}

fn call(name: &str, arguments: Value) -> LlmToolCall {
    LlmToolCall {
        id: format!("call-{}", uuid::Uuid::new_v4()),
        name: name.into(),
        arguments,
        thought_signature: None,
    }
}

struct ScriptedProvider {
    tools: Vec<LlmToolCall>,
    calls: AtomicUsize,
    reasoning: parking_lot::Mutex<Vec<Option<String>>>,
    contexts: parking_lot::Mutex<Vec<Vec<cade_ai::LlmMessage>>>,
    partial_error: bool,
}
impl ScriptedProvider {
    fn new(tools: Vec<LlmToolCall>) -> Self {
        Self {
            tools,
            calls: AtomicUsize::new(0),
            reasoning: Default::default(),
            contexts: Default::default(),
            partial_error: false,
        }
    }
}
#[async_trait::async_trait]
impl LlmProvider for ScriptedProvider {
    async fn complete(&self, _: &CompletionRequest) -> cade_ai::Result<CompletionResponse> {
        Err(cade_ai::Error::custom("complete is not used by these runs"))
    }
    async fn stream(&self, request: &CompletionRequest) -> cade_ai::Result<ModelStream> {
        self.reasoning.lock().push(request.reasoning_effort.clone());
        self.contexts.lock().push(request.messages.clone());
        let first = self.calls.fetch_add(1, Ordering::SeqCst) == 0;
        let mut chunks = if first {
            self.tools
                .iter()
                .cloned()
                .map(|call| Ok(StreamChunk::ToolCall(call)))
                .collect::<Vec<_>>()
        } else {
            vec![Ok(StreamChunk::Text("finished".into()))]
        };
        if self.partial_error {
            chunks.push(Err(cade_ai::Error::custom("broken stream")));
        } else {
            chunks.push(Ok(StreamChunk::Done));
        }
        Ok(Box::pin(futures::stream::iter(chunks)))
    }
}

async fn drain(handle: &mut RunHandle) -> Vec<Value> {
    tokio::time::timeout(std::time::Duration::from_secs(10), async {
        let mut events = vec![];
        while let Some(Ok(event)) = handle.events.recv().await {
            if event.data != "[DONE]" {
                events.push(serde_json::from_str(&event.data).unwrap());
            }
        }
        events
    })
    .await
    .expect("run must reach a terminal event")
}

fn tool_results(events: &[Value]) -> Vec<&Value> {
    events
        .iter()
        .filter(|event| event["message_type"] == "tool_result_message")
        .map(|event| &event["tool_result"])
        .collect()
}

#[tokio::test]
async fn two_workspaces_keep_relative_paths_modes_and_reasoning_separate() {
    let first = Workspace::new();
    let second = Workspace::new();
    std::fs::write(first.0.join("same.txt"), "first workspace").unwrap();
    std::fs::write(second.0.join("same.txt"), "second workspace").unwrap();
    for (workspace, mode, contents, write_allowed) in [
        (&first, "plan", "first workspace", false),
        (&second, "accept_edits", "second workspace", true),
    ] {
        let provider = Arc::new(ScriptedProvider::new(vec![
            call("read_file", json!({"path":"same.txt"})),
            call(
                "write_file",
                json!({"path":"created.txt", "content":"written here"}),
            ),
        ]));
        let state = state(provider.clone());
        let mut handle = ServerAgentRuntime::new(state.clone())
            .start_with_options(
                request(),
                RunExecutionOptions {
                    cwd: Some(workspace.0.clone()),
                    permission_mode: Some(mode.into()),
                    permissions: Some(Default::default()),
                    allowed_paths: Some(vec![".".into()]),
                    execution: Some(Default::default()),
                    reasoning_effort: Some("high".into()),
                    ..Default::default()
                },
            )
            .await
            .unwrap();
        let events = drain(&mut handle).await;
        let results = tool_results(&events);
        assert_eq!(results.len(), 2, "{events:?}");
        assert!(
            results[0]["output"].as_str().unwrap().contains(contents),
            "{results:?}"
        );
        assert_eq!(results[0]["is_error"], false);
        assert_eq!(results[1]["is_error"], !write_allowed);
        assert_eq!(workspace.0.join("created.txt").exists(), write_allowed);
        assert!(
            provider
                .reasoning
                .lock()
                .iter()
                .all(|effort| effort.as_deref() == Some("high"))
        );
        assert_eq!(
            sqlite::get_run(&state.db, &handle.run_id)
                .unwrap()
                .unwrap()
                .status,
            "done"
        );
    }
    assert!(!first.0.join("created.txt").exists());
}

#[tokio::test]
async fn allowed_paths_and_readonly_backend_survive_the_public_runtime_boundary() {
    let workspace = Workspace::new();
    std::fs::create_dir(workspace.0.join("allowed")).unwrap();
    std::fs::write(workspace.0.join("allowed/file.txt"), "allowed content").unwrap();
    std::fs::write(workspace.0.join("private.txt"), "must not disclose").unwrap();
    let provider = Arc::new(ScriptedProvider::new(vec![
        call("read_file", json!({"path":"allowed/file.txt"})),
        call("read_file", json!({"path":"private.txt"})),
        call(
            "write_file",
            json!({"path":"allowed/new.txt", "content":"not writable"}),
        ),
    ]));
    let state = state(provider);
    let mut handle = ServerAgentRuntime::new(state.clone())
        .start_with_options(
            request(),
            RunExecutionOptions {
                cwd: Some(workspace.0.clone()),
                permission_mode: Some("bypass".into()),
                permissions: Some(Default::default()),
                allowed_paths: Some(vec!["allowed".into()]),
                execution: Some(cade_core::settings::ExecutionProfile {
                    backend: cade_core::settings::ExecutionBackendKind::ReadOnly,
                    ..Default::default()
                }),
                ..Default::default()
            },
        )
        .await
        .unwrap();
    let events = drain(&mut handle).await;
    let results = tool_results(&events);
    assert_eq!(results.len(), 3);
    assert_eq!(results[0]["is_error"], false, "{results:?}");
    assert_eq!(results[1]["is_error"], true, "{results:?}");
    assert!(
        !results[1]["output"]
            .as_str()
            .unwrap()
            .contains("must not disclose")
    );
    assert_eq!(results[2]["is_error"], true, "{results:?}");
    assert!(!workspace.0.join("allowed/new.txt").exists());
    for event in results {
        let rows = sqlite::list_messages(&state.db, "parent", None, 100).unwrap();
        assert!(rows.iter().any(|row| row.role == "tool"
            && row.content["tool_call_id"] == event["id"]
            && row.content["is_error"] == event["is_error"]));
    }
}

#[tokio::test]
async fn explicit_max_turns_is_a_hard_cap_and_stream_failure_never_executes_partial_calls() {
    for (max_turns, partial_error) in [(Some(1), false), (None, true)] {
        let workspace = Workspace::new();
        let mut provider = ScriptedProvider::new(vec![call(
            "write_file",
            json!({"path":"output.txt", "content":"hello"}),
        )]);
        provider.partial_error = partial_error;
        let provider = Arc::new(provider);
        let state = state(provider.clone());
        let mut handle = ServerAgentRuntime::new(state.clone())
            .start_with_options(
                request(),
                RunExecutionOptions {
                    cwd: Some(workspace.0.clone()),
                    permission_mode: Some("accept_edits".into()),
                    permissions: Some(Default::default()),
                    execution: Some(Default::default()),
                    max_turns,
                    ..Default::default()
                },
            )
            .await
            .unwrap();
        let events = drain(&mut handle).await;
        assert_eq!(provider.calls.load(Ordering::SeqCst), 1);
        assert_eq!(workspace.0.join("output.txt").exists(), !partial_error);
        assert_eq!(
            sqlite::get_run(&state.db, &handle.run_id)
                .unwrap()
                .unwrap()
                .status,
            "error"
        );
        assert_eq!(events.last().unwrap()["status"], "error");
    }
}

struct BlockedSetup(tokio::sync::Notify);
#[async_trait::async_trait]
impl LlmProvider for BlockedSetup {
    async fn complete(&self, _: &CompletionRequest) -> cade_ai::Result<CompletionResponse> {
        unreachable!()
    }
    async fn stream(&self, _: &CompletionRequest) -> cade_ai::Result<ModelStream> {
        self.0.notify_one();
        futures::future::pending().await
    }
}

#[tokio::test]
async fn cancellation_interrupts_provider_setup_without_waiting_for_a_chunk() {
    let provider = Arc::new(BlockedSetup(tokio::sync::Notify::new()));
    let state = state(provider.clone());
    let mut handle = ServerAgentRuntime::new(state.clone())
        .try_start(request())
        .await
        .unwrap();
    tokio::time::timeout(std::time::Duration::from_secs(5), provider.0.notified())
        .await
        .unwrap();
    sqlite::request_run_cancellation(&state.db, &handle.run_id).unwrap();
    let events = drain(&mut handle).await;
    assert_eq!(events.last().unwrap()["status"], "cancelled");
}

struct BlockingBackend {
    entered: tokio::sync::Notify,
    dropped: Arc<AtomicBool>,
}
struct InvocationGuard(Arc<AtomicBool>);
impl Drop for InvocationGuard {
    fn drop(&mut self) {
        self.0.store(true, Ordering::SeqCst);
    }
}
#[async_trait::async_trait]
impl ExecutionBackend for BlockingBackend {
    async fn exec_bash(&self, _: &str, _: &Path, _: u64) -> cade_agent::Result<BashOutput> {
        unreachable!()
    }
    async fn read_file(&self, _: &Path) -> cade_agent::Result<String> {
        let _guard = InvocationGuard(self.dropped.clone());
        self.entered.notify_one();
        futures::future::pending().await
    }
    async fn write_file(&self, _: &Path, _: &str) -> cade_agent::Result<()> {
        unreachable!()
    }
    async fn path_exists(&self, _: &Path) -> bool {
        true
    }
    async fn list_dir(&self, _: &Path) -> cade_agent::Result<Vec<DirEntry>> {
        Ok(vec![])
    }
    fn name(&self) -> &'static str {
        "blocked-test"
    }
}

#[tokio::test]
async fn cancellation_aborts_awaited_tool_before_terminal_event() {
    let workspace = Workspace::new();
    let backend = Arc::new(BlockingBackend {
        entered: Default::default(),
        dropped: Arc::new(AtomicBool::new(false)),
    });
    let state = state(Arc::new(ScriptedProvider::new(vec![call(
        "read_file",
        json!({"path":"blocked.txt"}),
    )])));
    let mut handle = ServerAgentRuntime::new(state.clone())
        .start_with_options(
            request(),
            RunExecutionOptions {
                cwd: Some(workspace.0.clone()),
                backend: Some(backend.clone()),
                permissions: Some(Default::default()),
                ..Default::default()
            },
        )
        .await
        .unwrap();
    tokio::time::timeout(
        std::time::Duration::from_secs(5),
        backend.entered.notified(),
    )
    .await
    .unwrap();
    sqlite::request_run_cancellation(&state.db, &handle.run_id).unwrap();
    let events = drain(&mut handle).await;
    assert!(backend.dropped.load(Ordering::SeqCst));
    assert_eq!(events.last().unwrap()["status"], "cancelled");
}

#[tokio::test]
async fn inline_question_cancellation_withdraws_the_run_bound_queue_entry() {
    let state = state(Arc::new(ScriptedProvider::new(vec![call(
        "ask_user_question",
        json!({
            "questions":[{"question":"Choose an option", "header":"Choice", "options":[{"label":"A", "description":"First"}, {"label":"B", "description":"Second"}], "multiSelect":false}]
        }),
    )])));
    let mut handle = ServerAgentRuntime::new(state.clone())
        .try_start(request())
        .await
        .unwrap();
    let question = tokio::time::timeout(std::time::Duration::from_secs(5), async {
        loop {
            let event = handle.events.recv().await.unwrap().unwrap();
            let event: Value = serde_json::from_str(&event.data).unwrap();
            if event["message_type"] == "question_required" {
                break event;
            }
        }
    })
    .await
    .unwrap();
    assert_eq!(question["run_id"], handle.run_id);
    assert!(question["seq_id"].is_number());
    sqlite::request_run_cancellation(&state.db, &handle.run_id).unwrap();
    let events = drain(&mut handle).await;
    assert_eq!(events.last().unwrap()["status"], "cancelled");
    assert!(
        sqlite::list_pending_approvals(&state.db)
            .unwrap()
            .is_empty()
    );
    assert!(
        sqlite::get_approval_status(&state.db, question["id"].as_str().unwrap())
            .unwrap()
            .unwrap()
            .starts_with("denied:")
    );
}

#[tokio::test]
async fn run_acceptance_rejects_foreign_conversations_and_storage_failure_without_execution() {
    let provider = Arc::new(ScriptedProvider::new(vec![]));
    let state = state(provider.clone());
    let other = sqlite::AgentRow {
        id: "other".into(),
        name: "Other".into(),
        model: "test".into(),
        description: None,
        system_prompt: None,
        created_at: None,
        compaction_model: None,
        theme: None,
        active_plan_json: None,
        parent_id: None,
    };
    sqlite::create_agent(&state.db, &other).unwrap();
    let foreign = sqlite::create_conversation(&state.db, "other", "foreign").unwrap();
    let runtime = ServerAgentRuntime::new(state.clone());
    let mut foreign_request = request();
    foreign_request.conversation_id = Some(foreign.id);
    let rejected = runtime.try_start(foreign_request).await.err().unwrap();
    assert_eq!(rejected.status, axum::http::StatusCode::NOT_FOUND);
    state
        .db
        .get()
        .unwrap()
        .execute_batch("DROP TABLE runs")
        .unwrap();
    assert!(runtime.try_start(request()).await.is_err());
    assert!(
        sqlite::list_messages(&state.db, "parent", None, 100)
            .unwrap()
            .is_empty()
    );
    assert_eq!(provider.calls.load(Ordering::SeqCst), 0);
    let mut rejected = runtime.start(request()).await;
    assert!(
        rejected.run_id.is_empty(),
        "rejection must never invent a run id"
    );
    assert_eq!(
        drain(&mut rejected).await.last().unwrap()["status"],
        "error"
    );
}

struct PausedProvider {
    script: ScriptedProvider,
    entered: tokio::sync::Notify,
    release: tokio::sync::Notify,
}
#[async_trait::async_trait]
impl LlmProvider for PausedProvider {
    async fn complete(&self, _: &CompletionRequest) -> cade_ai::Result<CompletionResponse> {
        unreachable!()
    }
    async fn stream(&self, request: &CompletionRequest) -> cade_ai::Result<ModelStream> {
        if self.script.calls.load(Ordering::SeqCst) == 0 {
            self.entered.notify_one();
            self.release.notified().await;
        }
        self.script.stream(request).await
    }
}

#[tokio::test]
async fn workspace_settings_are_snapshotted_once_and_new_runs_observe_changes() {
    let workspace = Workspace::new();
    std::fs::create_dir(workspace.0.join(".cade")).unwrap();
    let settings = workspace.0.join(".cade/settings.local.json");
    std::fs::write(&settings, r#"{"reasoning_effort":"high"}"#).unwrap();
    std::fs::write(workspace.0.join("file.txt"), "context").unwrap();
    let provider = Arc::new(PausedProvider {
        script: ScriptedProvider::new(vec![call("read_file", json!({"path":"file.txt"}))]),
        entered: Default::default(),
        release: Default::default(),
    });
    let state = state(provider.clone());
    let runtime = ServerAgentRuntime::new(state.clone());
    let options = RunExecutionOptions {
        cwd: Some(workspace.0.clone()),
        permissions: Some(Default::default()),
        execution: Some(Default::default()),
        ..Default::default()
    };
    let mut handle = runtime
        .start_with_options(request(), options.clone())
        .await
        .unwrap();
    tokio::time::timeout(
        std::time::Duration::from_secs(5),
        provider.entered.notified(),
    )
    .await
    .unwrap();
    std::fs::write(&settings, r#"{"reasoning_effort":"low"}"#).unwrap();
    provider.release.notify_one();
    drain(&mut handle).await;
    assert_eq!(
        *provider.script.reasoning.lock(),
        vec![Some("high".into()), Some("high".into())]
    );
    let mut next = runtime
        .start_with_options(request(), options)
        .await
        .unwrap();
    drain(&mut next).await;
    assert_eq!(
        provider.script.reasoning.lock().last().unwrap().as_deref(),
        Some("low")
    );
}

#[tokio::test]
async fn http_and_messages_compatibility_handlers_forward_execution_options() {
    for compatibility in [false, true] {
        let workspace = Workspace::new();
        let provider = Arc::new(ScriptedProvider::new(vec![call(
            "write_file",
            json!({"path":"blocked.txt", "content":"blocked"}),
        )]));
        let state = state(provider.clone());
        let body = json!({
            "input":"work", "workspace":workspace.0, "permission_mode":"plan",
            "permissions":{}, "execution":{"backend":"local"}, "reasoning_effort":"high", "max_turns":1,
        });
        let response = if compatibility {
            crate::server::api::messages::stream_message(
                State(state.clone()),
                Path("parent".into()),
                Json(body),
            )
            .await
        } else {
            run_agent(State(state.clone()), Path("parent".into()), Json(body)).await
        };
        assert_eq!(response.status(), axum::http::StatusCode::OK);
        let bytes = tokio::time::timeout(
            std::time::Duration::from_secs(10),
            axum::body::to_bytes(response.into_body(), usize::MAX),
        )
        .await
        .unwrap()
        .unwrap();
        let text = String::from_utf8(bytes.to_vec()).unwrap();
        assert!(
            text.contains("tool_result_message") && text.contains("\"status\":\"error\""),
            "{text}"
        );
        assert!(!workspace.0.join("blocked.txt").exists());
        assert_eq!(*provider.reasoning.lock(), vec![Some("high".into())]);
    }
}

struct MetadataExecutor;
#[async_trait::async_trait]
impl runtime::CapabilityExecutor for MetadataExecutor {
    async fn execute(
        &self,
        _: runtime::TurnExecutionInput,
        calls: Vec<LlmToolCall>,
        _: SseTx,
    ) -> Vec<(cade_agent::tools::manager::ToolResult, Value)> {
        calls
            .into_iter()
            .map(|call| {
                (
                    cade_agent::tools::manager::ToolResult {
                        tool_call_id: call.id,
                        tool_name: call.name,
                        output: "界".repeat(4000),
                        is_error: true,
                        ui_resource_uri: Some("ui://test/result".into()),
                    },
                    call.arguments,
                )
            })
            .collect()
    }
}
struct StoredContext(sqlite::Db);
#[async_trait::async_trait]
impl runtime::ContextBuilder for StoredContext {
    async fn build(
        &self,
        agent: String,
        conversation: Option<String>,
        is_tool_return: bool,
    ) -> Result<runtime::RunContext, String> {
        let rows = sqlite::list_messages(&self.0, &agent, conversation.as_deref(), 100)
            .map_err(|e| e.to_string())?;
        if is_tool_return {
            let result = rows
                .iter()
                .find(|row| row.role == "tool")
                .expect("tool result must precede next context");
            assert_eq!(result.content["content"].as_str().unwrap().len(), 12000);
            assert_eq!(result.content["ui_resource_uri"], "ui://test/result");
            assert_eq!(result.content["is_error"], true);
        }
        Ok(("test".into(), vec![], vec![]))
    }
}

#[tokio::test]
async fn full_result_and_metadata_are_durable_before_result_event_and_next_context() {
    let state = state(Arc::new(ScriptedProvider::new(vec![call(
        "metadata",
        json!({}),
    )])));
    let runtime = ServerAgentRuntime::with_dependencies(
        state.clone(),
        Arc::new(StoredContext(state.db.clone())),
        Arc::new(MetadataExecutor),
    );
    let mut handle = runtime.try_start(request()).await.unwrap();
    let events = tokio::time::timeout(std::time::Duration::from_secs(5), async {
        let mut events = vec![];
        while let Some(Ok(event)) = handle.events.recv().await {
            if event.data == "[DONE]" {
                continue;
            }
            let event: Value = serde_json::from_str(&event.data).unwrap();
            if event["message_type"] == "tool_result_message" {
                let rows = sqlite::list_messages(&state.db, "parent", None, 100).unwrap();
                let stored = rows
                    .iter()
                    .find(|row| row.role == "tool")
                    .expect("persist before event");
                assert_eq!(stored.content["content"].as_str().unwrap().len(), 12000);
                assert_eq!(
                    stored.content["ui_resource_uri"],
                    event["tool_result"]["ui_resource_uri"]
                );
            }
            events.push(event);
        }
        events
    })
    .await
    .unwrap();
    let result = tool_results(&events)[0];
    assert_eq!(result["ui_resource_uri"], "ui://test/result");
    assert_eq!(result["is_error"], true);
    assert!(result["output"].as_str().unwrap().contains("truncated"));
    assert_eq!(events.last().unwrap()["status"], "done");
}

#[tokio::test]
async fn configured_allow_rule_executes_default_mode_and_result_storage_failure_stops_the_run() {
    let workspace = Workspace::new();
    let provider = Arc::new(ScriptedProvider::new(vec![call(
        "write_file",
        json!({"path":"allowed.txt", "content":"written"}),
    )]));
    let state = state(provider.clone());
    state.db.get().unwrap().execute_batch(
        "CREATE TRIGGER reject_tool_result BEFORE INSERT ON messages WHEN NEW.role = 'tool' BEGIN SELECT RAISE(FAIL, 'tool result storage unavailable'); END;"
    ).unwrap();
    let mut handle = ServerAgentRuntime::new(state.clone())
        .start_with_options(
            request(),
            RunExecutionOptions {
                cwd: Some(workspace.0.clone()),
                permissions: Some(cade_core::settings::PermissionSettings {
                    allow: vec!["write_file".into()],
                    ..Default::default()
                }),
                execution: Some(Default::default()),
                ..Default::default()
            },
        )
        .await
        .unwrap();
    let events = drain(&mut handle).await;
    assert_eq!(
        std::fs::read_to_string(workspace.0.join("allowed.txt")).unwrap(),
        "written"
    );
    assert!(
        tool_results(&events).is_empty(),
        "never publish an undurable result"
    );
    assert_eq!(
        provider.calls.load(Ordering::SeqCst),
        1,
        "never rebuild context after failed persistence"
    );
    assert_eq!(events.last().unwrap()["status"], "error");
    assert!(events.iter().any(|event| {
        event["error"]
            .as_str()
            .is_some_and(|error| error.contains("tool result storage unavailable"))
    }));
}

#[tokio::test]
async fn shared_agent_context_uses_each_accepted_workspace_without_cache_leakage() {
    let first = Workspace::new();
    let second = Workspace::new();
    let provider = Arc::new(ScriptedProvider::new(vec![]));
    let state = state(provider.clone());
    let runtime = ServerAgentRuntime::new(state);
    for workspace in [&first, &second] {
        let mut handle = runtime
            .start_with_options(
                request(),
                RunExecutionOptions {
                    cwd: Some(workspace.0.clone()),
                    permissions: Some(Default::default()),
                    execution: Some(Default::default()),
                    ..Default::default()
                },
            )
            .await
            .unwrap();
        drain(&mut handle).await;
    }
    let contexts = provider.contexts.lock();
    assert_eq!(contexts.len(), 2);
    for (context, own, other) in [
        (&contexts[0], &first, &second),
        (&contexts[1], &second, &first),
    ] {
        let system = context
            .iter()
            .filter(|message| message.role == "system")
            .map(|message| message.content.as_str())
            .collect::<Vec<_>>()
            .join("\n");
        assert!(system.contains(own.0.to_str().unwrap()), "{system}");
        assert!(!system.contains(other.0.to_str().unwrap()), "{system}");
    }
}

struct BackgroundSnapshotProvider {
    parent_calls: AtomicUsize,
    child_calls: AtomicUsize,
    parent_waiting: tokio::sync::Notify,
    child_reasoning: parking_lot::Mutex<Vec<Option<String>>>,
}
#[async_trait::async_trait]
impl LlmProvider for BackgroundSnapshotProvider {
    async fn complete(&self, request: &CompletionRequest) -> cade_ai::Result<CompletionResponse> {
        self.child_reasoning
            .lock()
            .push(request.reasoning_effort.clone());
        if self.child_calls.fetch_add(1, Ordering::SeqCst) == 0 {
            Ok(CompletionResponse {
                content: None,
                tool_calls: vec![
                    call("read_file", json!({"path":"allowed/file.txt"})),
                    call(
                        "write_file",
                        json!({"path":"allowed/new.txt", "content":"blocked by inherited backend"}),
                    ),
                    call("read_file", json!({"path":"private.txt"})),
                ],
                finish_reason: "tool_calls".into(),
            })
        } else {
            let result = request
                .messages
                .iter()
                .filter(|message| message.role == "tool")
                .map(|message| message.content.as_str())
                .collect::<Vec<_>>()
                .join("\n");
            Ok(CompletionResponse {
                content: Some(format!("Child result: {result}")),
                tool_calls: vec![],
                finish_reason: "stop".into(),
            })
        }
    }
    async fn stream(&self, _: &CompletionRequest) -> cade_ai::Result<ModelStream> {
        if self.parent_calls.fetch_add(1, Ordering::SeqCst) == 0 {
            Ok(Box::pin(futures::stream::iter(vec![
                Ok(StreamChunk::ToolCall(call(
                    "run_subagent",
                    json!({"prompt":"inspect files", "background":true}),
                ))),
                Ok(StreamChunk::Done),
            ])))
        } else {
            self.parent_waiting.notify_one();
            futures::future::pending().await
        }
    }
}

#[tokio::test]
async fn queued_background_child_owns_workspace_backend_and_policy_after_parent_cancel() {
    for contents in ["workspace one", "workspace two"] {
        let workspace = Workspace::new();
        std::fs::create_dir(workspace.0.join("allowed")).unwrap();
        std::fs::write(workspace.0.join("allowed/file.txt"), contents).unwrap();
        std::fs::write(workspace.0.join("private.txt"), "must not disclose").unwrap();
        let provider = Arc::new(BackgroundSnapshotProvider {
            parent_calls: AtomicUsize::new(0),
            child_calls: AtomicUsize::new(0),
            parent_waiting: Default::default(),
            child_reasoning: Default::default(),
        });
        let state = state(provider.clone());
        super::tests::seed_test_tools(
            &state.db,
            "parent",
            &["read_file", "write_file", "run_subagent"],
        );
        let slots = state
            .subagent_semaphore
            .clone()
            .acquire_many_owned(4)
            .await
            .unwrap();
        let mut handle = ServerAgentRuntime::new(state.clone())
            .start_with_options(
                request(),
                RunExecutionOptions {
                    cwd: Some(workspace.0.clone()),
                    allowed_paths: Some(vec!["allowed".into()]),
                    permission_mode: Some("bypass".into()),
                    permissions: Some(Default::default()),
                    execution: Some(cade_core::settings::ExecutionProfile {
                        backend: cade_core::settings::ExecutionBackendKind::ReadOnly,
                        ..Default::default()
                    }),
                    reasoning_effort: Some("high".into()),
                    ..Default::default()
                },
            )
            .await
            .unwrap();
        tokio::time::timeout(
            std::time::Duration::from_secs(5),
            provider.parent_waiting.notified(),
        )
        .await
        .unwrap();
        assert_eq!(
            provider.child_calls.load(Ordering::SeqCst),
            0,
            "child must remain queued"
        );
        sqlite::request_run_cancellation(&state.db, &handle.run_id).unwrap();
        assert_eq!(
            drain(&mut handle).await.last().unwrap()["status"],
            "cancelled"
        );
        drop(slots);
        let result = tokio::time::timeout(std::time::Duration::from_secs(10), async {
            loop {
                if let Some(result) = sqlite::list_messages(&state.db, "parent", None, 100)
                    .unwrap()
                    .into_iter()
                    .find(|row| row.content["phase"] == "outcome")
                {
                    break result;
                }
                tokio::time::sleep(std::time::Duration::from_millis(20)).await;
            }
        })
        .await
        .expect("independent child must complete after parent cancellation");
        let output = result.content["content"].as_str().unwrap();
        assert!(output.contains(contents), "{output}");
        assert!(!output.contains("must not disclose"), "{output}");
        assert!(!workspace.0.join("allowed/new.txt").exists());
        assert_eq!(result.content["status"], "done", "{output}");
        assert_eq!(
            *provider.child_reasoning.lock(),
            vec![Some("high".into()), Some("high".into())]
        );
    }
}
