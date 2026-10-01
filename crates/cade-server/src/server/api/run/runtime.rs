//! Server-owned lifecycle entry point for durable agent runs.
//!
//! HTTP/SSE routes and future in-process transports use this module to start
//! runs. The agentic loop remains in the parent module while this interface
//! owns request-side lifecycle work: activity tracking, user-message
//! persistence, run creation, global lifecycle publication, and event-channel
//! construction.

use std::path::PathBuf;
use std::sync::Arc;

use async_trait::async_trait;
use axum::response::sse::Event;
use cade_agent::backends::ExecutionBackend;
use cade_agent::tools::{manager::ToolResult, runtime::ToolRuntime};
use cade_ai::{LlmMessage, LlmToolCall};
use cade_core::settings::{ExecutionProfile, PermissionSettings, SettingsManager};
use cade_store::sqlite;
use serde_json::{Value, json};

use super::{
    SseTx, detect_theme_cmd, execution, maybe_set_conv_title, run_agent_loop_with_dependencies,
};
use crate::server::api::messages::{build_context, persist_checked};
use crate::server::state::AppState;

/// Bounded model context prepared for one agent turn.
pub(crate) type RunContext = (String, Vec<LlmMessage>, Vec<Value>);

/// Deep module used by the runtime to prepare bounded model context.
#[async_trait]
pub(crate) trait ContextBuilder: Send + Sync {
    async fn build(
        &self,
        agent_id: String,
        conversation_id: Option<String>,
        is_tool_return: bool,
    ) -> Result<RunContext, String>;
}

/// Execution parameters for one turn's tool execution.
#[derive(Debug, Clone)]
pub(crate) struct TurnExecutionInput {
    pub agent_id: String,
    pub conversation_id: Option<String>,
    pub run_id: String,
    pub input: String,
    pub permission_mode: Option<String>,
}

/// Deep module used by the runtime to execute all tool calls for one turn.
#[async_trait]
pub(crate) trait CapabilityExecutor: Send + Sync {
    async fn execute(
        &self,
        input: TurnExecutionInput,
        tool_calls: Vec<LlmToolCall>,
        events: SseTx,
    ) -> Vec<(ToolResult, Value)>;
}

#[derive(Clone)]
struct ServerContextBuilder {
    state: AppState,
}

#[async_trait]
impl ContextBuilder for ServerContextBuilder {
    async fn build(
        &self,
        agent_id: String,
        conversation_id: Option<String>,
        is_tool_return: bool,
    ) -> Result<RunContext, String> {
        Box::pin(build_context(
            self.state.clone(),
            agent_id,
            conversation_id,
            is_tool_return,
        ))
        .await
    }
}

#[derive(Clone)]
struct ServerCapabilityExecutor {
    state: AppState,
    options: Arc<ResolvedRunExecutionOptions>,
}

#[async_trait]
impl CapabilityExecutor for ServerCapabilityExecutor {
    async fn execute(
        &self,
        input: TurnExecutionInput,
        tool_calls: Vec<LlmToolCall>,
        events: SseTx,
    ) -> Vec<(ToolResult, Value)> {
        execution::execute_turn_tools_with_options(
            self.state.clone(),
            input,
            tool_calls,
            events,
            self.options.clone(),
        )
        .await
    }
}

/// Input required to start one server-owned agent run.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RunRequest {
    pub agent_id: String,
    pub conversation_id: Option<String>,
    pub input: String,
    pub permission_mode: Option<String>,
}

/// Transport-neutral overrides. Resolved exactly once when a run is accepted.
/// `max_turns` is a hard limit; omitted limits retain the adaptive default.
#[derive(Clone, Default, serde::Serialize, serde::Deserialize)]
#[serde(default)]
pub struct RunExecutionOptions {
    #[serde(alias = "workspace")]
    pub cwd: Option<PathBuf>,
    /// Client-owned Working Session, independent of Conversation identity.
    pub working_session_id: Option<String>,
    pub allowed_paths: Option<Vec<String>>,
    pub permission_mode: Option<String>,
    pub permissions: Option<PermissionSettings>,
    pub execution: Option<ExecutionProfile>,
    #[serde(skip)]
    pub backend: Option<Arc<dyn ExecutionBackend>>,
    /// In-process callers may supply the actual compatibility-accessor runtime.
    /// Its workspace, agent and conversation must match the accepted request.
    #[serde(skip)]
    pub tool_runtime: Option<Arc<ToolRuntime>>,
    pub reasoning_effort: Option<String>,
    pub max_turns: Option<usize>,
}

