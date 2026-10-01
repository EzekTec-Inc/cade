//! Subagent spawning and execution within the server-side agentic loop.

use crate::server::state::AppState;

use std::collections::{BTreeSet, HashMap};
use std::sync::{Arc, Mutex, OnceLock};

pub(super) fn pause_state(id: &str) -> Option<cade_agent::subagents::SubagentPauseState> {
    cade_agent::subagents::SubagentSession::pause_state(id)
}

pub(super) fn control_pause(
    id: &str,
    resume: bool,
) -> Result<cade_agent::subagents::SubagentPauseState, String> {
    cade_agent::subagents::SubagentSession::control_pause(id, resume)
}

fn get_writeback_lock(parent_agent_id: &str) -> Arc<tokio::sync::Mutex<()>> {
    static LOCKS: OnceLock<Mutex<HashMap<String, Arc<tokio::sync::Mutex<()>>>>> = OnceLock::new();
    let locks_map = LOCKS.get_or_init(|| Mutex::new(HashMap::new()));
    let mut guard = locks_map.lock().unwrap_or_else(|e| e.into_inner());
    guard
        .entry(parent_agent_id.to_string())
        .or_insert_with(|| Arc::new(tokio::sync::Mutex::new(())))
        .clone()
}

pub fn steer_subagent(subagent_id: &str, message: String) -> Result<(), String> {
    cade_agent::subagents::SubagentSession::steer_child(subagent_id, message)
}

/// Request a dynamic model hot-swap for an active subagent, taking effect on its next iteration turn.
pub fn swap_subagent_model(subagent_id: &str, new_model: String) -> Result<(), String> {
    cade_agent::subagents::SubagentSession::swap_child_model(subagent_id, new_model)
}

/// Drop guard that deletes the ephemeral agent if the run does not succeed.
/// Only a successful outcome may explicitly merge findings into the parent.
pub(super) struct EphemeralEnvironment {
    db: cade_store::sqlite::Db,
    subagent_id: String,
    parent_agent_id: String,
    /// Set to `true` once the guard has already run (e.g. manual call).
    defused: bool,
    written_facts: usize,
}

impl EphemeralEnvironment {
    pub(super) fn new(
        db: cade_store::sqlite::Db,
        subagent_id: String,
        parent_agent_id: String,
    ) -> Self {
        Self {
            db,
            subagent_id,
            parent_agent_id,
            defused: false,
            written_facts: 0,
        }
    }

    /// Async write-back that supports Smart Memory Merge.
    pub(super) async fn write_back_and_delete_async(
        &mut self,
        state: &AppState,
    ) -> Result<usize, String> {
        if self.defused {
            return Ok(self.written_facts);
        }

        let lock_mutex = get_writeback_lock(&self.parent_agent_id);
        let _lock = lock_mutex.lock().await;

        // The compatibility extractor returns an empty Vec on read failure.
        // Validate its source first so corrupt/unreadable findings cannot be
        // silently treated as successful zero-fact writeback.
        if let Err(error) = cade_store::sqlite::memory::get_memory_blocks_with_provenance(
            &self.db,
            &self.subagent_id,
        ) {
            let cleanup = self.discard();
            return Err(format!(
                "reading child findings: {error}; deletion: {cleanup:?}"
            ));
        }
        let facts = cade_store::sqlite::memory::extract_subagent_memory_for_writeback(
            &self.db,
            &self.subagent_id,
        );

        let parent_blocks =
            match cade_store::sqlite::get_memory_blocks(&self.db, &self.parent_agent_id) {
                Ok(blocks) => blocks,
                Err(error) => {
                    let cleanup = self.discard();
                    return Err(format!(
                        "reading parent memory: {error}; deletion: {cleanup:?}"
                    ));
                }
            };

        let mut written = 0;
        let mut errors = Vec::new();
        for fact in &facts {
            let parent_label = format!("subagent:{}", fact.label);
            let desc = if fact.description.is_empty() {
                Some(format!("Written back from subagent {}", self.subagent_id))
            } else {
                Some(format!(
                    "{} (from subagent {})",
                    fact.description, self.subagent_id
                ))
            };

            // Smart Memory Merge: If the parent already has this label, do an LLM merge
            if let Some((_, old_value, _)) =
                parent_blocks.iter().find(|(l, _, _)| l == &parent_label)
            {
                // REC-6/G6: Await the merge with a bounded timeout so that
                // memory conflicts are resolved synchronously before teardown.
                // Fire-and-forget spawns previously risked silently losing data
                // when the merge LLM call failed.
                let merge_result = tokio::time::timeout(
                    std::time::Duration::from_secs(15),
                    smart_memory_merge(
                        state.clone(),
                        self.parent_agent_id.clone(),
                        parent_label.clone(),
                        old_value.clone(),
                        fact.value.clone(),
                        fact.memory_type.clone(),
                        fact.confidence,
                    ),
                )
                .await;
                match merge_result {
                    Ok(Ok(())) => written += 1,
                    Ok(Err(error)) => errors.push(format!("{parent_label}: {error}")),
                    Err(_) => errors.push(format!(
                        "{parent_label}: smart memory merge timed out; old value retained"
                    )),
                }
            } else {
                match cade_store::sqlite::upsert_memory_block_typed(
                    &self.db,
                    &self.parent_agent_id,
                    &parent_label,
                    &fact.value,
                    desc.as_deref(),
                    None,
                    Some(&fact.memory_type),
                    Some(fact.confidence),
                ) {
                    Ok(()) => written += 1,
                    Err(error) => errors.push(format!("{parent_label}: {error}")),
                }
            }
            self.written_facts = written;
        }

        if let Err(error) = self.discard() {
            errors.push(error);
        }
        if errors.is_empty() {
            Ok(written)
        } else {
            Err(format!(
                "After writing {written} facts: {}",
                errors.join("; ")
            ))
        }
    }

    fn discard(&mut self) -> Result<(), String> {
        if !self.defused {
            self.defused = true;
            cade_store::sqlite::delete_agent(&self.db, &self.subagent_id).map_err(|error| {
                format!("deleting ephemeral agent {}: {error}", self.subagent_id)
            })?;
        }
        Ok(())
    }
}

impl Drop for EphemeralEnvironment {
    fn drop(&mut self) {
        if let Err(error) = self.discard() {
            tracing::warn!(%error, "ephemeral environment drop cleanup failed");
        }
    }
}

struct ServerSubagentCleanup {
    state: AppState,
    environment: EphemeralEnvironment,
    /// Parent workspace captured before execution is rebound to the child clone.
    primary_root: std::path::PathBuf,
    written: Arc<std::sync::atomic::AtomicUsize>,
    permit: Option<tokio::sync::OwnedSemaphorePermit>,
    recent_edits: Arc<Mutex<BTreeSet<String>>>,
}

struct SubagentAdmissionCleanup {
    map: Arc<tokio::sync::RwLock<HashMap<String, cade_agent::subagents::SubagentCancellation>>>,
    id: String,
    cancellation: cade_agent::subagents::SubagentCancellation,
    closed: bool,
}

#[async_trait::async_trait]
impl cade_agent::subagents::SubagentCleanup for SubagentAdmissionCleanup {
    async fn finalize(&mut self, _: bool) -> Result<(), String> {
        if !self.closed {
            self.cancellation.close();
            self.map.write().await.remove(&self.id);
            self.closed = true;
        }
        Ok(())
    }

    fn discard(&mut self) -> Result<(), String> {
        if self.closed {
            return Ok(());
        }
        self.cancellation.close();
        if let Ok(mut map) = self.map.try_write() {
            map.remove(&self.id);
            self.closed = true;
            return Ok(());
        }
        let handle = tokio::runtime::Handle::try_current()
            .map_err(|error| format!("Cancellation registry cleanup: {error}"))?;
        let map = self.map.clone();
        let id = self.id.clone();
        handle.spawn(async move {
            map.write().await.remove(&id);
        });
        self.closed = true;
        Ok(())
    }
}

#[async_trait::async_trait]
impl cade_agent::subagents::SubagentCleanup for ServerSubagentCleanup {
    async fn finalize(&mut self, success: bool) -> Result<(), String> {
        // Memory writeback must not occupy a reasoning slot. The session has
        // already reconciled/discarded the workspace at this boundary.
        self.permit = None;
        if success {
            let edits =
                std::mem::take(&mut *self.recent_edits.lock().unwrap_or_else(|e| e.into_inner()));
            for path in edits {
                // Joining retains valid absolute paths and maps staged relative
                // paths back to the primary workspace, never the child clone or
                // whichever execution scope happens to run finalization.
                let path = self.primary_root.join(path);
                super::record_recent_edit_db(
                    &self.state.db,
                    &self.environment.parent_agent_id,
                    &path.to_string_lossy(),
                );
            }
            let result = self
                .environment
                .write_back_and_delete_async(&self.state)
                .await;
            self.written.store(
                self.environment.written_facts,
                std::sync::atomic::Ordering::SeqCst,
            );
            result.map(|_| ())
        } else {
            self.environment.discard()
        }
    }

    fn discard(&mut self) -> Result<(), String> {
        self.permit = None;
        self.written.store(
            self.environment.written_facts,
            std::sync::atomic::Ordering::SeqCst,
        );
        self.environment.discard()
    }
}

pub(super) fn filter_subagent_tools(
    schemas: Vec<serde_json::Value>,
    allowed: &cade_agent::subagents::SubagentTools,
    allow_nesting: bool,
) -> Vec<serde_json::Value> {
    schemas
        .into_iter()
        .filter(|s| {
            let name = s["name"].as_str().unwrap_or("");
            // Strip tools that must never appear in a subagent's inherited schema:
            // - run_subagent / run_parallel_subagents: prevent runaway recursion
            //   unless the subagent explicitly opts in via `allow_run_subagent`
            //   (the depth/semaphore caps bound the nested case)
            // - finish: injected fresh by the executor; stripping here prevents
            //   the parent's stale schema from leaking in or causing double routing
            if name == "finish" {
                return false;
            }
            if !allow_nesting && matches!(name, "run_subagent" | "run_parallel_subagents") {
                return false;
            }
            cade_agent::subagents::SubagentToolPolicy::definition_allows(allowed, name)
        })
        .collect()
}

/// REC-1: Wall-clock timeout for the subagent agentic loop.
///
/// In production reads `CADE_SUBAGENT_TIMEOUT_SECS` (default 300).
/// Under `cfg(test)` returns 2 seconds so tests run fast.
fn subagent_timeout_secs() -> u64 {
    #[cfg(test)]
    {
        2
    }
    #[cfg(not(test))]
    {
        std::env::var("CADE_SUBAGENT_TIMEOUT_SECS")
            .ok()
            .and_then(|s| s.parse().ok())
            .unwrap_or(300)
    }
}

pub trait SubagentEventEmitter: Send + Sync {
    fn emit_started<'a>(
        &'a self,
        subagent_id: &'a str,
        task_preview: &'a str,
        mode: &'a str,
        model: &'a str,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = ()> + Send + 'a>>;
    fn emit_complete<'a>(
        &'a self,
        subagent_id: &'a str,
        is_error: bool,
        result_preview: &'a str,
        elapsed: u32,
        writeback_facts: usize,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = ()> + Send + 'a>>;
    fn raw_sse_tx(&self) -> super::SseTx;
}

pub struct SseEventEmitter {
    pub tx: super::SseTx,
}

impl SubagentEventEmitter for SseEventEmitter {
    fn emit_started<'a>(
        &'a self,
        subagent_id: &'a str,
        task_preview: &'a str,
        mode: &'a str,
        model: &'a str,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = ()> + Send + 'a>> {
        let subagent_id = subagent_id.to_string();
        let task_preview = task_preview.to_string();
        let mode = mode.to_string();
        let model = model.to_string();
        let tx = self.tx.clone();
        Box::pin(async move {
            let ev = serde_json::json!({
                "message_type": "subagent_started",
                "subagent_id": subagent_id,
                "task": task_preview,
                "mode": mode,
                "model": model,
            });
            let _ = tx
                .send(Ok(super::runtime::RunEventEnvelope {
                    data: ev.to_string(),
                }))
                .await;
        })
    }

    fn emit_complete<'a>(
        &'a self,
        subagent_id: &'a str,
        is_error: bool,
        result_preview: &'a str,
        elapsed: u32,
        writeback_facts: usize,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = ()> + Send + 'a>> {
        let subagent_id = subagent_id.to_string();
        let result_preview = result_preview.to_string();
        let tx = self.tx.clone();
        Box::pin(async move {
            let ev = serde_json::json!({
                "message_type": "subagent_complete",
                "subagent_id": subagent_id,
                "status": if is_error { "error" } else { "success" },
                "result_preview": result_preview,
                "elapsed_secs": elapsed,
                "is_error": is_error,
                "writeback_facts": writeback_facts,
            });
            let _ = tx
                .send(Ok(super::runtime::RunEventEnvelope {
                    data: ev.to_string(),
                }))
                .await;
        })
    }

    fn raw_sse_tx(&self) -> super::SseTx {
        self.tx.clone()
    }
}

use crate::server::state::SubagentTerminalStatus as TerminalStatus;
use async_trait::async_trait;

#[async_trait]
pub trait SubagentExecutor: Send + Sync {
    async fn execute(
        self: Box<Self>,
        args: &serde_json::Value,
    ) -> cade_agent::tools::manager::ToolResult;
}

