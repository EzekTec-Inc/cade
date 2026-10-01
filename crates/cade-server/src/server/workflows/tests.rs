use super::*;
use crate::server::api::run::runtime::{
    CapabilityExecutor, ContextBuilder, RunContext, TurnExecutionInput,
};
use crate::server::config::{LlmProviderKind, ServerConfig};
use cade_agent::tools::manager::ToolResult;
use cade_ai::{
    CompletionRequest, CompletionResponse, LlmMessage, LlmProvider, LlmToolCall, StreamChunk,
};
use cade_api_types::WorkflowStepDef;
use serde_json::{Value, json};
use std::pin::Pin;
use std::sync::Arc;
use tokio::sync::{Mutex, RwLock, mpsc};

#[derive(Clone, Copy)]
enum ProviderBehavior {
    Success,
    RequestFailure,
    ChunkFailure,
    Pending,
}
struct ControlledProvider {
    behavior: ProviderBehavior,
    requests: Mutex<Vec<CompletionRequest>>,
    started: mpsc::UnboundedSender<()>,
}

#[async_trait::async_trait]
impl LlmProvider for ControlledProvider {
    async fn complete(&self, _: &CompletionRequest) -> cade_ai::Result<CompletionResponse> {
        Err(cade_ai::Error::custom(
            "Workflow tests must exercise streaming runtime",
        ))
    }
    async fn stream(
        &self,
        request: &CompletionRequest,
    ) -> cade_ai::Result<
        Pin<Box<dyn tokio_stream::Stream<Item = cade_ai::Result<StreamChunk>> + Send>>,
    > {
        let mut requests = self.requests.lock().await;
        requests.push(request.clone());
        let number = requests.len();
        let _ = self.started.send(());
        match self.behavior {
            ProviderBehavior::RequestFailure => {
                Err(cade_ai::Error::custom("controlled provider failure"))
            }
            ProviderBehavior::ChunkFailure => Ok(Box::pin(tokio_stream::iter(vec![Err(
                cade_ai::Error::custom("controlled stream failure"),
            )]))),
            ProviderBehavior::Pending => Ok(Box::pin(futures::stream::pending())),
            ProviderBehavior::Success => Ok(Box::pin(tokio_stream::iter(vec![
                Ok(StreamChunk::Text(format!("provider-result-{number}"))),
                Ok(StreamChunk::Done),
            ]))),
        }
    }
}

struct StoredContext(Db);
#[async_trait::async_trait]
impl ContextBuilder for StoredContext {
    async fn build(
        &self,
        agent_id: String,
        conversation_id: Option<String>,
        _: bool,
    ) -> Result<RunContext, String> {
        let model = sqlite::get_agent(&self.0, &agent_id)
            .map_err(|error| error.to_string())?
            .ok_or("Workflow agent is missing")?
            .model;
        let content = sqlite::list_messages(&self.0, &agent_id, conversation_id.as_deref(), 1)
            .map_err(|error| error.to_string())?
            .into_iter()
            .next()
            .ok_or("Runtime did not persist its input")?
            .content;
        Ok((
            model,
            vec![LlmMessage {
                role: "user".into(),
                content: content["content"].as_str().unwrap().into(),
                tool_call_id: None,
                tool_calls: None,
                images: None,
                cache_control: None,
            }],
            vec![],
        ))
    }
}
struct NoTools;
#[async_trait::async_trait]
impl CapabilityExecutor for NoTools {
    async fn execute(
        &self,
        _: TurnExecutionInput,
        _: Vec<LlmToolCall>,
        _: mpsc::Sender<
            Result<crate::server::api::run::runtime::RunEventEnvelope, std::convert::Infallible>,
        >,
    ) -> Vec<(ToolResult, Value)> {
        panic!("Controlled workflow provider does not request tools")
    }
}