#[derive(Debug)]
pub struct RunStartError {
    pub status: axum::http::StatusCode,
    pub message: String,
}

impl std::fmt::Display for RunStartError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.message)
    }
}

impl std::error::Error for RunStartError {}

impl RunStartError {
    fn invalid(message: impl Into<String>) -> Self {
        Self {
            status: axum::http::StatusCode::BAD_REQUEST,
            message: message.into(),
        }
    }

    fn storage(error: impl std::fmt::Display) -> Self {
        Self {
            status: axum::http::StatusCode::INTERNAL_SERVER_ERROR,
            message: error.to_string(),
        }
    }
}

pub(crate) struct ResolvedRunExecutionOptions {
    pub cwd: PathBuf,
    pub runtime: Arc<ToolRuntime>,
    pub permissions: cade_core::permissions::PermissionManager,
    pub hooks: Arc<cade_core::hooks::HookEngine>,
    pub reasoning_effort: Option<String>,
    pub max_turns: usize,
    pub turns_ceiling: usize,
    pub session_cost_cap: Option<f64>,
    pub tool_turn_max_tokens: Option<u32>,
    pub permission_settings: PermissionSettings,
    pub hooks_config: cade_core::settings::HooksConfig,
    pub max_context_budget: Option<usize>,
    pub max_tokens_per_turn: Option<usize>,
}

impl ResolvedRunExecutionOptions {
    /// Rebind execution identity/root for an isolated child, retaining the
    /// accepted settings instead of resolving daemon/project defaults again.
    pub(crate) fn for_child(
        &self,
        runtime: Arc<ToolRuntime>,
        permissions: cade_core::permissions::PermissionManager,
        hooks: Arc<cade_core::hooks::HookEngine>,
        max_iters: usize,
        permission_settings: PermissionSettings,
    ) -> Arc<Self> {
        Arc::new(Self {
            cwd: runtime.cwd.clone(),
            runtime,
            permissions,
            hooks,
            reasoning_effort: self.reasoning_effort.clone(),
            max_turns: max_iters,
            turns_ceiling: max_iters,
            session_cost_cap: self.session_cost_cap,
            tool_turn_max_tokens: self.tool_turn_max_tokens,
            permission_settings,
            hooks_config: self.hooks_config.clone(),
            max_context_budget: self.max_context_budget,
            max_tokens_per_turn: self.max_tokens_per_turn,
        })
    }
}

tokio::task_local! {
    static EXECUTION_OPTIONS: Arc<ResolvedRunExecutionOptions>;
}

/// Task-local context is explicitly captured at every server-owned spawn.
/// It contains configuration, never the parent's cancellation ownership.
pub(crate) fn current_execution_options() -> Option<Arc<ResolvedRunExecutionOptions>> {
    EXECUTION_OPTIONS.try_with(Arc::clone).ok()
}

pub(crate) fn execution_workspace() -> PathBuf {
    current_execution_options()
        .map(|options| options.cwd.clone())
        .unwrap_or_else(|| std::env::current_dir().unwrap_or_default())
}

pub(crate) async fn in_execution_scope<F: std::future::Future>(
    options: Option<Arc<ResolvedRunExecutionOptions>>,
    work: F,
) -> F::Output {
    match options {
        Some(options) => EXECUTION_OPTIONS.scope(options, work).await,
        None => work.await,
    }
}

pub(crate) fn spawn_in_execution_scope<F>(work: F) -> tokio::task::JoinHandle<F::Output>
where
    F: std::future::Future + Send + 'static,
    F::Output: Send + 'static,
{
    let options = current_execution_options();
    tokio::spawn(in_execution_scope(options, work))
}