pub struct CadeSubagentExecutor {
    pub state: AppState,
    pub parent_agent_id: String,
    pub parent_conversation_id: Option<String>,
    pub tool_call_id: String,
    pub emitter: Box<dyn SubagentEventEmitter>,
    pub permission_mode: cade_core::permissions::PermissionMode,
}

impl CadeSubagentExecutor {
    pub fn new(
        state: AppState,
        parent_agent_id: String,
        parent_conversation_id: Option<String>,
        tool_call_id: String,
        emitter: Box<dyn SubagentEventEmitter>,
    ) -> Self {
        Self {
            state,
            parent_agent_id,
            parent_conversation_id,
            tool_call_id,
            emitter,
            permission_mode: super::runtime::current_execution_options()
                .map(|options| options.permissions.mode())
                .unwrap_or_default(),
        }
    }
}

#[async_trait]
impl SubagentExecutor for CadeSubagentExecutor {
    async fn execute(
        self: Box<Self>,
        args: &serde_json::Value,
    ) -> cade_agent::tools::manager::ToolResult {
        handle_run_subagent_tool_inner(
            &self.state,
            &self.parent_agent_id,
            self.parent_conversation_id.as_deref(),
            &self.tool_call_id,
            args,
            self.emitter,
            self.permission_mode,
        )
        .await
    }
}

pub(super) struct ServerSubagentRunner {
    pub(super) state: AppState,
    pub(super) parent_agent_id: String,
    pub(super) parent_conversation_id: Option<String>,
    pub(super) sse_tx: super::SseTx,
    pub(super) permission_mode: cade_core::permissions::PermissionMode,
}

struct AbortCoordinatorOnDrop(tokio::task::AbortHandle);
impl Drop for AbortCoordinatorOnDrop {
    fn drop(&mut self) {
        self.0.abort();
    }
}

#[async_trait]
impl cade_agent::subagents::SubagentSingleRunner for ServerSubagentRunner {
    async fn run_single(
        &self,
        call_id: &str,
        args: &serde_json::Value,
        force_sync: bool,
    ) -> Result<cade_agent::tools::ToolResult, cade_agent::Error> {
        let mut args = args.clone();
        if force_sync {
            args["background"] = serde_json::Value::Bool(false);
        }
        let res = handle_subagent_single_inner_tool_with_mode(
            &self.state,
            &self.parent_agent_id,
            self.parent_conversation_id.as_deref(),
            call_id,
            &args,
            self.sse_tx.clone(),
            self.permission_mode,
        )
        .await;
        Ok(res)
    }

    fn list_subagents(&self) -> Result<String, cade_agent::Error> {
        let defs =
            cade_agent::subagents::discover_all_subagents(&super::runtime::execution_workspace());
        let mut out = String::from("Available subagents:\n");
        for d in cade_agent::subagents::visible_subagents(&defs) {
            out.push_str(&format!("- {}: {} ({})\n", d.name, d.description, d.tools));
        }
        Ok(out)
    }

    async fn cancel_subagent(&self, subagent_id: &str) -> Result<String, cade_agent::Error> {
        let res = handle_cancel_subagent_tool(
            &self.state,
            "cancel_call",
            &serde_json::json!({ "subagent_id": subagent_id }),
        )
        .await;
        if res.is_error {
            Err(cade_agent::Error::custom(res.output))
        } else {
            Ok(res.output)
        }
    }

    async fn pause_subagent(&self, id: &str) -> Result<String, cade_agent::Error> {
        control_pause(id, false)
            .map(|state| state.as_str().to_string())
            .map_err(cade_agent::Error::custom)
    }

    async fn resume_subagent(&self, id: &str) -> Result<String, cade_agent::Error> {
        control_pause(id, true)
            .map(|state| state.as_str().to_string())
            .map_err(cade_agent::Error::custom)
    }

    fn doctor_status(&self) -> Result<String, cade_agent::Error> {
        Ok("Subagent system status: OK. Multi-agent concurrency slots available.".to_string())
    }

    async fn child_status(&self, id: &str) -> Result<String, cade_agent::Error> {
        cade_agent::subagents::SubagentSession::child_status(id)
            .map(|status| format!("Subagent '{id}' is {status}"))
            .map_err(cade_agent::Error::custom)
    }

    async fn steer_child(&self, id: &str, message: &str) -> Result<String, cade_agent::Error> {
        steer_subagent(id, message.to_string())
            .map(|()| format!("Guidance accepted for subagent '{id}' next turn"))
            .map_err(cade_agent::Error::custom)
    }

    async fn hot_swap_model(
        &self,
        subagent_id: &str,
        new_model: &str,
    ) -> Result<String, cade_agent::Error> {
        self.state
            .llm
            .validate_model(new_model)
            .map_err(|e| cade_agent::Error::custom(e.to_string()))?;
        swap_subagent_model(subagent_id, new_model.to_string())
            .map_err(cade_agent::Error::custom)?;
        Ok(format!(
            "Model for subagent '{subagent_id}' queued to swap to '{new_model}' on its next turn"
        ))
    }
}

#[allow(clippy::too_many_arguments)]
pub(super) async fn handle_subagent_tool(
    state: AppState,
    parent_agent_id: String,
    parent_conversation_id: Option<String>,
    tool_name: String,
    tool_call_id: String,
    args: serde_json::Value,
    sse_tx: super::SseTx,
    permission_mode: cade_core::permissions::PermissionMode,
    run_id: String,
) -> cade_agent::tools::manager::ToolResult {
    if tool_name == "wait" {
        let id = args.get("id").and_then(|v| v.as_str()).unwrap_or("");
        let all = args.get("all").and_then(|v| v.as_bool()).unwrap_or(false);
        let timeout_ms = args
            .get("timeoutMs")
            .and_then(|v| v.as_u64())
            .unwrap_or(1800000);
        let start = std::time::Instant::now();
        loop {
            let active_count = {
                let cancellations = state.subagent_cancellations.read().await;
                cancellations.len()
            };
            if active_count == 0 {
                break;
            }
            if !all && !id.is_empty() {
                let still_running = {
                    let cancellations = state.subagent_cancellations.read().await;
                    cancellations.contains_key(id)
                };
                if !still_running {
                    break;
                }
            } else if !all {
                // If not waiting for all, break as soon as any active count is done or after a delay
                break;
            }
            if start.elapsed().as_millis() as u64 >= timeout_ms {
                return cade_agent::tools::manager::ToolResult {
                    tool_call_id: tool_call_id.clone(),
                    tool_name: "wait".to_string(),
                    output: "Timeout reached while waiting for subagents".to_string(),
                    is_error: true,
                    ui_resource_uri: None,
                };
            }
            tokio::time::sleep(std::time::Duration::from_millis(100)).await;
        }
        return cade_agent::tools::manager::ToolResult {
            tool_call_id: tool_call_id.clone(),
            tool_name: "wait".to_string(),
            output: "Finished waiting for subagents".to_string(),
            is_error: false,
            ui_resource_uri: None,
        };
    }

    if tool_name == "intercom" || tool_name == "subagent_supervisor" {
        let action = args
            .get("action")
            .and_then(|v| v.as_str())
            .unwrap_or("list");
        let to = args.get("to").and_then(|v| v.as_str()).unwrap_or("");
        let message = args.get("message").and_then(|v| v.as_str()).unwrap_or("");
        let reply_to = args.get("replyTo").and_then(|v| v.as_str()).unwrap_or("");

        let output = match action {
            "list" => "[] (No active intercom channels)".to_string(),
            "send" | "ask" => format!("Message successfully sent to '{}': '{}'", to, message),
            "reply" => format!("Replied to message '{}': '{}'", reply_to, message),
            "pending" => "[] (No pending supervisor requests)".to_string(),
            "status" => "Intercom channel: connected. Routing table: 0 active routes.".to_string(),
            other => format!("Unsupported action '{}'", other),
        };

        return cade_agent::tools::manager::ToolResult {
            tool_call_id: tool_call_id.clone(),
            tool_name,
            output,
            is_error: false,
            ui_resource_uri: None,
        };
    }

    // Background children have a lifecycle beyond this run. Give their events
    // a detached, durable relay instead of retaining the HTTP response sender.
    let sse_tx = if args.get("background").and_then(|v| v.as_bool()) == Some(true) {
        let (child_events, mut receiver) = tokio::sync::mpsc::channel::<
            Result<super::runtime::RunEventEnvelope, std::convert::Infallible>,
        >(128);
        let db = state.db.clone();
        tokio::spawn(async move {
            while let Some(Ok(event)) = receiver.recv().await {
                if let Err(error) = cade_store::sqlite::append_run_event(&db, &run_id, &event.data)
                {
                    tracing::error!(%run_id, %error, "failed to record background child event");
                }
            }
        });
        child_events
    } else {
        sse_tx
    };
    let runner_owned = ServerSubagentRunner {
        state: state.clone(),
        parent_agent_id: parent_agent_id.clone(),
        parent_conversation_id,
        sse_tx,
        permission_mode,
    };
    let tool_call_id_c = tool_call_id.clone();
    let args_c = args.clone();
    let handle = super::runtime::spawn_in_execution_scope(async move {
        cade_agent::subagents::SubagentCoordinator::coordinate(
            &runner_owned,
            &tool_call_id_c,
            &args_c,
        )
        .await
    });
    // Dropping an awaited parent invocation must drop synchronous sessions.
    // Background sessions were transferred to their own launch owner already.
    let abort_on_drop = AbortCoordinatorOnDrop(handle.abort_handle());
    let outcome = handle.await;
    drop(abort_on_drop);
    match outcome {
        Ok(Ok(res)) => res,
        Ok(Err(e)) => cade_agent::tools::manager::ToolResult {
            tool_call_id: tool_call_id.clone(),
            tool_name: "subagent".to_string(),
            output: format!("Coordinator error: {e}"),
            is_error: true,
            ui_resource_uri: None,
        },
        Err(e) => cade_agent::tools::manager::ToolResult {
            tool_call_id: tool_call_id.clone(),
            tool_name: "subagent".to_string(),
            output: format!("Coordinator task join error: {e}"),
            is_error: true,
            ui_resource_uri: None,
        },
    }
}

async fn handle_subagent_single_inner_tool_with_mode(
    state: &AppState,
    parent_agent_id: &str,
    parent_conversation_id: Option<&str>,
    tool_call_id: &str,
    args: &serde_json::Value,
    sse_tx: super::SseTx,
    permission_mode: cade_core::permissions::PermissionMode,
) -> cade_agent::tools::manager::ToolResult {
    let mut concrete = CadeSubagentExecutor::new(
        state.clone(),
        parent_agent_id.to_string(),
        parent_conversation_id.map(str::to_owned),
        tool_call_id.to_string(),
        Box::new(SseEventEmitter { tx: sse_tx }),
    );
    concrete.permission_mode = permission_mode;
    let executor: Box<dyn SubagentExecutor> = Box::new(concrete);
    executor.execute(args).await
}

pub(super) async fn handle_run_subagent_tool(
    state: &AppState,
    parent_agent_id: &str,
    parent_conversation_id: Option<&str>,
    tool_call_id: &str,
    args: &serde_json::Value,
    sse_tx: super::SseTx,
) -> cade_agent::tools::manager::ToolResult {
    let executor: Box<dyn SubagentExecutor> = Box::new(CadeSubagentExecutor::new(
        state.clone(),
        parent_agent_id.to_string(),
        parent_conversation_id.map(str::to_owned),
        tool_call_id.to_string(),
        Box::new(SseEventEmitter { tx: sse_tx }),
    ));
    executor.execute(args).await
}

struct ServerSubagentLlm<'a> {
    state: &'a AppState,
    runtime: Arc<cade_agent::tools::runtime::ToolRuntime>,
}

#[async_trait::async_trait]
impl<'a> cade_agent::subagents::SubagentLlmExecutor for ServerSubagentLlm<'a> {
    fn prepare_turn(
        &self,
        model: &str,
        prompt: &str,
        tools: &[serde_json::Value],
    ) -> (String, Vec<serde_json::Value>) {
        let prompt = format!(
            "{prompt}\n\nExecution workspace: {}",
            self.runtime.cwd.display()
        );
        if cade_ai::catalogue::supports_tools_for_model(model) {
            (prompt, tools.to_vec())
        } else {
            (
                format!(
                    "{prompt}\n\nThis model has no tool calling. Complete the task in a final text response."
                ),
                Vec::new(),
            )
        }
    }

