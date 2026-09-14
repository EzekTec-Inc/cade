//! Server-owned lifecycle entry point for durable agent runs.
//!
//! HTTP/SSE routes and future in-process transports use this module to start
//! runs. The agentic loop remains in the parent module while this interface
//! owns request-side lifecycle work: activity tracking, user-message
//! persistence, run creation, global lifecycle publication, and event-channel
//! construction.

use axum::response::sse::Event;
use cade_store::sqlite;
use serde_json::json;

use super::{detect_theme_cmd, maybe_set_conv_title, persist, run_agent_loop};
use crate::server::state::AppState;

/// Input required to start one server-owned agent run.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RunRequest {
    pub agent_id: String,
    pub conversation_id: Option<String>,
    pub input: String,
}

/// Handle returned when a durable agent run has been accepted.
pub struct RunHandle {
    pub run_id: String,
    pub events: tokio::sync::mpsc::Receiver<Result<Event, std::convert::Infallible>>,
}

/// Small server-owned interface for beginning the canonical agentic loop.
///
/// The runtime owns durable run setup and the loop task. Transports own only
/// how they expose the returned ordered event receiver to their callers.
#[derive(Clone)]
pub struct ServerAgentRuntime {
    state: AppState,
}

impl ServerAgentRuntime {
    pub fn new(state: AppState) -> Self {
        Self { state }
    }

    /// Persist the request, create a durable run, and begin the agentic loop.
    pub async fn start(&self, request: RunRequest) -> RunHandle {
        update_activity(
            &self.state,
            &request.agent_id,
            request.conversation_id.clone(),
        )
        .await;

        let theme_cmd = detect_theme_cmd(&request.input);
        if theme_cmd.is_none() {
            if let Some(conversation_id) = request.conversation_id.as_deref() {
                maybe_set_conv_title(&self.state, conversation_id, &request.input);
            }
            persist(
                &self.state,
                &request.agent_id,
                request.conversation_id.as_deref(),
                "user",
                json!({ "content": request.input }),
            );
        }

        let run_id = make_run_id(
            &self.state,
            &request.agent_id,
            request.conversation_id.as_deref(),
        );
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
        tokio::spawn(run_agent_loop(
            self.state.clone(),
            request.agent_id,
            request.conversation_id,
            run_id.clone(),
            theme_cmd,
            events,
            request.input,
        ));

        RunHandle {
            run_id,
            events: receiver,
        }
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

/// Create a durable run record, falling back to a local identifier if storage
/// is unavailable so the caller can still observe a terminal failure event.
fn make_run_id(state: &AppState, agent_id: &str, conversation_id: Option<&str>) -> String {
    sqlite::create_run(&state.db, agent_id, conversation_id)
        .map(|run| run.id)
        .unwrap_or_else(|_| format!("run-local-{}", chrono::Utc::now().timestamp()))
}