impl RunExecutionOptions {
    pub(crate) fn resolve(
        self,
        state: &AppState,
        request: &RunRequest,
    ) -> Result<Arc<ResolvedRunExecutionOptions>, RunStartError> {
        let cwd = match self.cwd {
            Some(cwd) => cwd,
            None => std::env::current_dir().map_err(RunStartError::storage)?,
        };
        let cwd = cwd
            .canonicalize()
            .map_err(|error| RunStartError::invalid(format!("Invalid workspace: {error}")))?;
        if !cwd.is_dir() {
            return Err(RunStartError::invalid("Workspace must be a directory"));
        }
        if self.max_turns == Some(0) {
            return Err(RunStartError::invalid("max_turns must be positive"));
        }
        let settings = SettingsManager::new(&cwd).map_err(RunStartError::storage)?;
        let permission_settings = self
            .permissions
            .unwrap_or_else(|| settings.permission_settings().clone());
        let mode = request
            .permission_mode
            .as_deref()
            .or(self.permission_mode.as_deref())
            .map(|mode| {
                execution::parse_permission_mode(mode).ok_or_else(|| {
                    RunStartError::invalid(format!("Invalid permission_mode: {mode}"))
                })
            })
            .transpose()?
            .unwrap_or_default();
        let mut permissions = cade_core::permissions::PermissionManager::new_with_strict_bash(
            mode,
            permission_settings.strict_bash,
        );
        for (rules, deny) in [
            (&permission_settings.allow, false),
            (&permission_settings.deny, true),
        ] {
            for rule in rules {
                let rule = cade_core::permissions::PermissionRule::parse(rule)
                    .ok_or_else(|| RunStartError::invalid("Invalid permission rule"))?;
                if deny {
                    permissions.add_deny_rule(rule);
                } else {
                    permissions.add_allow_rule(rule);
                }
            }
        }
        if let Some(id) = self.working_session_id.as_deref() {
            let grants = state
                .permission_sessions
                .for_run(id, &cwd)
                .map_err(RunStartError::invalid)?;
            permissions = permissions.with_session_grants(grants);
        }
        let backend = if let Some(runtime) = self.tool_runtime.as_ref() {
            if runtime.cwd != cwd
                || runtime.agent_id != request.agent_id
                || runtime.conversation_id != request.conversation_id
            {
                return Err(RunStartError::invalid(
                    "Tool runtime does not belong to this run workspace/conversation",
                ));
            }
            if self.backend.is_some() || self.execution.is_some() || self.allowed_paths.is_some() {
                return Err(RunStartError::invalid(
                    "Configure backend and allowed paths on the supplied tool runtime",
                ));
            }
            runtime.backend.clone()
        } else if let Some(backend) = self.backend {
            backend
        } else {
            let profile = self
                .execution
                .unwrap_or_else(|| settings.execution_profile().clone());
            match profile.backend {
                cade_core::settings::ExecutionBackendKind::Virtual => Arc::new(
                    cade_agent::backends::VirtualSandboxBackend::new(cwd.clone()),
                )
                    as Arc<dyn ExecutionBackend>,
                // The legacy factory uses process cwd for MicroVM. Do not silently run elsewhere.
                cade_core::settings::ExecutionBackendKind::MicroVm => {
                    return Err(RunStartError::invalid(
                        "MicroVM requires an explicit workspace-bound backend",
                    ));
                }
                _ => {
                    let backend: Arc<dyn ExecutionBackend> =
                        Arc::from(cade_agent::backends::backend_from_profile(&profile));
                    if matches!(
                        profile.backend,
                        cade_core::settings::ExecutionBackendKind::Docker
                            | cade_core::settings::ExecutionBackendKind::Ssh
                    ) && backend.name() == "local"
                    {
                        return Err(RunStartError::invalid(
                            "Requested execution backend is unavailable",
                        ));
                    }
                    backend
                }
            }
        };
        let runtime = if let Some(runtime) = self.tool_runtime {
            runtime
        } else {
            let mut runtime = ToolRuntime::new(
                Arc::new(super::storage_impl::ServerStorageBackend {
                    state: state.clone(),
                }),
                state.mcp.clone(),
                request.agent_id.clone(),
                cwd.clone(),
            )
            .with_backend(backend)
            .with_conversation(request.conversation_id.clone())
            .with_extension(Arc::new(super::plugin_execution::ServerPluginTools::new(
                cwd.clone(),
                state.mcp.clone(),
            )));
            runtime.allowed_paths = self.allowed_paths.map(|paths| {
                paths
                    .into_iter()
                    .map(|path| {
                        let path = PathBuf::from(path);
                        let path = if path.is_absolute() {
                            path
                        } else {
                            cwd.join(path)
                        };
                        path.to_string_lossy().into_owned()
                    })
                    .collect()
            });
            Arc::new(runtime)
        };
        let max_turns = self.max_turns.unwrap_or_else(super::max_turns);
        let hooks_config = settings.merged_hooks();
        Ok(Arc::new(ResolvedRunExecutionOptions {
            hooks: Arc::new(cade_core::hooks::HookEngine::new(
                hooks_config.clone(),
                cwd.clone(),
                request.agent_id.clone(),
            )),
            cwd,
            runtime,
            permissions,
            reasoning_effort: self
                .reasoning_effort
                .or_else(|| settings.reasoning_effort()),
            max_turns,
            turns_ceiling: if self.max_turns.is_some() {
                max_turns
            } else {
                super::turns_ceiling(max_turns)
            },
            session_cost_cap: super::max_session_cost_usd(settings.max_session_cost_usd()),
            tool_turn_max_tokens: super::tool_turn_max_tokens(),
            permission_settings,
            hooks_config,
            max_context_budget: settings.max_context_budget(),
            max_tokens_per_turn: settings.max_tokens_per_turn(),
        }))
    }
}