    async fn complete_turn(
        &self,
        model: &str,
        system_prompt: &str,
        messages: &[cade_agent::subagents::SubagentMessage],
        tools: &[serde_json::Value],
    ) -> Result<cade_agent::subagents::SubagentTurnResponse, String> {
        let mut ai_messages = vec![cade_ai::LlmMessage {
            role: "system".to_string(),
            content: system_prompt.to_string(),
            tool_calls: None,
            tool_call_id: None,
            images: None,
            cache_control: None,
        }];

        for m in messages {
            let tool_calls = m.tool_calls.as_ref().map(|tcs| {
                tcs.iter()
                    .map(|tc| cade_ai::LlmToolCall {
                        id: tc.id.clone(),
                        name: tc.name.clone(),
                        arguments: tc.arguments.clone(),
                        thought_signature: None,
                    })
                    .collect()
            });
            ai_messages.push(cade_ai::LlmMessage {
                role: m.role.clone(),
                content: m.content.clone(),
                tool_calls,
                tool_call_id: m.tool_call_id.clone(),
                images: None,
                cache_control: None,
            });
        }

        let req = cade_ai::CompletionRequest {
            model: model.to_string(),
            messages: ai_messages,
            tools: tools.to_vec(),
            max_tokens: cade_ai::catalogue::max_tokens_for_model(model),
            reasoning_effort: super::runtime::current_execution_options()
                .and_then(|options| options.reasoning_effort.clone()),
        };

        let resp = self
            .state
            .llm
            .complete(&req)
            .await
            .map_err(|e| e.to_string())?;

        let tool_calls = resp
            .tool_calls
            .into_iter()
            .map(|tc| {
                let arguments = self.runtime.prepare_arguments(&tc.name, &tc.arguments);
                cade_agent::subagents::SubagentToolCall {
                    id: tc.id,
                    name: tc.name,
                    arguments,
                }
            })
            .collect();

        let tokens_used = resp
            .content
            .as_deref()
            .map(|t| cade_ai::count_tokens(model, t))
            .unwrap_or(0) as u64;

        Ok(cade_agent::subagents::SubagentTurnResponse {
            content: resp.content,
            tool_calls,
            tokens_used,
        })
    }
}

struct ServerSubagentTools<'a> {
    state: &'a AppState,
    parent_agent_id: String,
    runtime: Arc<cade_agent::tools::runtime::ToolRuntime>,
    permissions: cade_core::permissions::PermissionManager,
    hooks: Arc<cade_core::hooks::HookEngine>,
    recent_edits: Arc<Mutex<BTreeSet<String>>>,
}

struct ChildPipelineApproval {
    queue: HeadlessQueueAdapter,
    authorized_name: String,
    authorized_arguments: serde_json::Value,
}

#[async_trait::async_trait]
impl cade_agent::tools::ApprovalDelegate for ChildPipelineApproval {
    async fn request_approval(
        &self,
        _: &str,
        name: &str,
        arguments: &serde_json::Value,
        _: &str,
    ) -> cade_agent::Result<bool> {
        // SubagentSession already authorized this exact normalized call. A hook
        // changing its parameters must go back through the child's own queue.
        if name == self.authorized_name && arguments == &self.authorized_arguments {
            return Ok(true);
        }
        cade_core::permissions::PermissionService::request_permission(&self.queue, name, arguments)
            .await
            .map_err(cade_agent::Error::custom)
    }
}

#[async_trait::async_trait]
impl<'a> cade_agent::subagents::SubagentToolExecutor for ServerSubagentTools<'a> {
    async fn is_mcp_write(&self, tool_name: &str) -> bool {
        cade_agent::tools::is_mcp_write_tool(tool_name, &self.state.mcp).await
            || self.runtime.extension_is_write(tool_name)
    }

    async fn execute_tool(
        &self,
        tool_call_id: &str,
        tool_name: &str,
        arguments: &serde_json::Value,
        execution_path: &std::path::Path,
    ) -> Result<String, String> {
        if self.runtime.cwd != execution_path {
            return Err("Child tool runtime workspace does not match its execution path".into());
        }

        let pipeline = cade_agent::tools::ToolPipeline::new(
            self.runtime.clone(),
            self.permissions.clone(),
            self.hooks.clone(),
            Arc::new(ChildPipelineApproval {
                queue: HeadlessQueueAdapter {
                    db: self.state.db.clone(),
                    parent_agent_id: self.parent_agent_id.clone(),
                    subagent_id: self.runtime.agent_id.clone(),
                },
                authorized_name: cade_agent::tools::manager::canonical_name(tool_name).to_owned(),
                authorized_arguments: self.runtime.prepare_arguments(tool_name, arguments),
            }),
        );
        match pipeline.execute(tool_call_id, tool_name, arguments).await {
            Ok(executed) => {
                if !executed.is_error
                    && cade_agent::tools::manager::is_file_edit_tool(tool_name)
                    && let Some(path) = arguments["path"]
                        .as_str()
                        .or_else(|| arguments["file_path"].as_str())
                {
                    // These paths belong to the child's workspace until successful
                    // reconciliation. A conflict must not claim a parent edit.
                    let path = std::path::Path::new(path);
                    let path = path.strip_prefix(execution_path).unwrap_or(path);
                    self.recent_edits
                        .lock()
                        .unwrap_or_else(|e| e.into_inner())
                        .insert(path.to_string_lossy().to_string());
                }

                if executed.is_error {
                    Err(executed.output)
                } else {
                    Ok(executed.output)
                }
            }
            Err(error) => Err(error.to_string()),
        }
    }
}

pub(super) async fn handle_run_subagent_tool_inner(
    state: &AppState,
    parent_agent_id: &str,
    parent_conversation_id: Option<&str>,
    tool_call_id: &str,
    args: &serde_json::Value,
    emitter: Box<dyn SubagentEventEmitter>,
    parent_mode: cade_core::permissions::PermissionMode,
) -> cade_agent::tools::manager::ToolResult {
    use cade_agent::subagents::SubagentConfig;
    use cade_agent::tools::manager::ToolResult;

    // -- Parse + validate args through shared SubagentConfig -----------------
    let cfg = SubagentConfig::from_args(args);

    // Recursion-depth guard.  When a subagent spawns another subagent the
    // dispatcher injects `_subagent_depth = parent_depth + 1` into the
    // arguments before re-entering this function.  Default cap is 3.
    let max_depth: usize = std::env::var("CADE_SUBAGENT_MAX_DEPTH")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(3);
    if cfg.depth >= max_depth {
        return ToolResult {
            tool_call_id: tool_call_id.to_string(),
            tool_name: "run_subagent".to_string(),
            output: format!(
                "error: subagent recursion depth {} exceeds CADE_SUBAGENT_MAX_DEPTH ({max_depth}). \
                 Refusing to spawn deeper. Restructure the task or raise the limit if intentional.",
                cfg.depth
            ),
            is_error: true,
            ui_resource_uri: None,
        };
    }

    // Validate the requested definition before waiting for a slot or creating
    // any child state. Keep hidden definitions available for exact lookup.
    let cwd_for_defs = super::runtime::execution_workspace();
    let all_defs = cade_agent::subagents::discover_all_subagents(&cwd_for_defs);
    match cfg.resolve_definition(&all_defs) {
        Ok(_) => {}
        Err(reason) => {
            return ToolResult {
                tool_call_id: tool_call_id.to_string(),
                tool_name: "run_subagent".to_string(),
                output: reason,
                is_error: true,
                ui_resource_uri: None,
            };
        }
    };

    if let Err(reason) = cfg.validate() {
        return ToolResult {
            tool_call_id: tool_call_id.to_string(),
            tool_name: "run_subagent".to_string(),
            output: reason,
            is_error: true,
            ui_resource_uri: None,
        };
    }

    let subagent_id = format!("sa_{}", uuid::Uuid::new_v4());
    let mut session = cade_agent::subagents::SubagentSession::new(cfg.clone(), parent_agent_id);
    session.session_id = subagent_id.clone();
    session.register_control(true);
    let completion = session.completion();
    let (cancel_tx, mut cancel_rx) = tokio::sync::mpsc::channel(1);
    let cancellation = cade_agent::subagents::SubagentCancellation::new(cancel_tx)
        .with_completion(completion.clone());
    state
        .subagent_cancellations
        .write()
        .await
        .insert(subagent_id.clone(), cancellation.clone());
    session = session.with_cleanup(Box::new(SubagentAdmissionCleanup {
        map: state.subagent_cancellations.clone(),
        id: subagent_id.clone(),
        cancellation: cancellation.clone(),
        closed: false,
    }));

    if cfg.background {
        let state_owned = state.clone();
        let parent = parent_agent_id.to_string();
        let conversation = parent_conversation_id.map(str::to_owned);
        let call = tool_call_id.to_string();
        let args = args.clone();
        let completion_state = state.clone();
        let completion_parent = parent_agent_id.to_string();
        let completion_conversation = parent_conversation_id.map(str::to_owned);
        let completion_call = tool_call_id.to_string();
        let completion_id = subagent_id.clone();
        let completion_sse = emitter.raw_sse_tx();
        let completion_cancel = cancellation.clone();
        // launch_background spawns internally: capture explicitly rather than
        // relying on task-local inheritance, and retain no parent cancel token.
        let launch_options = super::runtime::current_execution_options();
        let launch = session.launch_background(
            state.subagent_semaphore.clone(),
            std::time::Duration::from_secs(subagent_timeout_secs()),
            cancel_rx,
            move |session, permit| async move {
                let (_, unused_rx) = tokio::sync::mpsc::channel(1);
                super::runtime::in_execution_scope(
                    launch_options,
                    run_subagent_with_permit(
                        &state_owned,
                        &parent,
                        conversation.as_deref(),
                        &call,
                        &args,
                        emitter,
                        parent_mode,
                        session,
                        permit,
                        unused_rx,
                    ),
                )
                .await
            },
            move |result| async move {
                completion_cancel.close();
                completion_state
                    .subagent_cancellations
                    .write()
                    .await
                    .remove(&completion_id);
                let (output, is_error, status) = match result {
                    Ok((result, status)) => (result.output, result.is_error, status),
                    Err(reason) => {
                        use cade_agent::subagents::session::SubagentLaunchFailure;
                        let status = match reason {
                            SubagentLaunchFailure::Cancelled => TerminalStatus::Cancelled,
                            SubagentLaunchFailure::TimedOut => TerminalStatus::Timeout,
                            SubagentLaunchFailure::Closed | SubagentLaunchFailure::Panicked => {
                                TerminalStatus::Error
                            }
                        };
                        let output = completion
                            .outcome()
                            .map(|outcome| outcome.summary_text().to_string())
                            .unwrap_or_else(|| reason.to_string());
                        (output, true, status)
                    }
                };
                let event = serde_json::json!({
                    "message_type": "subagent_complete",
                    "subagent_id": completion_id,
                    "status": status.as_str(),
                    "result_preview": output.chars().take(200).collect::<String>(),
                    "is_error": is_error,
                });
                deliver_background_result(
                    &completion_state,
                    &completion_parent,
                    completion_conversation.as_deref(),
                    &completion_call,
                    &completion_id,
                    output,
                    status,
                )
                .await;
                let _ = completion_sse
                    .send(Ok(super::runtime::RunEventEnvelope {
                        data: event.to_string(),
                    }))
                    .await;
            },
        );
        return ToolResult {
            tool_call_id: tool_call_id.to_string(),
            tool_name: "run_subagent".to_string(),
            output: format!(
                "Background subagent {} {} (launch acknowledged; outcome will arrive in this conversation)",
                launch.child_id,
                if launch.queued { "queued" } else { "started" }
            ),
            is_error: false,
            ui_resource_uri: None,
        };
    }

    let permit = match session
        .acquire_slot(
            state.subagent_semaphore.clone(),
            std::time::Duration::from_secs(subagent_timeout_secs()),
            &mut cancel_rx,
        )
        .await
    {
        Ok(permit) => permit,
        Err(reason) => {
            cancellation.close();
            state
                .subagent_cancellations
                .write()
                .await
                .remove(&subagent_id);
            return ToolResult {
                tool_call_id: tool_call_id.to_string(),
                tool_name: "run_subagent".to_string(),
                output: completion
                    .outcome()
                    .map(|outcome| outcome.summary_text().to_string())
                    .unwrap_or_else(|| reason.to_string()),
                is_error: true,
                ui_resource_uri: None,
            };
        }
    };

    let (result, _status) = run_subagent_with_permit(
        state,
        parent_agent_id,
        parent_conversation_id,
        tool_call_id,
        args,
        emitter,
        parent_mode,
        session,
        permit,
        cancel_rx,
    )
    .await;
    cancellation.close();
    state
        .subagent_cancellations
        .write()
        .await
        .remove(&subagent_id);
    result
}

pub(super) async fn deliver_background_result(
    state: &AppState,
    parent_agent_id: &str,
    parent_conversation_id: Option<&str>,
    tool_call_id: &str,
    subagent_id: &str,
    result: String,
    status: TerminalStatus,
) {
    let is_error = status != TerminalStatus::Done;
    let pending = crate::server::state::SubagentResult {
        subagent_id: subagent_id.to_string(),
        tool_call_id: format!("{tool_call_id}:outcome:{subagent_id}"),
        task_preview: String::new(),
        result,
        is_error,
        status,
        elapsed_secs: 0,
    };
    if let Err(error) =
        store_background_outcome(state, parent_agent_id, parent_conversation_id, &pending)
    {
        tracing::warn!(%subagent_id, %error, "background outcome delivery deferred to next parent run");
        state
            .pending_subagent_results
            .write()
            .await
            .entry((
                parent_agent_id.to_string(),
                parent_conversation_id.map(str::to_owned),
            ))
            .or_default()
            .push(pending);
        crate::server::api::agents::publish_global_event(
            Some(&state.db),
            "subagent_delivery_failed",
            serde_json::json!({
                "agent_id": parent_agent_id,
                "conversation_id": parent_conversation_id,
                "subagent_id": subagent_id,
                "status": "pending_retry",
                "error": "Background outcome could not be persisted; retry on the next parent run",
            }),
        );
    }
}

