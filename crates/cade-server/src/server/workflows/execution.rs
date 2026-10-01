use super::{WorkflowDef, WorkflowDispatch, WorkflowEngine};
use crate::server::api::run::runtime::RunRequest;
use cade_api_types::{WorkflowStatus, WorkflowStepEvent};
use cade_store::sqlite::{self, Db, WorkflowRunRecord};
use serde_json::{Value, json};
use std::collections::HashMap;
use std::sync::{Arc, OnceLock};
use tokio::sync::{Mutex, RwLock, broadcast, oneshot};

#[derive(Default)]
struct RunControl {
    cancelled: bool,
    active_run: Option<String>,
    finished: bool,
}
type WorkflowStartSender = oneshot::Sender<Result<WorkflowDispatch, String>>;

struct ActiveWorkflow {
    control: Mutex<RunControl>,
    events: broadcast::Sender<WorkflowStepEvent>,
    started: Mutex<Option<WorkflowStartSender>>,
}
static ACTIVE: OnceLock<RwLock<HashMap<String, Arc<ActiveWorkflow>>>> = OnceLock::new();
fn active() -> &'static RwLock<HashMap<String, Arc<ActiveWorkflow>>> {
    ACTIVE.get_or_init(|| RwLock::new(HashMap::new()))
}

pub(super) async fn subscribe(run_id: &str) -> Option<broadcast::Receiver<WorkflowStepEvent>> {
    active()
        .read()
        .await
        .get(run_id)
        .map(|run| run.events.subscribe())
}

pub(super) async fn cancel(db: &Db, run_id: &str) -> Result<bool, String> {
    let Some(run) = active().read().await.get(run_id).cloned() else {
        return Ok(false);
    };
    let mut control = run.control.lock().await;
    if control.finished {
        return Ok(false);
    }
    if let Some(agent_run) = &control.active_run {
        sqlite::request_run_cancellation(db, agent_run).map_err(|error| error.to_string())?;
    }
    sqlite::update_workflow_run_status(db, run_id, "cancelling", None, None)
        .map_err(|error| error.to_string())?;
    control.cancelled = true;
    Ok(true)
}

pub(super) async fn dispatch(
    engine: WorkflowEngine,
    workflow: WorkflowDef,
    params: Value,
) -> Result<(WorkflowDispatch, broadcast::Receiver<WorkflowStepEvent>), String> {
    let order = workflow.order()?;
    let agents = sqlite::list_agents(&engine.db).map_err(|error| error.to_string())?;
    let mut agent_ids = Vec::new();
    for step in &workflow.steps {
        let reference = step
            .agent
            .as_deref()
            .ok_or("Workflow step requires an agent")?;
        let id = if let Some(agent) = agents.iter().find(|agent| agent.id == reference) {
            agent.id.clone()
        } else {
            let matches: Vec<_> = agents
                .iter()
                .filter(|agent| agent.name == reference)
                .collect();
            if matches.len() != 1 {
                return Err(format!("Unknown or ambiguous workflow agent: {reference}"));
            }
            matches[0].id.clone()
        };
        agent_ids.push(id);
    }
    let run_id = format!("wfrun-{}", uuid::Uuid::new_v4());
    sqlite::create_workflow_run(
        &engine.db,
        &WorkflowRunRecord {
            run_id: run_id.clone(),
            workflow_name: workflow.name.clone(),
            status: "running".into(),
            current_step: 0,
            total_steps: workflow.steps.len(),
            params_json: Some(params.to_string()),
            error: None,
            created_at: chrono::Utc::now().timestamp(),
            completed_at: None,
        },
    )
    .map_err(|error| error.to_string())?;
    let (events, receiver) = broadcast::channel(128);
    let (started_tx, started_rx) = oneshot::channel();
    let run = Arc::new(ActiveWorkflow {
        control: Mutex::new(RunControl::default()),
        events,
        started: Mutex::new(Some(started_tx)),
    });
    active().write().await.insert(run_id.clone(), run.clone());
    let task_id = run_id.clone();
    tokio::spawn(async move {
        let outcome = execute(
            &engine, &workflow, &params, &order, &agent_ids, &task_id, &run,
        )
        .await;
        let mut control = run.control.lock().await;
        control.active_run = None;
        let (status, error) = match outcome {
            Ok(WorkflowStatus::Cancelled) => ("cancelled", None),
            Ok(_) if control.cancelled => ("cancelled", None),
            Ok(_) => ("succeeded", None),
            Err(error) => ("failed", Some(error)),
        };
        if let Err(error) = sqlite::update_workflow_run_status(
            &engine.db,
            &task_id,
            status,
            error.as_deref(),
            Some(chrono::Utc::now().timestamp()),
        ) {
            tracing::error!(%error, run_id = %task_id, "failed to persist workflow terminal outcome");
        }
        control.finished = true;
        drop(control);
        // Setup failure must not acknowledge a fictitious canonical execution.
        if let Some(started) = run.started.lock().await.take() {
            let _ =
                started.send(Err(error.unwrap_or_else(|| {
                    "Workflow stopped before accepting an agent run".into()
                })));
        }
        active().write().await.remove(&task_id);
    });
    let accepted = started_rx
        .await
        .map_err(|_| "Workflow task ended before accepting an agent run".to_owned())??;
    Ok((accepted, receiver))
}