/// Internal loop input derived from an accepted runtime request.
pub(crate) struct LoopRequest {
    pub agent_id: String,
    pub conversation_id: Option<String>,
    pub run_id: String,
    pub theme_command: Option<String>,
    pub input: String,
    pub permission_mode: Option<String>,
    pub options: Arc<ResolvedRunExecutionOptions>,
}

/// Ordered run event envelope emitted by the canonical runtime.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RunEventEnvelope {
    pub data: String,
}

impl From<RunEventEnvelope> for Event {
    fn from(env: RunEventEnvelope) -> Self {
        Event::default().data(env.data)
    }
}

/// Handle returned when a durable agent run has been accepted.
#[derive(Debug)]
pub struct RunHandle {
    pub run_id: String,
    pub events: tokio::sync::mpsc::Receiver<Result<RunEventEnvelope, std::convert::Infallible>>,
}

/// Small server-owned interface for beginning the canonical agentic loop.
///
/// The runtime owns durable run setup and the loop task. Transports own only
/// how they expose the returned ordered event receiver to their callers.
#[derive(Clone)]
pub struct ServerAgentRuntime {
    state: AppState,
    context_builder: Arc<dyn ContextBuilder>,
    capability_executor: Option<Arc<dyn CapabilityExecutor>>,
    execution_options: RunExecutionOptions,
}

impl ServerAgentRuntime {
    pub fn new(state: AppState) -> Self {
        Self {
            context_builder: Arc::new(ServerContextBuilder {
                state: state.clone(),
            }),
            capability_executor: None,
            execution_options: RunExecutionOptions::default(),
            state,
        }
    }

    #[cfg(test)]
    pub(crate) fn with_dependencies(
        state: AppState,
        context_builder: Arc<dyn ContextBuilder>,
        capability_executor: Arc<dyn CapabilityExecutor>,
    ) -> Self {
        Self {
            state,
            context_builder,
            capability_executor: Some(capability_executor),
            execution_options: RunExecutionOptions::default(),
        }
    }

    /// Persist the request, create a durable run, and begin the agentic loop.
    pub async fn start(&self, request: RunRequest) -> RunHandle {
        match self.try_start(request).await {
            Ok(handle) => handle,
            Err(error) => {
                // Compatibility entrypoint: reject before spawning execution; no fictitious id.
                let (tx, events) = tokio::sync::mpsc::channel(3);
                for payload in [
                    json!({"message_type":"error", "error":error.message}),
                    json!({"message_type":"run_done", "status":"error"}),
                ] {
                    let _ = tx.try_send(Ok(RunEventEnvelope {
                        data: payload.to_string(),
                    }));
                }
                let _ = tx.try_send(Ok(RunEventEnvelope {
                    data: "[DONE]".into(),
                }));
                RunHandle {
                    run_id: String::new(),
                    events,
                }
            }
        }
    }

    pub fn with_execution_options(mut self, options: RunExecutionOptions) -> Self {
        self.execution_options = options;
        self
    }

    pub async fn try_start(&self, request: RunRequest) -> Result<RunHandle, RunStartError> {
        self.start_with_options(request, self.execution_options.clone())
            .await
    }

    /// Prepare the same tool runtime that an in-process session exposes publicly.
    /// Supply it through `RunExecutionOptions::tool_runtime` to execute through it.
    pub fn prepare_tool_runtime(
        &self,
        request: &RunRequest,
        options: RunExecutionOptions,
    ) -> Result<Arc<ToolRuntime>, RunStartError> {
        Ok(options.resolve(&self.state, request)?.runtime.clone())
    }