/// Persist a standalone conversation notification. A tool-role row would be
/// discarded by the provider sanitizer because its launch tool call was
/// already answered with the acknowledgement on the previous turn.
pub(super) fn store_background_outcome(
    state: &AppState,
    parent_agent_id: &str,
    parent_conversation_id: Option<&str>,
    pending: &crate::server::state::SubagentResult,
) -> Result<(), String> {
    let subagent_id = &pending.subagent_id;
    let result = &pending.result;
    let is_error = pending.is_error;
    let body = format!(
        "[background subagent {subagent_id} {}]\n{result}",
        pending.status.as_str()
    );
    let row = cade_store::sqlite::MessageRow {
        id: format!("subagent-outcome-{subagent_id}"),
        agent_id: parent_agent_id.to_string(),
        conversation_id: parent_conversation_id.map(str::to_owned),
        role: "user".to_string(),
        char_count: body.len(),
        content: serde_json::json!({
            "content": body,
            "tool_call_id": pending.tool_call_id,
            "launch_tool_call_id": pending.tool_call_id.rsplit_once(":outcome:").map(|(id, _)| id).unwrap_or(&pending.tool_call_id),
            "tool_name": "run_subagent",
            "subagent_id": subagent_id,
            "phase": "outcome",
            "status": pending.status.as_str(),
            "is_error": is_error,
        }),
    };
    match cade_store::sqlite::insert_message(&state.db, &row) {
        Ok(()) => Ok(()),
        Err(error) => {
            // Retrying an already delivered child must not block the parent.
            // A different row or a persistent storage error still fails.
            if cade_store::sqlite::message_content_matches(&state.db, &row.id, &row.content)
                .unwrap_or(false)
            {
                Ok(())
            } else {
                Err(error.to_string())
            }
        }
    }
}

#[allow(clippy::too_many_arguments)]
async fn run_subagent_with_permit(
    state: &AppState,
    parent_agent_id: &str,
    parent_conversation_id: Option<&str>,
    tool_call_id: &str,
    args: &serde_json::Value,
    emitter: Box<dyn SubagentEventEmitter>,
    parent_mode: cade_core::permissions::PermissionMode,
    mut session: cade_agent::subagents::SubagentSession,
    permit: tokio::sync::OwnedSemaphorePermit,
    mut cancel_rx: tokio::sync::mpsc::Receiver<()>,
) -> (cade_agent::tools::manager::ToolResult, TerminalStatus) {
    use cade_agent::subagents::SubagentConfig;
    use cade_agent::tools::manager::ToolResult;
    let cfg = SubagentConfig::from_args(args);
    let cwd_for_defs = super::runtime::execution_workspace();
    let accepted_options = super::runtime::current_execution_options();
    let all_defs = cade_agent::subagents::discover_all_subagents(&cwd_for_defs);
    let def_opt = match cfg.resolve_definition(&all_defs) {
        Ok(def) => def,
        Err(reason) => {
            return failed_session_result(&mut session, tool_call_id, reason).await;
        }
    };
    let max_depth: usize = std::env::var("CADE_SUBAGENT_MAX_DEPTH")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(3);
    let subagent_id = session.session_id.clone();
    let task_preview: String = cfg.prompt.chars().take(80).collect();
    let prompt = cfg.prompt_with_test_command();

    let is_subagent_readonly = def_opt
        .map(|d| d.tools.is_readonly())
        .unwrap_or_else(|| cfg.mode == "plan" || cfg.mode == "recall");

    let use_isolation = cfg.enforce_isolation
        || (std::env::var("CADE_ISOLATION")
            .map(|v| v == "true")
            .unwrap_or(false)
            && !is_subagent_readonly);
    let mut session_config = cfg.clone();
    session_config.enforce_isolation = use_isolation;
    session.config = session_config;
    if use_isolation {
        if accepted_options
            .as_ref()
            .is_some_and(|options| !matches!(options.runtime.backend.name(), "local" | "readonly"))
        {
            return failed_session_result(
                &mut session,
                tool_call_id,
                "error: workspace isolation requires a local execution backend".into(),
            )
            .await;
        }
        let root = cwd_for_defs.clone();
        // Merge the isolated snapshot through the workspace guard. A newly
        // initialized git branch has unrelated history to the parent and
        // cannot be merged back into an existing repository.
        if let Err(e) = session.prepare_workspace(&root, None).await {
            return failed_session_result(
                &mut session,
                tool_call_id,
                format!("error: required subagent isolation setup failed: {e}"),
            )
            .await;
        }
    }

    let parent_model = cade_store::sqlite::get_agent(&state.db, parent_agent_id)
        .ok()
        .flatten()
        .map(|a| a.model)
        .unwrap_or_else(|| state.config.default_model.clone());

    let model = cfg
        .resolve_model(def_opt)
        .map(|s| s.to_string())
        .unwrap_or_else(|| cade_ai::catalogue::select_fast_subagent_model(&parent_model, None));

    emitter
        .emit_started(&subagent_id, &task_preview, &cfg.mode, &model)
        .await;

    let start_time = std::time::Instant::now();

    // Build system prompt via shared resolution chain
    let system_prompt_base = cfg.resolve_system_prompt(def_opt);
    // Append "Task: <prompt>" so the subagent sees it in the system context
    // (the prompt is also sent as a separate user message below).
    let system_prompt = format!("{system_prompt_base}\n\nTask: {prompt}");

    // Seed the parent agent's pinned + short-tier memory blocks into the
    // subagent's system prompt so it inherits project context, persona,
    // and the active goal.  Uses the shared SubagentConfig helper to
    // ensure filtering and capping are identical in both paths.
    let seed_section: String = {
        let raw_blocks =
            cade_store::sqlite::get_active_blocks(&state.db, parent_agent_id).unwrap_or_default();
        let seed: Vec<cade_agent::agent::client::MemoryBlock> = raw_blocks
            .into_iter()
            .map(|(label, value, description, tier, _last_turn)| {
                cade_agent::agent::client::MemoryBlock {
                    label,
                    value,
                    description: if description.is_empty() {
                        None
                    } else {
                        Some(description)
                    },
                    tier: if tier.is_empty() { None } else { Some(tier) },
                }
            })
            .collect();
        let filtered = SubagentConfig::build_seed_memory(seed);
        SubagentConfig::format_seed_section(&filtered)
    };

    let parent_context =
        cade_store::sqlite::list_messages(&state.db, parent_agent_id, parent_conversation_id, 8)
            .unwrap_or_default()
            .into_iter()
            .map(|m| cade_agent::subagents::SubagentMessage {
                role: m.role,
                content: match m.content {
                    serde_json::Value::String(s) => s,
                    other => other.to_string(),
                },
                tool_calls: None,
                tool_call_id: None,
            })
            .collect();

    let system_prompt_full = format!("{system_prompt}{seed_section}");

    // ── Subagent agentic loop (Approach C) ──────────────────────────────
    //
    // Iterates LLM → tool dispatch → LLM with tool result, up to
    // `max_iters` rounds.  Tools are loaded from the parent agent's tool
    // list (with `run_subagent` stripped — see `filter_subagent_tools`)
    // and dispatched through the same `cade_agent::tools::manager::dispatch`
    // helper the parent loop uses.  No SSE streaming inside the loop and
    // no per-iteration DB persistence — subagents are ephemeral and only
    // their final result flows back to the parent.
    //
    // The loop terminates when either:
    //   (a) the LLM returns no tool_calls (assistant produced a final answer),
    //   (b) `max_iters` is reached (safety cap),
    //   (c) an LLM or dispatch error surfaces.
    let max_iters: usize = std::env::var("CADE_SUBAGENT_MAX_ITERS")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(10);
    let max_iters = accepted_options
        .as_ref()
        .map(|options| max_iters.min(options.max_turns))
        .unwrap_or(max_iters);

    // Snapshot the parent agent's tool schemas, stripped of `run_subagent`
    // for defence-in-depth alongside the depth counter.  If the parent is
    // not yet wired (no rows), `agent_tool_ids` is empty meaning "all
    // registered tools".
    let (parent_tool_schemas, inherited_tools): (Vec<serde_json::Value>, Vec<String>) = {
        let parent_tool_ids =
            cade_store::sqlite::get_agent_tool_ids(&state.db, parent_agent_id).unwrap_or_default();
        let all = cade_store::sqlite::list_tools(&state.db)
            .unwrap_or_default()
            .into_iter()
            .filter(|tool| {
                !tool.tags.iter().any(|tag| tag == "plugin") && !tool.id.starts_with("tool-plugin-")
            })
            .collect::<Vec<_>>();
        let mut raw: Vec<serde_json::Value> = if parent_tool_ids.is_empty() {
            all.into_iter().filter_map(|t| t.json_schema).collect()
        } else {
            all.into_iter()
                .filter(|t| parent_tool_ids.contains(&t.id))
                .filter_map(|t| t.json_schema)
                .collect()
        };
        // Dynamically include live capability schemas from CapabilityMesh seam (ADR-0020)
        use cade_core::capabilities::mesh::{CapabilityExecutionContext, CapabilityMesh};
        let cap_cx = CapabilityExecutionContext::new(parent_agent_id.to_string());
        let live_mesh = state.mcp.active_catalog(&cap_cx).await;
        for cap_s in live_mesh {
            let name = cap_s.schema["name"].as_str().unwrap_or("").to_string();
            if name.is_empty() || raw.iter().any(|r| r["name"].as_str() == Some(&name)) {
                continue;
            }
            raw.push(cap_s.schema);
        }
        for tool in super::plugin_execution::ready_catalog(&cwd_for_defs, &state.mcp)
            .await
            .tools
        {
            raw.retain(|schema| schema["name"].as_str() != Some(tool.name.as_str()));
            raw.push(tool.schema);
        }
        let tools_filter = def_opt.map(|d| &d.tools).unwrap_or_else(|| {
            if cfg.mode == "plan" {
                &cade_agent::subagents::SubagentTools::Readonly
            } else {
                &cade_agent::subagents::SubagentTools::All
            }
        });
        let allow_nesting = def_opt.map(|d| d.allow_run_subagent).unwrap_or(false);
        let inherited = raw
            .iter()
            .filter_map(|s| s["name"].as_str().map(str::to_string))
            .collect();
        let mut filtered = filter_subagent_tools(raw, tools_filter, allow_nesting);

        // REC-4/G4: Inject the built-in `finish` tool so the model has an
        // explicit, canonical way to signal completion.  This replaces the
        // implicit "no tool_calls = done" heuristic which could not distinguish
        // genuine completion from a confused model emitting prose mid-task.
        filtered.push(cade_agent::subagents::canonical_finish_tool_schema());
        (filtered, inherited)
    };

    let execution_path = session.execution_path(&cwd_for_defs).to_path_buf();
    let allowed_paths = inherited_child_paths(
        accepted_options
            .as_ref()
            .and_then(|options| options.runtime.allowed_paths.clone()),
        cfg.resolve_allowed_paths(def_opt),
        &cwd_for_defs,
        &execution_path,
    );

    // Create a lightweight ephemeral DB row for the subagent so its
    // meta-tool calls (update_memory, load_skill, etc.) are scoped to
    // its own namespace rather than writing into the parent agent's
    // memory store (memory isolation fix).
    if let Err(error) = cade_store::sqlite::create_agent(
        &state.db,
        &cade_store::sqlite::AgentRow {
            id: subagent_id.clone(),
            name: cfg.ephemeral_agent_name(&subagent_id),
            model: model.clone(),
            description: Some(cfg.ephemeral_description()),
            system_prompt: None,
            created_at: None,
            compaction_model: None,
            theme: None,
            active_plan_json: None,
            parent_id: Some(parent_agent_id.to_string()),
        },
    ) {
        return failed_session_result(
            &mut session,
            tool_call_id,
            format!("Creating ephemeral child: {error}"),
        )
        .await;
    }

    let writeback_facts = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let recent_edits = Arc::new(Mutex::new(BTreeSet::new()));
    session = session.with_cleanup(Box::new(ServerSubagentCleanup {
        state: state.clone(),
        environment: EphemeralEnvironment::new(
            state.db.clone(),
            subagent_id.clone(),
            parent_agent_id.to_string(),
        ),
        primary_root: cwd_for_defs.clone(),
        written: writeback_facts.clone(),
        permit: Some(permit),
        recent_edits: recent_edits.clone(),
    }));

    // Hierarchical memory mounting: Copy parent agent's core memory blocks
    // (project, persona, active_goal) into subagent's sandboxed namespace for grounding.
    if let Ok(parent_blocks) = cade_store::sqlite::get_memory_blocks(&state.db, parent_agent_id) {
        for (label, value, description) in parent_blocks {
            if matches!(label.as_str(), "project" | "persona" | "active_goal") {
                let _ = cade_store::sqlite::upsert_memory_block(
                    &state.db,
                    &subagent_id,
                    &label,
                    &value,
                    Some(&description),
                    None,
                );
            }
        }
    }

    let mut child_runtime = cade_agent::tools::runtime::ToolRuntime::new(
        Arc::new(super::storage_impl::ServerStorageBackend {
            state: state.clone(),
        }),
        state.mcp.clone(),
        subagent_id.clone(),
        execution_path.clone(),
    )
    .with_conversation(parent_conversation_id.map(str::to_owned));
    child_runtime.allowed_paths = allowed_paths;
    if let Some(options) = accepted_options.as_ref() {
        child_runtime.backend = options.runtime.backend.clone();
        child_runtime.extension = if execution_path == cwd_for_defs {
            options.runtime.extension.clone()
        } else {
            // The main-owned adapter separates package discovery from script
            // execution. Never reuse a parent's execution root in a clone.
            options.runtime.extension.as_ref().map(|_| {
                Arc::new(super::plugin_execution::ServerPluginTools::for_workspace(
                    cwd_for_defs.clone(),
                    execution_path.clone(),
                    state.mcp.clone(),
                )) as Arc<dyn cade_agent::tools::runtime::ToolExtension>
            })
        };
    }
    if child_runtime.extension.is_none() {
        child_runtime.extension = Some(Arc::new(
            super::plugin_execution::ServerPluginTools::for_workspace(
                cwd_for_defs.clone(),
                execution_path.clone(),
                state.mcp.clone(),
            ),
        ));
    }
    let child_runtime = Arc::new(child_runtime);
    let llm_executor = ServerSubagentLlm {
        state,
        runtime: child_runtime.clone(),
    };
    let mut permission_settings = accepted_options
        .as_ref()
        .map(|options| options.permission_settings.clone())
        .or_else(|| {
            cade_core::settings::SettingsManager::new(&cwd_for_defs)
                .ok()
                .map(|settings| settings.permission_settings().clone())
        });
    if let Some(settings) = permission_settings.as_mut() {
        for rule in settings.allow.iter_mut().chain(settings.deny.iter_mut()) {
            if let Some(mut parsed) = cade_core::permissions::PermissionRule::parse(rule)
                && let Some(pattern) = parsed.pattern.as_deref()
                && std::path::Path::new(pattern).is_absolute()
                && let Ok(relative) = std::path::Path::new(pattern).strip_prefix(&cwd_for_defs)
            {
                parsed.pattern = Some(execution_path.join(relative).to_string_lossy().into_owned());
                *rule = parsed.to_string();
            }
        }
    }
    let permissions = if let Some(ref settings) = permission_settings {
        let permissions = cade_core::permissions::PermissionManager::new_with_strict_bash(
            parent_mode,
            settings.strict_bash,
        );
        permissions.reload_from_settings(settings);
        permissions
    } else {
        cade_core::permissions::PermissionManager::new(parent_mode)
    };
    let hooks_config = accepted_options
        .as_ref()
        .map(|options| options.hooks_config.clone())
        .unwrap_or_else(|| {
            cade_core::settings::SettingsManager::new(&cwd_for_defs)
                .map(|settings| settings.merged_hooks())
                .unwrap_or_default()
        });
    let tools_executor = ServerSubagentTools {
        state,
        parent_agent_id: parent_agent_id.to_owned(),
        runtime: child_runtime,
        permissions: permissions.clone(),
        hooks: Arc::new(cade_core::hooks::HookEngine::new(
            hooks_config,
            execution_path,
            subagent_id.clone(),
        )),
        recent_edits,
    };
    let child_options = accepted_options.as_ref().map(|options| {
        options.for_child(
            tools_executor.runtime.clone(),
            tools_executor.permissions.clone(),
            tools_executor.hooks.clone(),
            max_iters,
            permission_settings.clone().unwrap_or_default(),
        )
    });
    let policy = cade_agent::subagents::SubagentToolPolicy {
        permissions,
        tools: inherited_child_tools(
            def_opt.map(|d| d.tools.clone()).unwrap_or_else(|| {
                if cfg.mode == "plan" || cfg.mode == "recall" {
                    cade_agent::subagents::SubagentTools::Readonly
                } else {
                    cade_agent::subagents::SubagentTools::All
                }
            }),
            tools_executor.runtime.allowed_paths.as_deref(),
        ),
        inherited_tools,
        allow_nesting: def_opt.is_some_and(|d| d.allow_run_subagent),
        max_depth,
    };
    session = session
        .with_parent_conversation_id(parent_conversation_id.map(str::to_owned))
        .with_parent_context(parent_context)
        .with_max_iters(max_iters)
        .with_max_tokens_budget(cfg.max_tokens_budget)
        .with_tool_policy(policy);

    // Only Ask reaches the existing PermissionService; Deny and Allow are
    // decided by the effective policy before any approval request is made.
    session = session.with_permission_service(Arc::new(HeadlessQueueAdapter {
        db: state.db.clone(),
        parent_agent_id: parent_agent_id.to_string(),
        subagent_id: subagent_id.clone(),
    }));

    // Event forwarder from SubagentSession to SSE stream
    let (session_evt_tx, mut session_evt_rx) = tokio::sync::mpsc::channel(128);
    let raw_sse = emitter.raw_sse_tx();
    let s_id_c = subagent_id.clone();
    let max_it = max_iters;
    let mut pause_states = session.subscribe_pause();
    let pause_sse = raw_sse.clone();
    let pause_id = subagent_id.clone();
    tokio::spawn(async move {
        while pause_states.changed().await.is_ok() {
            let state = *pause_states.borrow_and_update();
            // The session sends Paused as a distinct boundary event. Do not
            // duplicate it through the watch observer.
            if state == cade_agent::subagents::SubagentPauseState::Paused {
                continue;
            }
            let event = serde_json::json!({
                "message_type": "subagent_state",
                "subagent_id": pause_id,
                "status": state,
            });
            let _ = pause_sse.try_send(Ok(super::runtime::RunEventEnvelope {
                data: event.to_string(),
            }));
            if state == cade_agent::subagents::SubagentPauseState::Finished {
                break;
            }
        }
    });
    tokio::spawn(async move {
        while let Some(evt) = session_evt_rx.recv().await {
            match evt {
                cade_agent::subagents::SubagentEvent::TurnStarted { turn, .. } => {
                    let iter_ev = serde_json::json!({
                        "message_type": "subagent_iter",
                        "subagent_id": s_id_c,
                        "iter": turn,
                        "max_iters": max_it,
                    });
                    let _ = raw_sse
                        .send(Ok(super::runtime::RunEventEnvelope {
                            data: iter_ev.to_string(),
                        }))
                        .await;
                }
                cade_agent::subagents::SubagentEvent::OutputChunk { text } => {
                    let out_ev = serde_json::json!({
                        "message_type": "subagent_output",
                        "subagent_id": s_id_c,
                        "chunk": text,
                    });
                    let _ = raw_sse
                        .send(Ok(super::runtime::RunEventEnvelope {
                            data: out_ev.to_string(),
                        }))
                        .await;
                }
                cade_agent::subagents::SubagentEvent::SteeringApplied { messages } => {
                    let event = serde_json::json!({
                        "message_type": "subagent_steered",
                        "subagent_id": s_id_c,
                        "messages": messages,
                    });
                    let _ = raw_sse.try_send(Ok(super::runtime::RunEventEnvelope {
                        data: event.to_string(),
                    }));
                }
                cade_agent::subagents::SubagentEvent::ToolExecuting { tool_name, .. } => {
                    let tool_ev = serde_json::json!({
                        "message_type": "subagent_tool_start",
                        "subagent_id": s_id_c,
                        "tool": tool_name,
                    });
                    let _ = raw_sse
                        .send(Ok(super::runtime::RunEventEnvelope {
                            data: tool_ev.to_string(),
                        }))
                        .await;
                }
                cade_agent::subagents::SubagentEvent::ToolCompleted {
                    tool_name,
                    is_error,
                    ..
                } => {
                    let tool_ev = serde_json::json!({
                        "message_type": "subagent_tool_end",
                        "subagent_id": s_id_c,
                        "tool": tool_name,
                        "is_error": is_error,
                    });
                    let _ = raw_sse
                        .send(Ok(super::runtime::RunEventEnvelope {
                            data: tool_ev.to_string(),
                        }))
                        .await;
                }
                cade_agent::subagents::SubagentEvent::PauseStateChanged { state } => {
                    let event = serde_json::json!({
                        "message_type": "subagent_state",
                        "subagent_id": s_id_c,
                        "status": state,
                    });
                    let _ = raw_sse.try_send(Ok(super::runtime::RunEventEnvelope {
                        data: event.to_string(),
                    }));
                }
                _ => {}
            }
        }
    });

    session = session.with_event_emitter(cade_agent::subagents::SubagentEventEmitter::new(Some(
        session_evt_tx,
    )));

    let root_path = cwd_for_defs;

    let available_providers = cade_ai::catalogue::available_env_providers();
    let failover_candidates = build_failover_chain(&model, &parent_model, &available_providers);
    let timeout_dur = std::time::Duration::from_secs(subagent_timeout_secs());
    let completion = session.completion();
    let outcome = super::runtime::in_execution_scope(
        child_options,
        session.run_controlled(
            &llm_executor,
            &tools_executor,
            model,
            system_prompt_full,
            prompt.clone(),
            parent_tool_schemas,
            failover_candidates,
            &root_path,
            timeout_dur,
            &mut cancel_rx,
        ),
    )
    .await;

    let elapsed = start_time.elapsed().as_secs() as u32;

    let writeback_count = writeback_facts.load(std::sync::atomic::Ordering::SeqCst);

    let (output, is_error, status) = match outcome {
        cade_agent::subagents::SubagentOutcome::Done { summary, .. } => {
            (summary, false, TerminalStatus::Done)
        }
        outcome => {
            use cade_agent::subagents::session::SubagentLaunchFailure;
            let status = match completion.failure() {
                Some(SubagentLaunchFailure::Cancelled) => TerminalStatus::Cancelled,
                Some(SubagentLaunchFailure::TimedOut) => TerminalStatus::Timeout,
                _ => TerminalStatus::Error,
            };
            (outcome.summary_text().to_string(), true, status)
        }
    };

    let result_preview: String = output.chars().take(200).collect();
    if !cfg.background {
        emitter
            .emit_complete(
                &subagent_id,
                is_error,
                &result_preview,
                elapsed,
                writeback_count,
            )
            .await;
    }

    // C2: truncate at a UTF-8 char boundary, never at a raw byte index.
    let output_final = if output.len() > super::SSE_OUTPUT_TRUNCATE_BYTES {
        let head = super::truncate_at_char_boundary(&output, super::SSE_OUTPUT_TRUNCATE_BYTES);
        format!("{}…\n[truncated: {} chars total]", head, output.len())
    } else {
        output
    };

    (
        ToolResult {
            tool_call_id: tool_call_id.to_string(),
            tool_name: "run_subagent".to_string(),
            output: output_final,
            is_error,
            ui_resource_uri: None,
        },
        status,
    )
}