fn fixture(
    behavior: ProviderBehavior,
) -> (
    WorkflowEngine,
    Arc<ControlledProvider>,
    mpsc::UnboundedReceiver<()>,
) {
    let db = sqlite::open(":memory:").unwrap();
    let (started, receiver) = mpsc::unbounded_channel();
    let provider = Arc::new(ControlledProvider {
        behavior,
        requests: Mutex::new(vec![]),
        started,
    });
    let router = cade_ai::LlmRouter::build(&cade_ai::AiConfig {
        anthropic_api_key: None,
        openai_api_key: None,
        google_api_key: None,
        deepseek_api_key: None,
        ollama_base_url: String::new(),
        llm_provider: String::new(),
    });
    let config = Arc::new(ServerConfig {
        addr: "127.0.0.1:0".parse().unwrap(),
        db_path: ":memory:".into(),
        llm_provider: LlmProviderKind::Anthropic,
        default_model: String::new(),
        anthropic_api_key: None,
        openai_api_key: None,
        google_api_key: None,
        deepseek_api_key: None,
        ollama_base_url: String::new(),
        api_key: None,
        allowed_origin: None,
        max_context_budget: None,
        max_tokens_per_turn: Some(64_000),
    });
    let state = AppState::new_in_process(
        db.clone(),
        provider.clone(),
        Arc::new(RwLock::new(router)),
        config,
        Arc::new(crate::server::state::McpManager::empty()),
    );
    for id in ["first-agent", "second-agent", "third-agent"] {
        sqlite::create_agent(
            &db,
            &sqlite::AgentRow {
                id: id.into(),
                name: id.into(),
                model: format!("fixture/{id}"),
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
    }
    let runtime = ServerAgentRuntime::with_dependencies(
        state,
        Arc::new(StoredContext(db.clone())),
        Arc::new(NoTools),
    );
    (
        WorkflowEngine::with_runtime(db, runtime),
        provider,
        receiver,
    )
}

fn step(name: &str, agent: &str, dependencies: &[&str]) -> WorkflowStepDef {
    WorkflowStepDef {
        name: name.into(),
        agent: Some(agent.into()),
        prompt: format!("Perform {name}"),
        depends_on: dependencies.iter().map(|name| (*name).into()).collect(),
    }
}
fn workflow() -> WorkflowDef {
    WorkflowDef {
        name: "controlled-workflow".into(),
        description: "Fixture".into(),
        // Reverse declaration order proves dependency scheduling, not a for loop.
        steps: vec![
            step("third", "third-agent", &["second"]),
            step("second", "second-agent", &["first"]),
            step("first", "first-agent", &[]),
        ],
    }
}
async fn drain(mut receiver: broadcast::Receiver<WorkflowStepEvent>) -> Vec<WorkflowStepEvent> {
    tokio::time::timeout(std::time::Duration::from_secs(5), async move {
        let mut events = vec![];
        while let Ok(event) = receiver.recv().await {
            events.push(event);
        }
        events
    })
    .await
    .expect("Workflow must finish and close its stream")
}

#[tokio::test]
async fn workflow_orders_dependencies_and_reports_actual_canonical_outputs() {
    let (engine, provider, _) = fixture(ProviderBehavior::Success);
    let (accepted, receiver) = engine
        .dispatch_with_execution(workflow(), json!({"branch":"fixture-branch"}))
        .await
        .unwrap();
    let run_id = accepted.run_id;
    assert_ne!(run_id, accepted.execution_id);
    assert_eq!(
        accepted.agent_id, "first-agent",
        "Acceptance identifies the dependency root, not declaration index zero"
    );
    let first_execution = sqlite::get_run(&engine.db, &accepted.execution_id)
        .unwrap()
        .unwrap();
    assert_eq!(
        first_execution.agent_id, accepted.agent_id,
        "The acknowledged canonical execution already exists"
    );
    let events = drain(receiver).await;
    let terminal: Vec<_> = events
        .iter()
        .filter(|event| event.status == WorkflowStatus::Succeeded)
        .collect();
    assert_eq!(
        terminal
            .iter()
            .map(|event| event.step_index)
            .collect::<Vec<_>>(),
        vec![2, 1, 0]
    );
    assert_eq!(
        terminal
            .iter()
            .map(|event| event.output_chunk.as_deref().unwrap())
            .collect::<Vec<_>>(),
        vec![
            "provider-result-1",
            "provider-result-2",
            "provider-result-3"
        ]
    );
    let record = sqlite::get_workflow_run(&engine.db, &run_id)
        .unwrap()
        .unwrap();
    assert_eq!(record.status, "succeeded");
    assert!(record.completed_at.is_some());
    let requests = provider.requests.lock().await;
    assert_eq!(requests.len(), 3);
    assert_eq!(requests[0].model, "fixture/first-agent");
    assert!(requests[0].messages[0].content.contains("fixture-branch"));
    assert!(
        requests[1].messages[0]
            .content
            .contains("provider-result-1")
    );
    assert!(
        requests[2].messages[0]
            .content
            .contains("provider-result-2")
    );
    for agent in ["first-agent", "second-agent", "third-agent"] {
        let runs = sqlite::list_agent_runs(&engine.db, agent, 10).unwrap();
        assert_eq!(runs.len(), 1, "One canonical run per actual step");
        assert_eq!(runs[0].status, "done");
        assert!(
            runs[0].conversation_id.is_some(),
            "Workflow step input is isolated from the agent's shared default timeline"
        );
        assert!(
            sqlite::run_events_after(&engine.db, &runs[0].id, -1)
                .unwrap()
                .iter()
                .any(|(_, event)| event.contains("run_done"))
        );
    }
    assert!(!engine.cancel(&run_id).await.unwrap());
}

#[tokio::test]
async fn runtime_failure_fails_workflow_and_skips_remaining_steps() {
    let (engine, provider, _) = fixture(ProviderBehavior::RequestFailure);
    let (run_id, receiver) = engine.dispatch(workflow(), json!({})).await.unwrap();
    let events = drain(receiver).await;
    assert!(
        !events
            .iter()
            .any(|event| event.status == WorkflowStatus::Succeeded)
    );
    assert_eq!(
        events
            .iter()
            .filter(|event| event.status == WorkflowStatus::Skipped)
            .count(),
        2
    );
    assert!(events.iter().any(|event| {
        event.status == WorkflowStatus::Failed
            && event
                .error
                .as_deref()
                .unwrap()
                .contains("controlled provider failure")
    }));
    let record = sqlite::get_workflow_run(&engine.db, &run_id)
        .unwrap()
        .unwrap();
    assert_eq!(record.status, "failed");
    assert!(
        record
            .error
            .unwrap()
            .contains("controlled provider failure")
    );
    assert_eq!(provider.requests.lock().await.len(), 1);
    assert_eq!(
        sqlite::list_agent_runs(&engine.db, "first-agent", 1).unwrap()[0].status,
        "error"
    );
    assert!(
        sqlite::list_agent_runs(&engine.db, "second-agent", 1)
            .unwrap()
            .is_empty()
    );
}

#[tokio::test]
async fn concurrent_workflows_isolate_inputs_for_the_same_agent() {
    let (engine, provider, _) = fixture(ProviderBehavior::Success);
    let (left, left_events) = engine
        .dispatch(workflow(), json!({"payload":"left-only"}))
        .await
        .unwrap();
    let (right, right_events) = engine
        .dispatch(workflow(), json!({"payload":"right-only"}))
        .await
        .unwrap();
    let _ = tokio::join!(drain(left_events), drain(right_events));
    let requests = provider.requests.lock().await;
    let first: Vec<_> = requests
        .iter()
        .filter(|request| request.model == "fixture/first-agent")
        .collect();
    assert_eq!(first.len(), 2);
    assert_eq!(
        first
            .iter()
            .filter(|request| request.messages[0].content.contains("left-only"))
            .count(),
        1
    );
    assert_eq!(
        first
            .iter()
            .filter(|request| request.messages[0].content.contains("right-only"))
            .count(),
        1
    );
    let runs = sqlite::list_agent_runs(&engine.db, "first-agent", 10).unwrap();
    assert_ne!(runs[0].conversation_id, runs[1].conversation_id);
    for run_id in [left, right] {
        assert_eq!(
            sqlite::get_workflow_run(&engine.db, &run_id)
                .unwrap()
                .unwrap()
                .status,
            "succeeded"
        );
    }
}

#[tokio::test]
async fn stream_error_is_never_reported_as_workflow_success() {
    let (engine, _, _) = fixture(ProviderBehavior::ChunkFailure);
    let (run_id, receiver) = engine.dispatch(workflow(), json!({})).await.unwrap();
    let events = drain(receiver).await;
    assert!(
        !events
            .iter()
            .any(|event| event.status == WorkflowStatus::Succeeded)
    );
    assert_eq!(
        sqlite::get_workflow_run(&engine.db, &run_id)
            .unwrap()
            .unwrap()
            .status,
        "failed"
    );
}

#[tokio::test]
async fn cancellation_stops_canonical_runtime_without_overwriting_cancelled_status() {
    let (engine, provider, mut started) = fixture(ProviderBehavior::Pending);
    let (run_id, receiver) = engine.dispatch(workflow(), json!({})).await.unwrap();
    started.recv().await.unwrap();
    assert!(engine.cancel(&run_id).await.unwrap());
    let events = drain(receiver).await;
    assert!(
        events
            .iter()
            .any(|event| event.status == WorkflowStatus::Cancelled)
    );
    assert!(
        !events
            .iter()
            .any(|event| event.status == WorkflowStatus::Succeeded)
    );
    assert_eq!(
        events
            .iter()
            .filter(|event| event.status == WorkflowStatus::Skipped)
            .count(),
        2
    );
    assert_eq!(provider.requests.lock().await.len(), 1);
    let canonical = sqlite::list_agent_runs(&engine.db, "first-agent", 1)
        .unwrap()
        .remove(0);
    assert_eq!(canonical.status, "cancelled");
    let record = sqlite::get_workflow_run(&engine.db, &run_id)
        .unwrap()
        .unwrap();
    assert_eq!(record.status, "cancelled");
    assert!(record.completed_at.is_some());
    assert!(!engine.cancel(&run_id).await.unwrap());
}

#[tokio::test]
async fn invalid_dependencies_or_agents_are_rejected_before_any_run_starts() {
    let (engine, provider, _) = fixture(ProviderBehavior::Success);
    let mut invalid = workflow();
    invalid.steps[2].depends_on.push("third".into());
    assert!(
        engine
            .dispatch(invalid, json!({}))
            .await
            .unwrap_err()
            .contains("cycle")
    );
    let mut invalid = workflow();
    invalid.steps[2].depends_on.push("unknown".into());
    assert!(
        engine
            .dispatch(invalid, json!({}))
            .await
            .unwrap_err()
            .contains("unknown dependency")
    );
    let mut invalid = workflow();
    invalid.steps[2].agent = Some("missing-agent".into());
    assert!(
        engine
            .dispatch(invalid, json!({}))
            .await
            .unwrap_err()
            .contains("Unknown or ambiguous")
    );
    let mut invalid = workflow();
    invalid.steps[2].name = "second".into();
    assert!(engine.dispatch(invalid, json!({})).await.is_err());
    assert!(provider.requests.lock().await.is_empty());
    assert!(
        sqlite::list_workflow_runs(&engine.db, None, 10)
            .unwrap()
            .is_empty()
    );
}

#[tokio::test]
async fn runtime_setup_failure_is_not_acknowledged_as_an_execution() {
    let (mut engine, provider, _) = fixture(ProviderBehavior::Success);
    engine.runtime = engine.runtime.with_execution_options(
        crate::server::api::run::runtime::RunExecutionOptions {
            max_turns: Some(0),
            ..Default::default()
        },
    );
    let error = engine
        .dispatch_with_execution(workflow(), json!({}))
        .await
        .unwrap_err();
    assert!(error.contains("max_turns must be positive"), "{error}");
    assert!(provider.requests.lock().await.is_empty());
    assert!(
        sqlite::list_agent_runs(&engine.db, "first-agent", 10)
            .unwrap()
            .is_empty()
    );
    let records = sqlite::list_workflow_runs(&engine.db, Some("controlled-workflow"), 10).unwrap();
    assert_eq!(records.len(), 1);
    assert_eq!(records[0].status, "failed");
    assert!(records[0].completed_at.is_some());
    assert!(
        records[0]
            .error
            .as_deref()
            .unwrap()
            .contains("max_turns must be positive")
    );
}

#[tokio::test]
async fn file_discovery_get_and_dispatch_share_actual_definition() {
    let (engine, _, _) = fixture(ProviderBehavior::Success);
    let directory = tempfile::tempdir().unwrap();
    let engine = engine.with_directory(directory.path().into());
    std::fs::write(
        directory.path().join("controlled-workflow.json"),
        json!(workflow()).to_string(),
    )
    .unwrap();
    let summaries = engine.list_workflows().await.unwrap();
    let summary = summaries
        .iter()
        .find(|summary| summary.name == "controlled-workflow")
        .unwrap();
    assert_eq!(summary.steps_count, 3);
    let def = engine.definition("controlled-workflow").unwrap().unwrap();
    let (run_id, receiver) = engine.dispatch(def, json!({})).await.unwrap();
    drain(receiver).await;
    assert_eq!(
        engine
            .list_workflows()
            .await
            .unwrap()
            .into_iter()
            .find(|summary| summary.name == "controlled-workflow")
            .unwrap()
            .last_run
            .unwrap()
            .run_id,
        run_id
    );
    assert!(engine.definition("does-not-exist").unwrap().is_none());
    assert!(engine.definition("../escape").is_err());
}

#[tokio::test]
async fn legacy_webhook_config_keeps_its_agent_selected_model_and_real_outcome() {
    let (engine, provider, _) = fixture(ProviderBehavior::Success);
    let directory = tempfile::tempdir().unwrap();
    let engine = engine.with_directory(directory.path().into());
    std::fs::write(directory.path().join("legacy.json"), json!({
        "name":"Legacy", "agent":"legacy-agent", "model":"fixture/legacy-selected", "prompt":"Do legacy work"
    }).to_string()).unwrap();
    engine.prepare_legacy_agent("legacy").unwrap();
    let def = engine.definition("legacy").unwrap().unwrap();
    let (run_id, receiver) = engine
        .dispatch(def, json!({"input":"webhook"}))
        .await
        .unwrap();
    drain(receiver).await;
    assert_eq!(
        provider.requests.lock().await[0].model,
        "fixture/legacy-selected"
    );
    assert_eq!(
        sqlite::get_workflow_run(&engine.db, &run_id)
            .unwrap()
            .unwrap()
            .status,
        "succeeded"
    );
}