fn event(
    run: &ActiveWorkflow,
    run_id: &str,
    workflow: &WorkflowDef,
    index: usize,
    status: WorkflowStatus,
    output: Option<String>,
    error: Option<String>,
) {
    let _ = run.events.send(WorkflowStepEvent {
        run_id: run_id.into(),
        workflow_name: workflow.name.clone(),
        step_index: index,
        step_name: workflow.steps[index].name.clone(),
        status,
        output_chunk: output,
        error,
    });
}

async fn execute(
    engine: &WorkflowEngine,
    workflow: &WorkflowDef,
    params: &Value,
    order: &[usize],
    agent_ids: &[String],
    run_id: &str,
    run: &ActiveWorkflow,
) -> Result<WorkflowStatus, String> {
    let mut outputs = HashMap::<String, String>::new();
    for (position, &index) in order.iter().enumerate() {
        let mut control = run.control.lock().await;
        if control.cancelled {
            for &pending in &order[position..] {
                event(
                    run,
                    run_id,
                    workflow,
                    pending,
                    WorkflowStatus::Skipped,
                    None,
                    Some("Workflow cancelled".into()),
                );
            }
            return Ok(WorkflowStatus::Cancelled);
        }
        sqlite::update_workflow_run_step(&engine.db, run_id, index)
            .map_err(|error| error.to_string())?;
        let step = &workflow.steps[index];
        let dependencies: HashMap<_, _> = step
            .depends_on
            .iter()
            .map(|name| (name, &outputs[name]))
            .collect();
        let input = format!(
            "{}\n\nWorkflow parameters:\n{}\n\nDependency outputs:\n{}",
            step.prompt,
            params,
            json!(dependencies)
        );
        // Concurrent workflows using the same agent must not share the default
        // message timeline or consume another run's webhook payload.
        let conversation = sqlite::create_conversation(
            &engine.db,
            &agent_ids[index],
            &format!("{} / {} / {run_id}", workflow.name, step.name),
        )
        .map_err(|error| error.to_string())?;
        let mut handle = engine
            .runtime
            .try_start(RunRequest {
                agent_id: agent_ids[index].clone(),
                conversation_id: Some(conversation.id),
                input,
                // Canonical permission policy applies; webhook input cannot bypass it.
                permission_mode: None,
            })
            .await
            .map_err(|error| error.to_string())?;
        control.active_run = Some(handle.run_id.clone());
        if let Some(started) = run.started.lock().await.take() {
            let _ = started.send(Ok(WorkflowDispatch {
                run_id: run_id.into(),
                execution_id: handle.run_id.clone(),
                agent_id: agent_ids[index].clone(),
            }));
        }
        drop(control);
        event(
            run,
            run_id,
            workflow,
            index,
            WorkflowStatus::Running,
            None,
            None,
        );
        let mut output = String::new();
        let mut runtime_error = None;
        // Never drop this receiver on cancellation: the canonical loop owns
        // finalization and durable cancellation, independent of presentation.
        while let Some(Ok(envelope)) = handle.events.recv().await {
            let Ok(payload) = serde_json::from_str::<Value>(&envelope.data) else {
                continue;
            };
            match payload["message_type"].as_str() {
                Some("assistant_message") => {
                    if let Some(chunk) = payload["content"].as_str() {
                        output.push_str(chunk);
                        event(
                            run,
                            run_id,
                            workflow,
                            index,
                            WorkflowStatus::Running,
                            Some(chunk.to_owned()),
                            None,
                        );
                    }
                }
                Some("error") => {
                    runtime_error = payload["error"].as_str().map(String::from);
                }
                _ => {}
            }
        }
        let mut control = run.control.lock().await;
        control.active_run = None;
        let canonical = sqlite::get_run(&engine.db, &handle.run_id)
            .map_err(|error| error.to_string())?
            .ok_or_else(|| format!("Canonical runtime run {} was not persisted", handle.run_id))?;
        let (status, failure) = match canonical.status.as_str() {
            "done" if !control.cancelled && runtime_error.is_none() => {
                (WorkflowStatus::Succeeded, None)
            }
            "done" | "cancelled" if control.cancelled || canonical.status == "cancelled" => {
                (WorkflowStatus::Cancelled, None)
            }
            _ => (
                WorkflowStatus::Failed,
                Some(runtime_error.unwrap_or_else(|| {
                    format!(
                        "Agent run {} ended with status '{}'",
                        handle.run_id, canonical.status
                    )
                })),
            ),
        };
        // Once the last canonical outcome is known, cancellation must not
        // retroactively replace an already completed workflow's outcome.
        control.finished = position + 1 == order.len() || status != WorkflowStatus::Succeeded;
        drop(control);
        event(
            run,
            run_id,
            workflow,
            index,
            status,
            Some(output.clone()),
            failure.clone(),
        );
        if status != WorkflowStatus::Succeeded {
            for &pending in &order[position + 1..] {
                event(
                    run,
                    run_id,
                    workflow,
                    pending,
                    WorkflowStatus::Skipped,
                    None,
                    Some("Workflow stopped before this step".into()),
                );
            }
            return match failure {
                Some(error) => Err(error),
                None => Ok(WorkflowStatus::Cancelled),
            };
        }
        outputs.insert(step.name.clone(), output);
    }
    Ok(WorkflowStatus::Succeeded)
}