async fn failed_session_result(
    session: &mut cade_agent::subagents::SubagentSession,
    tool_call_id: &str,
    error: String,
) -> (cade_agent::tools::manager::ToolResult, TerminalStatus) {
    let outcome = session
        .finalize_outcome(cade_agent::subagents::SubagentOutcome::Failed { error })
        .await;
    (
        cade_agent::tools::manager::ToolResult {
            tool_call_id: tool_call_id.to_string(),
            tool_name: "run_subagent".to_string(),
            output: outcome.summary_text().to_string(),
            is_error: true,
            ui_resource_uri: None,
        },
        TerminalStatus::Error,
    )
}

/// A child can narrow the parent's filesystem grants, never replace or widen
/// them. Rebase grants into an isolated clone without changing process cwd.
fn inherited_child_paths(
    parent: Option<Vec<String>>,
    requested: Option<Vec<String>>,
    primary: &std::path::Path,
    execution: &std::path::Path,
) -> Option<Vec<String>> {
    if parent.is_none() && requested.is_none() {
        return None;
    }
    let (Ok(primary), Ok(execution)) =
        (resolve_child_grant(primary), resolve_child_grant(execution))
    else {
        return Some(Vec::new());
    };
    // Keep the two authorities separate until both have been resolved in the
    // source namespace. Rebasing raw syntax first would erase the constraints
    // represented by `..`, symlinks, or a differently spelled primary root.
    let resolve = |paths: Vec<String>| {
        paths
            .into_iter()
            .filter(|path| !path.trim().is_empty())
            .filter_map(|path| {
                let path = std::path::PathBuf::from(path);
                let path = if path.is_absolute() {
                    path
                } else {
                    primary.join(path)
                };
                resolve_child_grant(&path).ok()
            })
            .collect::<Vec<_>>()
    };
    let parent = parent.map(resolve);
    let requested = requested.map(resolve);
    let paths = match (parent, requested) {
        (None, None) => Vec::new(),
        (Some(paths), None) | (None, Some(paths)) => paths,
        (Some(parent), Some(requested)) => {
            let mut intersection = Vec::new();
            for parent in parent {
                for child in &requested {
                    if child.starts_with(&parent) {
                        intersection.push(child.clone());
                    } else if parent.starts_with(child) {
                        intersection.push(parent.clone());
                    }
                }
            }
            intersection
        }
    };
    let mut paths = paths
        .into_iter()
        .filter_map(|path| {
            match path.strip_prefix(&primary) {
                Ok(relative) => {
                    let projected = execution.join(relative);
                    let resolved = resolve_child_grant(&projected).ok()?;
                    // A clone-side alias must not redirect a source grant to a
                    // different subtree. Native validation receives frozen,
                    // absolute grant names, never an unresolved clone symlink.
                    (resolved == projected).then_some(resolved)
                }
                Err(_) => Some(path),
            }
        })
        .collect::<Vec<_>>();
    paths.sort();
    paths.dedup();
    Some(
        paths
            .into_iter()
            .map(|path| path.to_string_lossy().into_owned())
            .collect(),
    )
}