    /// Validate ownership and configuration before accepting any durable work.
    pub async fn start_with_options(
        &self,
        request: RunRequest,
        options: RunExecutionOptions,
    ) -> Result<RunHandle, RunStartError> {
        let agent =
            sqlite::get_agent(&self.state.db, &request.agent_id).map_err(RunStartError::storage)?;
        if agent.is_none() {
            return Err(RunStartError {
                status: axum::http::StatusCode::NOT_FOUND,
                message: "Agent not found".into(),
            });
        }
        if let Some(id) = request.conversation_id.as_deref() {
            match sqlite::get_conversation(&self.state.db, id).map_err(RunStartError::storage)? {
                Some(conversation) if conversation.agent_id == request.agent_id => {}
                _ => {
                    return Err(RunStartError {
                        status: axum::http::StatusCode::NOT_FOUND,
                        message: "Conversation not found for agent".into(),
                    });
                }
            }
        }
        let options = options.resolve(&self.state, &request)?;
        let run_id = sqlite::create_run(
            &self.state.db,
            &request.agent_id,
            request.conversation_id.as_deref(),
        )
        .map_err(RunStartError::storage)?
        .id;
        let theme_cmd = detect_theme_cmd(&request.input);
        if theme_cmd.is_none() && !request.input.is_empty() {
            if let Err(error) = persist_checked(
                &self.state,
                &request.agent_id,
                request.conversation_id.as_deref(),
                "user",
                json!({"content": request.input}),
            ) {
                let _ = sqlite::finish_run(&self.state.db, &run_id, "error");
                return Err(RunStartError::storage(error));
            }
            if let Some(id) = request.conversation_id.as_deref() {
                maybe_set_conv_title(&self.state, id, &request.input);
            }
        }
        update_activity(
            &self.state,
            &request.agent_id,
            request.conversation_id.clone(),
        )
        .await;

        crate::server::api::agents::publish_global_event(
            Some(&self.state.db),
            "run_started",
            json!({
                "run_id": run_id,
                "agent_id": request.agent_id,
                "conversation_id": request.conversation_id,
            }),
        );

        let (events, receiver) = tokio::sync::mpsc::channel(128);
        tokio::spawn(in_execution_scope(
            Some(options.clone()),
            run_agent_loop_with_dependencies(
                self.state.clone(),
                LoopRequest {
                    agent_id: request.agent_id,
                    conversation_id: request.conversation_id,
                    run_id: run_id.clone(),
                    theme_command: theme_cmd,
                    input: request.input,
                    permission_mode: request.permission_mode,
                    options: options.clone(),
                },
                events,
                self.context_builder.clone(),
                self.capability_executor.clone().unwrap_or_else(|| {
                    Arc::new(ServerCapabilityExecutor {
                        state: self.state.clone(),
                        options,
                    })
                }),
            ),
        ));

        Ok(RunHandle {
            run_id,
            events: receiver,
        })
    }
}

/// Record that the agent is active and update its conversation pointer.
async fn update_activity(state: &AppState, agent_id: &str, conversation_id: Option<String>) {
    let mut activity = state.agent_activity.write().await;
    let entry =
        activity
            .entry(agent_id.to_owned())
            .or_insert(crate::server::state::AgentActivity {
                last_active_ts: 0,
                needs_consolidation: false,
                conversation_id: conversation_id.clone(),
                last_consolidation_turn: 0,
                last_omitted_turns: 0,
            });
    entry.last_active_ts = chrono::Utc::now().timestamp();
    entry.conversation_id = conversation_id;
}

/// Explicit durable cancellation, independent of transport disconnects and child runs.
pub(crate) async fn cancellation_requested(db: &sqlite::Db, run_id: &str) {
    let mut interval = tokio::time::interval(std::time::Duration::from_millis(50));
    loop {
        interval.tick().await;
        match sqlite::is_run_cancellation_requested(db, run_id) {
            Ok(false) => {}
            Ok(true) => return,
            Err(error) => {
                tracing::error!(%run_id, %error, "cannot verify run cancellation; stopping work");
                return;
            }
        }
    }
}

pub(crate) async fn until_cancelled<T>(
    db: &sqlite::Db,
    run_id: &str,
    work: impl std::future::Future<Output = T>,
) -> Option<T> {
    tokio::select! {
        biased;
        _ = cancellation_requested(db, run_id) => None,
        result = work => Some(result),
    }
}