/// Resolve each existing ancestor before interpreting subsequent `..` parts.
/// Missing descendants are normalized lexically, so grants for future paths
/// have the same comparison semantics as existing paths. Broken links and I/O
/// failures reject the grant rather than falling back to unchecked syntax.
fn resolve_child_grant(path: &std::path::Path) -> std::io::Result<std::path::PathBuf> {
    use std::path::{Component, PathBuf};
    if !path.is_absolute() {
        return Err(std::io::Error::other(
            "filesystem grants require an absolute workspace",
        ));
    }
    let mut resolved = PathBuf::new();
    for part in path.components() {
        match part {
            Component::Prefix(_) | Component::RootDir => resolved.push(part.as_os_str()),
            Component::CurDir => {}
            Component::ParentDir => {
                match std::fs::symlink_metadata(&resolved) {
                    Ok(metadata) if !metadata.is_dir() => {
                        return Err(std::io::Error::other("grant ancestor is not a directory"));
                    }
                    Ok(_) => {}
                    Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                    Err(error) => return Err(error),
                }
                resolved.pop();
            }
            Component::Normal(_) => {
                resolved.push(part.as_os_str());
                match std::fs::symlink_metadata(&resolved) {
                    Ok(_) => resolved = resolved.canonicalize()?,
                    Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                    Err(error) => return Err(error),
                }
            }
        }
    }
    Ok(resolved)
}

fn inherited_child_tools(
    mut tools: cade_agent::subagents::SubagentTools,
    resolved: Option<&[String]>,
) -> cade_agent::subagents::SubagentTools {
    if let cade_agent::subagents::SubagentTools::Restricted { allowed_paths, .. } = &mut tools {
        // The shared session gate must enforce the same intersection as native
        // execution, not re-authorize the definition's original raw paths.
        *allowed_paths = resolved.unwrap_or_default().to_vec();
    }
    tools
}

struct CadeSubagentRunner {
    state: AppState,
    parent_agent_id: String,
    parent_conversation_id: Option<String>,
    sse_tx: super::SseTx,
}

#[async_trait::async_trait]
impl cade_agent::team::SubagentRunner for CadeSubagentRunner {
    async fn run_subagent(
        &self,
        task_call_id: &str,
        args: &serde_json::Value,
    ) -> Result<cade_agent::tools::manager::ToolResult, String> {
        Ok(handle_run_subagent_tool(
            &self.state,
            &self.parent_agent_id,
            self.parent_conversation_id.as_deref(),
            task_call_id,
            args,
            self.sse_tx.clone(),
        )
        .await)
    }
}

struct CadeLlmCompleter {
    state: AppState,
}

#[async_trait::async_trait]
impl cade_agent::team::LlmCompleter for CadeLlmCompleter {
    async fn complete(
        &self,
        model: &str,
        system_prompt: Option<&str>,
        prompt: &str,
    ) -> Result<String, String> {
        let mut messages = Vec::new();
        if let Some(sys) = system_prompt {
            messages.push(cade_ai::LlmMessage {
                role: "system".to_string(),
                content: sys.to_string(),
                tool_call_id: None,
                tool_calls: None,
                images: None,
                cache_control: None,
            });
        }
        messages.push(cade_ai::LlmMessage {
            role: "user".to_string(),
            content: prompt.to_string(),
            tool_call_id: None,
            tool_calls: None,
            images: None,
            cache_control: None,
        });

        let req = cade_ai::CompletionRequest {
            model: model.to_string(),
            messages,
            tools: vec![],
            max_tokens: 3000,
            reasoning_effort: super::runtime::current_execution_options()
                .and_then(|options| options.reasoning_effort.clone()),
        };

        match self.state.llm.complete(&req).await {
            Ok(resp) => {
                if let Some(content) = resp.content {
                    Ok(content)
                } else {
                    Err("No content returned from LLM".to_string())
                }
            }
            Err(e) => Err(format!("LLM completion error: {e}")),
        }
    }
}

pub(super) async fn handle_run_team_tool(
    state: AppState,
    parent_agent_id: String,
    parent_conversation_id: Option<String>,
    tool_call_id: String,
    args: serde_json::Value,
    sse_tx: super::SseTx,
) -> cade_agent::tools::manager::ToolResult {
    use cade_agent::team::{TeamConfig, TeamExecutor};
    use cade_agent::tools::manager::ToolResult;

    let parent_model = cade_store::sqlite::get_agent(&state.db, &parent_agent_id)
        .ok()
        .flatten()
        .map(|a| a.model)
        .unwrap_or_else(|| state.config.default_model.clone());

    let config = TeamConfig::from_args(&args);
    if let Err(e) = config.validate() {
        return ToolResult {
            tool_call_id: tool_call_id.clone(),
            tool_name: "run_team".to_string(),
            output: e,
            is_error: true,
            ui_resource_uri: None,
        };
    }

    let cwd = super::runtime::execution_workspace();
    let all_teams = cade_agent::team::discovery::discover_all_teams(&cwd);
    let team_def = match cade_agent::team::discovery::resolve_team_def(&config.team_id, &all_teams)
    {
        Some(t) => t,
        None => {
            return ToolResult {
                tool_call_id: tool_call_id.clone(),
                tool_name: "run_team".to_string(),
                output: format!("error: team not found: {}", config.team_id),
                is_error: true,
                ui_resource_uri: None,
            };
        }
    };

    let runner = CadeSubagentRunner {
        state: state.clone(),
        parent_agent_id: parent_agent_id.clone(),
        parent_conversation_id,
        sse_tx: sse_tx.clone(),
    };
    let llm = CadeLlmCompleter {
        state: state.clone(),
    };

    let executor = TeamExecutor::new();
    match executor
        .run_team(
            team_def,
            &config,
            &parent_model,
            &tool_call_id,
            &runner,
            &llm,
        )
        .await
    {
        Ok(results) => {
            let mut aggregated_json = Vec::new();
            for r in results {
                aggregated_json.push(serde_json::json!({
                    "task_index": r.task_index,
                    "output": r.output,
                    "is_error": r.is_error,
                }));
            }
            ToolResult {
                tool_call_id: tool_call_id.clone(),
                tool_name: "run_team".to_string(),
                output: serde_json::to_string_pretty(&aggregated_json).unwrap_or_default(),
                is_error: false,
                ui_resource_uri: None,
            }
        }
        Err(e) => ToolResult {
            tool_call_id: tool_call_id.clone(),
            tool_name: "run_team".to_string(),
            output: e,
            is_error: true,
            ui_resource_uri: None,
        },
    }
}
pub(super) async fn handle_cancel_subagent_tool(
    state: &AppState,
    tool_call_id: &str,
    args: &serde_json::Value,
) -> cade_agent::tools::manager::ToolResult {
    use cade_agent::tools::manager::ToolResult;

    let subagent_id = match args.get("subagent_id").and_then(|v| v.as_str()) {
        Some(id) => id,
        None => {
            return ToolResult {
                tool_call_id: tool_call_id.to_string(),
                tool_name: "cancel_subagent".to_string(),
                output: "error: 'subagent_id' is required".to_string(),
                is_error: true,
                ui_resource_uri: None,
            };
        }
    };

    let tx_opt = {
        let map = state.subagent_cancellations.read().await;
        map.get(subagent_id).cloned()
    };

    if let Some(tx) = tx_opt {
        let delivered = tx.cancel().is_ok();
        ToolResult {
            tool_call_id: tool_call_id.to_string(),
            tool_name: "cancel_subagent".to_string(),
            output: if delivered {
                format!("Cancel signal sent to subagent {subagent_id}")
            } else {
                format!("error: subagent {subagent_id} is no longer accepting cancellation")
            },
            is_error: !delivered,
            ui_resource_uri: None,
        }
    } else {
        ToolResult {
            tool_call_id: tool_call_id.to_string(),
            tool_name: "cancel_subagent".to_string(),
            output: format!("error: no active subagent found with ID {subagent_id}"),
            is_error: true,
            ui_resource_uri: None,
        }
    }
}

pub(super) async fn smart_memory_merge(
    state: AppState,
    agent_id: String,
    label: String,
    old_value: String,
    new_value: String,
    memory_type: String,
    confidence: f64,
) -> Result<(), String> {
    let prompt = format!(
        "You are a memory merge sub-agent. The parent agent already has a memory block labeled `{label}`. \
         A subagent just returned new information for this exact label. Synthesize the old and new facts into a single coherent block.\n\
         If there are conflicts, resolve them by keeping the most recent/detailed information or by noting the discrepancy.\n\
         Do not include any preamble, just the final merged content.\n\n\
         OLD VALUE:\n{old_value}\n\n\
         NEW VALUE:\n{new_value}"
    );

    // Grab model (cheapest capable)
    let compaction_model = cade_store::sqlite::get_agent(&state.db, &agent_id)
        .ok()
        .flatten()
        .map(|agent| {
            agent
                .compaction_model
                .unwrap_or_else(|| cade_ai::catalogue::fast_model_for_main_model(&agent.model))
        })
        .unwrap_or_else(|| {
            cade_ai::catalogue::select_fast_subagent_model(&state.config.default_model, None)
        });

    let max_tokens = cade_ai::catalogue::max_tokens_for_model(&compaction_model);

    let req = cade_ai::CompletionRequest {
        model: compaction_model,
        messages: vec![cade_ai::LlmMessage {
            role: "user".to_string(),
            content: prompt,
            tool_call_id: None,
            tool_calls: None,
            images: None,
            cache_control: None,
        }],
        tools: vec![],
        max_tokens,
        reasoning_effort: None,
    };

    let resp = state
        .llm
        .complete(&req)
        .await
        .map_err(|error| error.to_string())?;
    let merged = resp
        .content
        .filter(|text| !text.trim().is_empty())
        .ok_or_else(|| "Smart memory merge returned no content".to_string())?;
    let desc = "Smart merged after subagent run".to_string();
    cade_store::sqlite::upsert_memory_block_typed(
        &state.db,
        &agent_id,
        &label,
        merged.trim(),
        Some(&desc),
        None,
        Some(&memory_type),
        Some(confidence),
    )
    .map_err(|error| format!("Saving smart memory merge: {error}"))
}

#[allow(dead_code)]
pub struct HeadlessQueueAdapter {
    pub db: cade_store::sqlite::Db,
    pub parent_agent_id: String,
    pub subagent_id: String,
}

type ChildApprovalWithdrawal =
    futures::future::Shared<futures::future::BoxFuture<'static, Result<(), String>>>;

fn child_approval_withdrawals()
-> &'static Mutex<HashMap<String, HashMap<String, ChildApprovalWithdrawal>>> {
    static WITHDRAWALS: OnceLock<Mutex<HashMap<String, HashMap<String, ChildApprovalWithdrawal>>>> =
        OnceLock::new();
    WITHDRAWALS.get_or_init(|| Mutex::new(HashMap::new()))
}

fn forget_child_approval_withdrawal(parent: &str, id: &str) {
    let mut pending = child_approval_withdrawals()
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    if let Some(requests) = pending.get_mut(parent) {
        requests.remove(id);
        if requests.is_empty() {
            pending.remove(parent);
        }
    }
}

async fn finish_prior_child_approval_withdrawals(parent: &str) -> Result<(), String> {
    loop {
        let pending = child_approval_withdrawals()
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .get(parent)
            .map(|requests| {
                requests
                    .iter()
                    .map(|(id, work)| (id.clone(), work.clone()))
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();
        if pending.is_empty() {
            return Ok(());
        }
        for (id, work) in pending {
            work.await
                .map_err(|error| format!("Previous child approval withdrawal failed: {error}"))?;
            forget_child_approval_withdrawal(parent, &id);
        }
    }
}

struct ChildApprovalCleanupGuard {
    db: cade_store::sqlite::Db,
    id: String,
    parent_agent_id: String,
    subagent_id: String,
    conversation_id: Option<String>,
    armed: bool,
}

impl ChildApprovalCleanupGuard {
    fn withdraw(&self) -> Result<(), String> {
        const STATUS: &str = "denied:Subagent approval request cancelled";
        let changed = cade_store::sqlite::resolve_pending_approval(&self.db, &self.id, STATUS)
            .map_err(|error| format!("Withdrawing approval {}: {error}", self.id))?;
        if changed {
            let sequence = crate::server::api::agents::publish_global_event(
                Some(&self.db),
                "approval_resolved",
                serde_json::json!({
                    "message_type": "approval_resolved", "id": self.id,
                    "status": STATUS, "approved": false,
                    "agent_id": self.parent_agent_id, "subagent_id": self.subagent_id,
                    "conversation_id": self.conversation_id,
                }),
            );
            if sequence == 0 {
                tracing::error!(approval_id = %self.id, "approval withdrawal was broadcast but global event persistence failed");
            }
        }
        Ok(())
    }
}

impl Drop for ChildApprovalCleanupGuard {
    fn drop(&mut self) {
        if !self.armed {
            return;
        }
        use futures::FutureExt;
        let cleanup = Self {
            db: self.db.clone(),
            id: self.id.clone(),
            parent_agent_id: self.parent_agent_id.clone(),
            subagent_id: self.subagent_id.clone(),
            conversation_id: self.conversation_id.clone(),
            armed: false,
        };
        let parent = cleanup.parent_agent_id.clone();
        let id = cleanup.id.clone();
        let (completion, completed) = tokio::sync::oneshot::channel();
        let work = async move {
            completed
                .await
                .map_err(|error| format!("Approval withdrawal worker ended: {error}"))?
        }
        .boxed()
        .shared();
        child_approval_withdrawals()
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .entry(parent.clone())
            .or_default()
            .insert(id.clone(), work);
        // Drop only schedules work. A dedicated worker also works during Tokio
        // shutdown/outside a runtime; no database or publisher I/O occurs here.
        // Catch and report worker panics instead of detaching an unobserved
        // panic, and make completion awaitable by every later parent request.
        let spawned = std::thread::Builder::new().name("cade-approval-withdrawal".into()).spawn(move || {
            let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| cleanup.withdraw()))
                .unwrap_or_else(|_| Err("Child approval withdrawal worker panicked".into()));
            if let Err(error) = &result {
                tracing::error!(approval_id = %id, agent_id = %parent, %error, "child approval withdrawal failed");
            }
            let success = result.is_ok();
            if completion.send(result).is_err() {
                tracing::error!(approval_id = %id, "child approval withdrawal completion observer closed");
            }
            if success { forget_child_approval_withdrawal(&parent, &id); }
        });
        if let Err(error) = spawned {
            tracing::error!(approval_id = %self.id, %error, "could not start child approval withdrawal worker");
        }
    }
}

#[async_trait]
impl cade_core::permissions::PermissionService for HeadlessQueueAdapter {
    async fn request_permission(
        &self,
        tool_name: &str,
        args: &serde_json::Value,
    ) -> Result<bool, String> {
        finish_prior_child_approval_withdrawals(&self.parent_agent_id).await?;
        let approval_id = format!("app-{}", uuid::Uuid::new_v4());
        let args_str = args.to_string();
        if let Err(e) = cade_store::sqlite::create_pending_approval(
            &self.db,
            &approval_id,
            &self.parent_agent_id,
            Some(&self.subagent_id),
            tool_name,
            &args_str,
        ) {
            tracing::warn!("Failed to create pending approval: {e}");
            return Ok(false);
        }

        let conversation_id = super::runtime::current_execution_options()
            .and_then(|options| options.runtime.conversation_id.clone());
        let _pending = ChildApprovalCleanupGuard {
            db: self.db.clone(),
            id: approval_id.clone(),
            parent_agent_id: self.parent_agent_id.clone(),
            subagent_id: self.subagent_id.clone(),
            conversation_id: conversation_id.clone(),
            armed: true,
        };

        crate::server::api::agents::publish_global_event(
            Some(&self.db),
            "approval_required",
            serde_json::json!({
                "message_type": "approval_required",
                "id": approval_id,
                "agent_id": self.parent_agent_id,
                "subagent_id": self.subagent_id,
                "conversation_id": conversation_id,
                "tool_name": tool_name,
                "arguments": args,
            }),
        );

        // Trigger native desktop notification via CADE's cross-platform desktop notification service
        #[cfg(feature = "desktop")]
        {
            let title = "CADE — Approval Required";
            let body = format!(
                "Subagent [{}] requests permission to run '{}'",
                self.subagent_id, tool_name
            );
            if let Err(e) = cade_desktop::desktop::notify::send_notification(
                title,
                &body,
                cade_desktop::desktop::notify::Urgency::Critical,
            ) {
                tracing::warn!("Failed to send desktop notification: {e}");
            }
        }

        // Wait for approval
        let timeout_secs = 600;
        let start_time = std::time::Instant::now();
        let mut poll_interval = std::time::Duration::from_millis(200);

        loop {
            if start_time.elapsed().as_secs() > timeout_secs {
                return Err("Approval request timed out after 10 minutes.".to_string());
            }

            match cade_store::sqlite::get_approval_status(&self.db, &approval_id) {
                Ok(Some(status)) if status == "approved" || status.starts_with("approved:") => {
                    return Ok(true);
                }
                Ok(Some(status)) if status == "denied" => return Ok(false),
                Ok(Some(status)) if status.starts_with("denied:") => {
                    return Err(format!("Permission Denied: {}", &status[7..]));
                }
                Ok(Some(status)) if status == "pending" => {}
                other => return Err(format!("Approval status unavailable: {other:?}")),
            }

            tokio::time::sleep(poll_interval).await;
            poll_interval = (poll_interval * 2).min(std::time::Duration::from_secs(1));
        }
    }
}

/// Build a prioritized multi-provider failover chain.
pub(crate) fn build_failover_chain(
    primary_model: &str,
    parent_model: &str,
    available_providers: &[String],
) -> Vec<String> {
    let mut chain = Vec::new();
    let mut seen = std::collections::HashSet::new();

    // 1. Primary requested model
    if !primary_model.is_empty() {
        chain.push(primary_model.to_string());
        seen.insert(primary_model.to_string());
    }

    // 2. Parent model (if distinct)
    if !parent_model.is_empty() && !seen.contains(parent_model) {
        chain.push(parent_model.to_string());
        seen.insert(parent_model.to_string());
    }

    // 3. Configured fast/default choices for available providers. IDs remain
    // upstream identifiers, so nested deployment names are qualified once.
    let registry = cade_ai::provider_registry::ProviderRegistry::configured();
    let mut providers: Vec<_> = available_providers
        .iter()
        .filter_map(|name| registry.get(name))
        .collect();
    providers.sort_by_key(|provider| (provider.fast_priority, provider.priority, &provider.name));
    for provider in providers {
        for model in provider
            .fast_model
            .iter()
            .chain(provider.default_model.iter())
        {
            if model.trim().is_empty() {
                continue;
            }
            let model_id = format!("{}/{model}", provider.name);
            if seen.insert(model_id.clone()) {
                chain.push(model_id);
            }
        }
    }

    chain
}

/// Determine whether an error warrants trying the next candidate in the failover chain.
#[allow(dead_code)]
pub(crate) fn is_failover_worthy_error(err_str: &str) -> bool {
    let s = err_str.to_lowercase();
    s.contains("404")
        || s.contains("not found")
        || s.contains("429")
        || s.contains("rate limit")
        || s.contains("credit balance")
        || s.contains("insufficient_quota")
        || s.contains("quota exceeded")
        || s.contains("billing")
        || s.contains("402")
        || s.contains("unauthorized")
        || s.contains("invalid api key")
}

#[cfg(test)]
#[path = "subagent_review_tests.rs"]
mod review_tests;

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    struct EmptyMergeLlm;

    #[async_trait::async_trait]
    impl cade_ai::LlmProvider for EmptyMergeLlm {
        async fn complete(
            &self,
            _: &cade_ai::CompletionRequest,
        ) -> cade_ai::Result<cade_ai::CompletionResponse> {
            Ok(cade_ai::CompletionResponse {
                content: None,
                tool_calls: vec![],
                finish_reason: "stop".into(),
            })
        }

        async fn stream(
            &self,
            _: &cade_ai::CompletionRequest,
        ) -> cade_ai::Result<
            std::pin::Pin<
                Box<dyn tokio_stream::Stream<Item = cade_ai::Result<cade_ai::StreamChunk>> + Send>,
            >,
        > {
            panic!("cleanup uses complete, not streaming")
        }
    }

    fn cleanup_test_state() -> AppState {
        let state = super::super::tests::build_state_with_llm(Arc::new(EmptyMergeLlm));
        for id in ["cleanup-parent", "cleanup-child"] {
            cade_store::sqlite::create_agent(
                &state.db,
                &cade_store::sqlite::AgentRow {
                    id: id.into(),
                    name: id.into(),
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
        }
        cade_store::sqlite::upsert_memory_block(
            &state.db,
            "cleanup-child",
            "finding",
            "child fact",
            None,
            None,
        )
        .unwrap();
        state
    }

    fn cleanup_test_session(
        state: &AppState,
    ) -> (
        cade_agent::subagents::SubagentSession,
        Arc<std::sync::atomic::AtomicUsize>,
    ) {
        cleanup_test_session_in_workspace(state, super::super::runtime::execution_workspace())
    }

    fn cleanup_test_session_in_workspace(
        state: &AppState,
        primary_root: std::path::PathBuf,
    ) -> (
        cade_agent::subagents::SubagentSession,
        Arc<std::sync::atomic::AtomicUsize>,
    ) {
        let count = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let session = cade_agent::subagents::SubagentSession::new(
            cade_agent::subagents::SubagentConfig::from_args(&serde_json::json!({"prompt":"task"})),
            "cleanup-parent",
        )
        .with_cleanup(Box::new(ServerSubagentCleanup {
            state: state.clone(),
            environment: EphemeralEnvironment::new(
                state.db.clone(),
                "cleanup-child".into(),
                "cleanup-parent".into(),
            ),
            primary_root,
            written: count.clone(),
            permit: None,
            recent_edits: Arc::new(Mutex::new(BTreeSet::from(["src/child.rs".to_string()]))),
        }));
        (session, count)
    }

    fn cleanup_done() -> cade_agent::subagents::SubagentOutcome {
        cade_agent::subagents::SubagentOutcome::Done {
            summary: "task completed".into(),
            iterations: 0,
            tool_calls_count: 0,
            token_usage: 0,
        }
    }

    #[tokio::test]
    async fn server_session_owns_ephemeral_writeback_and_deletion_once() {
        for success in [true, false] {
            let state = cleanup_test_state();
            let (mut session, written) = cleanup_test_session(&state);
            let result = if success {
                session.finalize_outcome(cleanup_done()).await
            } else {
                session
                    .finalize_interruption(
                        cade_agent::subagents::session::SubagentLaunchFailure::Cancelled,
                    )
                    .await
            };
            assert_eq!(result.is_success(), success, "{}", result.summary_text());
            assert!(
                cade_store::sqlite::get_agent(&state.db, "cleanup-child")
                    .unwrap()
                    .is_none()
            );
            let blocks =
                cade_store::sqlite::get_memory_blocks(&state.db, "cleanup-parent").unwrap();
            assert_eq!(
                blocks
                    .iter()
                    .any(|(label, value, _)| label == "subagent:finding" && value == "child fact"),
                success
            );
            assert_eq!(
                blocks
                    .iter()
                    .any(|(label, value, _)| label == "recent_edits"
                        && value.contains("src/child.rs")),
                success
            );
            assert_eq!(
                written.load(std::sync::atomic::Ordering::SeqCst),
                usize::from(success)
            );
            assert_eq!(session.finalize_outcome(cleanup_done()).await, result);
        }
    }

    #[tokio::test]
    async fn server_session_recent_edits_name_each_primary_workspace_after_reconciliation() {
        let first_root = tempfile::tempdir().unwrap();
        let second_root = tempfile::tempdir().unwrap();
        let first_state = cleanup_test_state();
        let second_state = cleanup_test_state();
        let (mut first, _) =
            cleanup_test_session_in_workspace(&first_state, first_root.path().to_path_buf());
        let (mut second, _) =
            cleanup_test_session_in_workspace(&second_state, second_root.path().to_path_buf());
        first
            .prepare_workspace(first_root.path(), None)
            .await
            .unwrap();
        second
            .prepare_workspace(second_root.path(), None)
            .await
            .unwrap();
        let first_clone = first.execution_path(first_root.path()).to_path_buf();
        let second_clone = second.execution_path(second_root.path()).to_path_buf();
        for (clone, content) in [
            (&first_clone, "first workspace"),
            (&second_clone, "second workspace"),
        ] {
            fs::create_dir(clone.join("src")).unwrap();
            fs::write(clone.join("src/child.rs"), content).unwrap();
        }
        for state in [&first_state, &second_state] {
            assert!(
                !cade_store::sqlite::get_memory_blocks(&state.db, "cleanup-parent")
                    .unwrap()
                    .iter()
                    .any(|(label, _, _)| label == "recent_edits"),
                "metadata waits for reconciliation"
            );
        }
        let (first_outcome, second_outcome) = tokio::join!(
            first.finalize_outcome(cleanup_done()),
            second.finalize_outcome(cleanup_done()),
        );
        assert!(
            first_outcome.is_success(),
            "{}",
            first_outcome.summary_text()
        );
        assert!(
            second_outcome.is_success(),
            "{}",
            second_outcome.summary_text()
        );
        for (state, primary, clone, other_primary, content) in [
            (
                &first_state,
                first_root.path(),
                &first_clone,
                second_root.path(),
                "first workspace",
            ),
            (
                &second_state,
                second_root.path(),
                &second_clone,
                first_root.path(),
                "second workspace",
            ),
        ] {
            assert_eq!(
                fs::read_to_string(primary.join("src/child.rs")).unwrap(),
                content
            );
            let blocks =
                cade_store::sqlite::get_memory_blocks(&state.db, "cleanup-parent").unwrap();
            let (_, value, _) = blocks
                .iter()
                .find(|(label, _, _)| label == "recent_edits")
                .unwrap();
            assert_eq!(
                value,
                &format!(
                    "Recently edited: {}",
                    primary.join("src/child.rs").display()
                )
            );
            assert!(
                !value.contains(&clone.to_string_lossy().to_string()),
                "metadata must not name the discarded clone"
            );
            assert!(
                !value.contains(&other_primary.to_string_lossy().to_string()),
                "metadata must not name another workspace"
            );
            assert!(!clone.exists());
        }
    }

    #[tokio::test]
    async fn server_session_memory_merge_failure_is_reported_and_parent_fact_preserved() {
        let state = cleanup_test_state();
        cade_store::sqlite::upsert_memory_block(
            &state.db,
            "cleanup-parent",
            "subagent:finding",
            "parent fact",
            None,
            None,
        )
        .unwrap();
        let (mut session, written) = cleanup_test_session(&state);
        let receipt = session.completion();
        let result = session.finalize_outcome(cleanup_done()).await;
        assert!(!result.is_success());
        assert!(
            result
                .summary_text()
                .contains("Smart memory merge returned no content")
        );
        assert_eq!(receipt.outcome(), Some(result));
        assert!(
            cade_store::sqlite::get_agent(&state.db, "cleanup-child")
                .unwrap()
                .is_none()
        );
        let blocks = cade_store::sqlite::get_memory_blocks(&state.db, "cleanup-parent").unwrap();
        assert!(
            blocks
                .iter()
                .any(|(label, value, _)| label == "subagent:finding" && value == "parent fact")
        );
        assert_eq!(written.load(std::sync::atomic::Ordering::SeqCst), 0);
    }

    #[tokio::test]
    async fn server_session_ephemeral_deletion_failure_is_not_reported_as_success() {
        let state = cleanup_test_state();
        state.db.get().unwrap().execute_batch(
            "CREATE TRIGGER fail_child_cleanup BEFORE DELETE ON agents WHEN OLD.id = 'cleanup-child'
             BEGIN SELECT RAISE(FAIL, 'injected child deletion failure'); END;",
        ).unwrap();
        let (mut session, _) = cleanup_test_session(&state);
        let result = session.finalize_outcome(cleanup_done()).await;
        assert!(!result.is_success());
        assert!(
            result
                .summary_text()
                .contains("injected child deletion failure")
        );
        assert!(
            cade_store::sqlite::get_agent(&state.db, "cleanup-child")
                .unwrap()
                .is_some()
        );
        assert_eq!(session.finalize_outcome(cleanup_done()).await, result);
        state
            .db
            .get()
            .unwrap()
            .execute_batch("DROP TRIGGER fail_child_cleanup;")
            .unwrap();
        cade_store::sqlite::delete_agent(&state.db, "cleanup-child").unwrap();
    }

    #[tokio::test]
    async fn server_session_unreadable_findings_are_an_error_instead_of_empty_writeback() {
        let state = cleanup_test_state();
        state
            .db
            .get()
            .unwrap()
            .execute_batch(
                "UPDATE shared_memory_blocks SET value = X'FF' WHERE id IN
             (SELECT block_id FROM agent_memory_blocks WHERE agent_id = 'cleanup-child');",
            )
            .unwrap();
        let (mut session, written) = cleanup_test_session(&state);
        let result = session.finalize_outcome(cleanup_done()).await;
        assert!(!result.is_success());
        assert!(result.summary_text().contains("reading child findings"));
        assert!(
            cade_store::sqlite::get_agent(&state.db, "cleanup-child")
                .unwrap()
                .is_none()
        );
        assert_eq!(written.load(std::sync::atomic::Ordering::SeqCst), 0);
    }

    #[tokio::test]
    async fn server_session_queued_drop_closes_its_cancellation_registration() {
        let mut session = cade_agent::subagents::SubagentSession::new(
            cade_agent::subagents::SubagentConfig::from_args(&serde_json::json!({"prompt":"task"})),
            "parent",
        );
        session.register_control(true);
        let receipt = session.completion();
        let (tx, _rx) = tokio::sync::mpsc::channel(1);
        let cancellation =
            cade_agent::subagents::SubagentCancellation::new(tx).with_completion(receipt.clone());
        let map = Arc::new(tokio::sync::RwLock::new(HashMap::new()));
        map.write()
            .await
            .insert(session.session_id.clone(), cancellation.clone());
        let id = session.session_id.clone();
        let session = session.with_cleanup(Box::new(SubagentAdmissionCleanup {
            map: map.clone(),
            id,
            cancellation: cancellation.clone(),
            closed: false,
        }));
        drop(session);
        assert!(map.read().await.is_empty());
        assert!(cancellation.cancel().is_err());
        assert!(!receipt.outcome().unwrap().is_success());
    }

    #[tokio::test]
    async fn server_subagent_approval_waits_for_real_queue_decision() {
        use cade_core::permissions::PermissionService;
        let db = cade_store::sqlite::open(":memory:").unwrap();
        cade_store::sqlite::create_agent(
            &db,
            &cade_store::sqlite::AgentRow {
                id: "parent".into(),
                name: "Parent".into(),
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
        let adapter = HeadlessQueueAdapter {
            db: db.clone(),
            parent_agent_id: "parent".into(),
            subagent_id: "child".into(),
        };
        let request = tokio::spawn(async move {
            adapter
                .request_permission("write_file", &serde_json::json!({"path":"src/lib.rs"}))
                .await
        });
        let pending = tokio::time::timeout(std::time::Duration::from_secs(3), async {
            loop {
                let rows = cade_store::sqlite::list_pending_approvals(&db).unwrap();
                if !rows.is_empty() {
                    break rows;
                }
                tokio::time::sleep(std::time::Duration::from_millis(10)).await;
            }
        })
        .await
        .unwrap();
        assert!(!request.is_finished(), "queueing is not execution approval");
        assert_eq!(pending[0].tool_name, "write_file");
        cade_store::sqlite::resolve_pending_approval(&db, &pending[0].id, "denied").unwrap();
        assert!(!request.await.unwrap().unwrap());
    }

    #[test]
    fn test_filter_subagent_tools_constitutional_inheritance() {
        let schemas = vec![
            serde_json::json!({ "name": "read_file" }),
            serde_json::json!({ "name": "serena__search_for_pattern" }),
            serde_json::json!({ "name": "serena__replace_content" }),
            serde_json::json!({ "name": "read__replace_content" }),
            serde_json::json!({ "name": "desktop-commander-mcp__read_file" }),
            serde_json::json!({ "name": "run_subagent" }),
            serde_json::json!({ "name": "finish" }),
        ];

        let filtered = filter_subagent_tools(
            schemas,
            &cade_agent::subagents::SubagentTools::Readonly,
            false,
        );

        let names: Vec<&str> = filtered
            .iter()
            .map(|s| s["name"].as_str().unwrap())
            .collect();
        assert!(names.contains(&"read_file"));
        assert!(
            names.contains(&"serena__search_for_pattern"),
            "read/search MCP tools must pass in readonly mode"
        );
        assert!(
            names.contains(&"desktop-commander-mcp__read_file"),
            "desktop read MCP tools must pass"
        );
        assert!(
            !names.contains(&"serena__replace_content"),
            "mutating tools must be filtered in readonly mode"
        );
        assert!(
            !names.contains(&"read__replace_content"),
            "namespace cannot make a write tool read-only"
        );
        assert!(!names.contains(&"run_subagent"), "nesting tools filtered");
        assert!(
            !names.contains(&"finish"),
            "stale finish filtered for fresh injection"
        );
    }

    #[tokio::test]
    async fn test_workspace_cloning_and_copy_back() -> std::io::Result<()> {
        let src = tempfile::tempdir()?;

        // Create some mock source files
        fs::write(src.path().join("a.txt"), "hello")?;
        fs::create_dir(src.path().join("sub"))?;
        fs::write(src.path().join("sub/b.txt"), "world")?;

        // Clone it
        let clone_dir = cade_agent::tools::IsolatedWorkspace::clone_from(src.path())?;
        assert!(clone_dir.path().join("a.txt").exists());
        assert!(clone_dir.path().join("sub/b.txt").exists());

        // Modify in clone
        fs::write(clone_dir.path().join("a.txt"), "hello modified")?;
        fs::write(clone_dir.path().join("sub/b.txt"), "world modified")?;
        fs::write(clone_dir.path().join("new.txt"), "fresh file")?;

        // Copy back
        clone_dir.merge_back().await?;

        assert_eq!(
            fs::read_to_string(src.path().join("a.txt"))?,
            "hello modified"
        );
        assert_eq!(
            fs::read_to_string(src.path().join("sub/b.txt"))?,
            "world modified"
        );
        assert_eq!(
            fs::read_to_string(src.path().join("new.txt"))?,
            "fresh file"
        );

        Ok(())
    }

    async fn run_git_test(cwd: &std::path::Path, args: &[&str]) -> (i32, String, String) {
        let mut cmd = tokio::process::Command::new("git");
        cmd.args(args).current_dir(cwd);
        let out = cmd.output().await.unwrap();
        let exit = out.status.code().unwrap_or(-1);
        let stdout = String::from_utf8_lossy(&out.stdout).to_string();
        let stderr = String::from_utf8_lossy(&out.stderr).to_string();
        (exit, stdout, stderr)
    }

    #[tokio::test]
    async fn test_workspace_cloning_and_copy_back_with_git_branch() -> std::io::Result<()> {
        let src = tempfile::tempdir()?;

        // Setup a mock git repository on the host
        let (init_exit, _, _) = run_git_test(src.path(), &["init"]).await;
        assert_eq!(init_exit, 0);
        let _ = run_git_test(src.path(), &["config", "user.name", "CADE User"]).await;
        let _ = run_git_test(src.path(), &["config", "user.email", "user@cade.ai"]).await;

        // Create some mock source files
        fs::write(src.path().join("a.txt"), "hello")?;
        fs::create_dir(src.path().join("sub"))?;
        fs::write(src.path().join("sub/b.txt"), "world")?;

        // Commit initial files on main
        let _ = run_git_test(src.path(), &["add", "-A"]).await;
        let _ = run_git_test(src.path(), &["commit", "-m", "Initial commit"]).await;

        // Clone it
        let clone_dir = cade_agent::tools::IsolatedWorkspace::clone_from(src.path())?;

        // Enable git branch sandboxing
        let clone_dir = clone_dir.with_git_branch("temp-sub-1").await;

        // Modify in clone
        fs::write(clone_dir.path().join("a.txt"), "hello modified in branch")?;
        fs::write(
            clone_dir.path().join("sub/b.txt"),
            "world modified in branch",
        )?;
        fs::write(clone_dir.path().join("new_in_branch.txt"), "fresh file")?;

        let (head_exit, head_before, _) = run_git_test(src.path(), &["rev-parse", "HEAD"]).await;
        assert_eq!(head_exit, 0);
        // Branch mode uses the same conflict-aware delta reconciliation. It
        // must not merge unrelated history or mutate the host's Git index.
        clone_dir.merge_back().await?;
        let (_, head_after, _) = run_git_test(src.path(), &["rev-parse", "HEAD"]).await;
        assert_eq!(head_before, head_after);
        let (_, staged, _) = run_git_test(src.path(), &["diff", "--cached", "--name-only"]).await;
        assert!(staged.trim().is_empty());

        assert_eq!(
            fs::read_to_string(src.path().join("a.txt"))?,
            "hello modified in branch"
        );
        assert_eq!(
            fs::read_to_string(src.path().join("sub/b.txt"))?,
            "world modified in branch"
        );
        assert_eq!(
            fs::read_to_string(src.path().join("new_in_branch.txt"))?,
            "fresh file"
        );

        Ok(())
    }

    #[test]
    fn test_build_failover_chain_ordering_and_deduplication() {
        let providers = vec!["gemini".to_string(), "openai".to_string()];
        let registry = cade_ai::provider_registry::ProviderRegistry::configured();
        let primary_provider = registry.get("gemini").unwrap();
        let primary = format!(
            "{}/{}",
            primary_provider.name,
            primary_provider
                .fast_model
                .as_ref()
                .or(primary_provider.default_model.as_ref())
                .unwrap()
        );
        let parent = "parent-provider/explicit-parent-model";
        let chain = super::build_failover_chain(&primary, parent, &providers);

        assert_eq!(chain[0], primary);
        assert_eq!(chain[1], parent);
        let alternate = registry.get("openai").unwrap();
        for model in alternate
            .fast_model
            .iter()
            .chain(alternate.default_model.iter())
        {
            assert!(chain.contains(&format!("{}/{model}", alternate.name)));
        }
        for model in chain.iter().skip(2) {
            let (provider, _) = model.split_once('/').unwrap();
            assert!(
                providers
                    .iter()
                    .any(|p| registry.get(p).is_some_and(|def| def.name == provider))
            );
        }
        // Deduplicated
        assert_eq!(chain.iter().filter(|m| *m == &primary).count(), 1);
    }

    #[test]
    fn test_is_failover_worthy_error() {
        assert!(super::is_failover_worthy_error("HTTP 404 Not Found"));
        assert!(super::is_failover_worthy_error(
            "HTTP 429 Rate limit exceeded"
        ));
        assert!(super::is_failover_worthy_error(
            "Your credit balance is too low to access the Anthropic API"
        ));
        assert!(super::is_failover_worthy_error(
            "insufficient_quota error from provider"
        ));
        assert!(!super::is_failover_worthy_error(
            "Invalid json syntax in tool call"
        ));
    }
}
