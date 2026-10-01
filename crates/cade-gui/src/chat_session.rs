//! ChatSessionCoordinator module for cade-gui (PRD #65 / Issue #66).
//!
//! Encapsulates optimistic message insertions, SSE stream decoding,
//! reasoning block accumulation, and message ID stabilization behind a clean seam.

use cade_api_types::{ChatMessage, StreamEvent};
use dioxus::prelude::*;
use std::sync::Arc;
use std::sync::atomic::AtomicBool;

use crate::api::CadeApiClient;
use crate::types::AppState;

mod timeline;
pub use timeline::{ChatTimeline, ParsedMessageCache};

/// Questions share the pending queue but have their own answer UI.
pub fn is_tool_approval(row: &serde_json::Value) -> bool {
    row["tool_name"]
        .as_str()
        .is_some_and(|name| name != "ask_user_question")
}

/// Merge run-stream approvals into the same pending list used by the dashboard.
/// The global feed may have already delivered the request; keep one row per ID.
pub fn track_approval_event(pending: &mut Vec<serde_json::Value>, event: &StreamEvent) {
    match event.msg_type() {
        "approval_required" | "question_required" => {
            if let Some(id) = event.approval_id() {
                let mut data = event.data.clone();
                if event.msg_type() == "question_required"
                    && let Some(row) = data.as_object_mut()
                {
                    row.insert(
                        "tool_name".into(),
                        serde_json::Value::String("ask_user_question".into()),
                    );
                }
                if let Some(existing) = pending.iter_mut().find(|item| item["id"] == id) {
                    // Queue rows fetched on reconnect have string arguments and
                    // no reason. Enrich them with the live run's reviewed data.
                    if let (Some(row), Some(fields)) = (existing.as_object_mut(), data.as_object())
                    {
                        row.extend(fields.clone());
                    }
                } else {
                    pending.push(data);
                }
            }
        }
        "approval_resolved" | "question_resolved" => {
            if let Some(id) = event.approval_id() {
                pending.retain(|item| item["id"] != id);
            }
        }
        _ => {}
    }
}

// region:    --- Types

/// Outcome of a dispatched chat turn.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ChatTurnOutcome {
    Completed {
        final_message_id: String,
        content_length: usize,
        had_reasoning: bool,
        assigned_conversation_id: Option<String>,
    },
    Cancelled,
    Failed(String),
}

/// Standalone, deep coordinator managing conversation turn lifecycles and SSE streaming.
#[derive(Clone)]
pub struct ChatSessionCoordinator {
    api_client: CadeApiClient,
    agent_id: String,
    conversation_id: Option<String>,
}

impl ChatSessionCoordinator {
    pub fn new(
        api_client: CadeApiClient,
        agent_id: impl Into<String>,
        conversation_id: Option<String>,
    ) -> Self {
        Self {
            api_client,
            agent_id: agent_id.into(),
            conversation_id,
        }
    }

    /// Process an incoming stream event and update message state in-place.
    /// If the placeholder message with `stream_id` does not exist yet, it is automatically
    /// created and appended to prevent dropping events during external run following.
    pub fn apply_stream_event(
        messages: &mut Vec<ChatMessage>,
        stream_id: &str,
        event: StreamEvent,
        reasoning_acc: &mut String,
    ) {
        let idx = if let Some(i) = messages.iter().position(|m| m.id == stream_id) {
            i
        } else {
            messages.push(ChatMessage {
                id: stream_id.to_string(),
                role: "assistant".to_string(),
                content: serde_json::Value::String(String::new()),
                conversation_id: None,
            });
            messages.len() - 1
        };

        match event.msg_type() {
            "assistant_message" => {
                if let Some(delta) = event.content() {
                    let existing = messages[idx].content.as_str().unwrap_or("").to_string();
                    messages[idx].content = serde_json::Value::String(format!("{existing}{delta}"));
                }
            }
            "thought" | "reasoning_message" => {
                let r_text = event.reasoning().or_else(|| event.content()).unwrap_or("");
                if !r_text.is_empty() {
                    reasoning_acc.push_str(r_text);
                    let reasoning_block = format!("<reasoning>\n{reasoning_acc}\n</reasoning>");
                    let existing = messages[idx].content.as_str().unwrap_or("").to_string();
                    let updated = if existing.is_empty() || existing == reasoning_block {
                        reasoning_block.clone()
                    } else if let Some(tail) = existing.split("</reasoning>").nth(1) {
                        format!("{reasoning_block}{tail}")
                    } else {
                        format!("{reasoning_block}\n{existing}")
                    };
                    messages[idx].content = serde_json::Value::String(updated);
                }
            }
            "tool_call_message" | "tool_executing" => {
                let name = event
                    .tool_name()
                    .or_else(|| event.data.get("name").and_then(|v| v.as_str()))
                    .unwrap_or("tool");
                let args = event.tool_arguments();
                let existing = messages[idx].content.as_str().unwrap_or("").to_string();
                let label = if event.msg_type() == "tool_executing" {
                    "Tool Executing"
                } else {
                    "Tool call"
                };
                let tool_block = format!("\n\n[{label}: {name}]\nArguments: {args}\n");
                messages[idx].content =
                    serde_json::Value::String(format!("{existing}{tool_block}"));
            }
            "tool_result_message" | "tool_completed" => {
                let name = event.tool_name().unwrap_or("tool");
                let is_error = event
                    .tool_payload()
                    .get("is_error")
                    .and_then(|v| v.as_bool())
                    .unwrap_or(false);
                let output = event.tool_output().unwrap_or("");
                let status_label = if is_error { "Failed" } else { "Completed" };
                let ui_meta = if let Some(uri) = event
                    .tool_payload()
                    .get("ui_resource_uri")
                    .and_then(|v| v.as_str())
                {
                    format!("\n[UI Widget Resource: {uri}]\n")
                } else {
                    String::new()
                };
                let existing = messages[idx].content.as_str().unwrap_or("").to_string();
                let result_block =
                    format!("\n[Tool {status_label}: {name}]{ui_meta}\nOutput: {output}\n");
                messages[idx].content =
                    serde_json::Value::String(format!("{existing}{result_block}"));
            }
            "approval_required" => {
                if let Some(request) = event.approval_request() {
                    let existing = messages[idx].content.as_str().unwrap_or("").to_string();
                    let approval_card = format!(
                        "\n\n[Approval Required: {}] (ID: {})\n{}\nArguments: {}\n",
                        request.tool_name, request.id, request.reason, request.arguments
                    );
                    messages[idx].content =
                        serde_json::Value::String(format!("{existing}{approval_card}"));
                }
            }
            "approval_resolved" => {
                let approval_id = event.approval_id().unwrap_or("unknown");
                let approved = event
                    .data
                    .get("approved")
                    .and_then(|v| v.as_bool())
                    .unwrap_or_else(|| event.data["status"].as_str() == Some("approved"));
                let verdict_str = if approved { "Approved" } else { "Denied" };
                let existing = messages[idx].content.as_str().unwrap_or("").to_string();
                let resolved_block =
                    format!("\n[Approval Resolved: {approval_id} -> {verdict_str}]\n");
                messages[idx].content =
                    serde_json::Value::String(format!("{existing}{resolved_block}"));
            }
            "progress" => {
                let percent = event
                    .data
                    .get("percent")
                    .and_then(|v| v.as_f64())
                    .unwrap_or(0.0);
                let msg = event
                    .data
                    .get("message")
                    .and_then(|v| v.as_str())
                    .unwrap_or("");
                let existing = messages[idx].content.as_str().unwrap_or("").to_string();
                let progress_block = format!("\n[Progress: {:.0}%] {}\n", percent, msg);
                messages[idx].content =
                    serde_json::Value::String(format!("{existing}{progress_block}"));
            }
            "error" => {
                let err_msg = event.error().unwrap_or("Unknown error");
                let existing = messages[idx].content.as_str().unwrap_or("");
                messages[idx].content =
                    serde_json::Value::String(format!("{existing}\n\n[Error] {err_msg}"));
            }
            _ => {}
        }
    }

    /// Dispatches a user prompt, managing optimistic state insertions,
    /// SSE event streaming, reasoning accumulator tags, and final ID stabilization.
    pub async fn dispatch_turn(
        &self,
        prompt: &str,
        mut state: AppState,
        cancel_token: Arc<AtomicBool>,
    ) -> Result<ChatTurnOutcome, String> {
        let text = prompt.trim().to_string();
        Self::sync_selection(state);
        if state.selected_agent.peek().as_ref().map(|a| a.id.as_str())
            != Some(self.agent_id.as_str())
            || *state.active_conversation.peek() != self.conversation_id
        {
            return Ok(ChatTurnOutcome::Cancelled);
        }
        let lease = state.chat_timeline.write().begin_turn(&text)?;
        state
            .active_stream
            .set(crate::types::SafeAbortHandle(cancel_token.clone()));
        Self::publish(state);
        let result = self
            .api_client
            .stream_messages(
                &self.agent_id,
                &text,
                self.conversation_id.as_deref(),
                Some(cancel_token.clone()),
                |event: StreamEvent| {
                    Self::receive(state, lease, event);
                },
            )
            .await;
        Self::sync_selection(state);
        if !state.chat_timeline.peek().owns(lease)
            || cancel_token.load(std::sync::atomic::Ordering::Acquire)
        {
            return Ok(ChatTurnOutcome::Cancelled);
        }
        // EOF is transport state, not execution success. A known run can be
        // reattached without resubmitting the prompt or starting another run.
        if state.chat_timeline.peek().is_loading() {
            let run = state.chat_timeline.peek().run_id().map(str::to_owned);
            if let Some(run) = run {
                Self::replay(state, self.api_client.clone(), lease, run).await;
            } else {
                let error = result
                    .err()
                    .unwrap_or_else(|| "Stream closed before assigning a run.".into());
                state.chat_timeline.write().transport_failed(lease, &error);
                Self::publish(state);
                return Err(error);
            }
        }
        if !state.chat_timeline.peek().owns(lease) {
            return Ok(ChatTurnOutcome::Cancelled);
        }
        let status = state
            .chat_timeline
            .peek()
            .outcome(lease)
            .unwrap_or("unknown")
            .to_owned();
        if status.starts_with("transport_error:") {
            return Ok(ChatTurnOutcome::Failed(status));
        }
        let messages = state.chat_timeline.peek().messages().to_vec();
        let last = messages.iter().rev().find(|m| m.role == "assistant");
        let outcome = ChatTurnOutcome::Completed {
            final_message_id: last.map(|m| m.id.clone()).unwrap_or_default(),
            content_length: last.map(|m| m.text().len()).unwrap_or(0),
            had_reasoning: last.is_some_and(|m| m.text().contains("<reasoning>")),
            assigned_conversation_id: state.chat_timeline.peek().conversation().map(str::to_owned),
        };
        Self::refresh_history(state, self.api_client.clone());
        match status.as_str() {
            "done" | "completed" | "succeeded" => Ok(outcome),
            "cancelled" | "canceled" => Ok(ChatTurnOutcome::Cancelled),
            _ => Ok(ChatTurnOutcome::Failed(status)),
        }
    }

    /// Signals are presentation mirrors; all timeline mutation lives here.
    pub fn sync_selection(mut state: AppState) -> bool {
        let agent = state
            .selected_agent
            .peek()
            .as_ref()
            .map(|a| a.id.clone())
            .unwrap_or_default();
        if !state
            .chat_timeline
            .peek()
            .same_session(&state.api_key.peek())
        {
            state.pending_approvals.set(Vec::new());
        }
        let changed = state.chat_timeline.write().select(
            agent,
            state.active_conversation.peek().clone(),
            state.api_key.peek().clone(),
        );
        if changed {
            state
                .active_stream
                .peek()
                .0
                .store(true, std::sync::atomic::Ordering::Release);
            Self::publish(state);
        }
        changed
    }

    fn publish(mut state: AppState) {
        let timeline = state.chat_timeline.peek();
        let messages = timeline.messages().to_vec();
        let conversation = timeline.conversation().map(str::to_owned);
        let loading = timeline.is_loading();
        let run = if loading {
            timeline.run_id().map(str::to_owned)
        } else {
            None
        };
        drop(timeline);
        state.parsed_messages.write().retain_messages(&messages);
        state.messages.set(messages);
        state.is_loading.set(loading);
        state.active_stream_id.set(run);
        if *state.active_conversation.peek() != conversation {
            state.active_conversation.set(conversation);
        }
    }

    fn receive(mut state: AppState, lease: timeline::TurnLease, event: StreamEvent) {
        Self::sync_selection(state);
        let accepted = state.chat_timeline.write().apply(lease, event.clone());
        if accepted {
            // Publish assigned conversation before any helper synchronizes the
            // selection; otherwise adoption would be mistaken for navigation.
            Self::publish(state);
            Self::pending_event(state, &event);
            if event.is_terminal() {
                let run = event.run_id();
                for row in state.runs.write().iter_mut() {
                    if row["id"].as_str() == run {
                        row["status"] = event.data["status"].clone();
                    }
                }
            }
            if event.msg_type() == "theme_update"
                && let Some(name) = event.data["theme_name"].as_str()
            {
                let mut agent = state.selected_agent.peek().clone();
                if let Some(agent) = agent.as_mut() {
                    agent.theme = Some(name.to_owned());
                }
                state.selected_agent.set(agent);
            }
        }
    }

    pub fn pending_event(mut state: AppState, event: &StreamEvent) {
        Self::sync_selection(state);
        let mut pending = state.pending_approvals.peek().clone();
        state
            .chat_timeline
            .write()
            .pending_event(&mut pending, event);
        if pending != *state.pending_approvals.peek() {
            state.pending_approvals.set(pending);
        }
    }

    pub fn pending_snapshot(mut state: AppState, rows: Vec<serde_json::Value>) {
        Self::sync_selection(state);
        let rows = state
            .chat_timeline
            .peek()
            .pending_snapshot(rows, &state.pending_approvals.peek());
        state.pending_approvals.set(rows);
    }

    pub fn refresh_selection(mut state: AppState, client: CadeApiClient) {
        Self::sync_selection(state);
        if client.api_key.is_empty() {
            return;
        }
        Self::refresh_history(state, client.clone());
        let epoch = state.chat_timeline.peek().epoch();
        let agent = state
            .selected_agent
            .peek()
            .as_ref()
            .map(|a| a.id.clone())
            .unwrap_or_default();
        if agent.is_empty() {
            return;
        }
        spawn_forever(async move {
            if let Ok(mut conversations) = client.list_conversations(&agent).await {
                Self::sync_selection(state);
                if state.chat_timeline.peek().epoch() != epoch {
                    return;
                }
                conversations
                    .sort_by_key(|conversation| std::cmp::Reverse(conversation.updated_at));
                state.conversations.set(conversations);
            }
            if let Ok(runs) = crate::api::list_agent_runs(&agent, &client.api_key).await {
                Self::sync_selection(state);
                if state.chat_timeline.peek().epoch() != epoch {
                    return;
                }
                state.runs.set(runs.clone());
                for run in runs
                    .iter()
                    .filter(|run| run["status"].as_str() == Some("running"))
                {
                    Self::follow_run(
                        state,
                        client.clone(),
                        &agent,
                        run["conversation_id"].as_str(),
                        run["id"].as_str().unwrap_or_default(),
                    );
                }
            }
        });
    }

    pub fn refresh_history(mut state: AppState, client: CadeApiClient) {
        Self::sync_selection(state);
        if client.api_key.is_empty() {
            return;
        }
        let agent = state
            .selected_agent
            .peek()
            .as_ref()
            .map(|a| a.id.clone())
            .unwrap_or_default();
        if agent.is_empty() {
            return;
        }
        let conversation = state.active_conversation.peek().clone();
        let ticket = state.chat_timeline.write().history_ticket();
        spawn_forever(async move {
            if let Ok(rows) = client.get_messages(&agent, conversation.as_deref()).await {
                Self::sync_selection(state);
                if state.chat_timeline.write().reconcile_history(ticket, rows) {
                    Self::publish(state);
                }
            }
        });
    }

    pub fn persisted(mut state: AppState, agent: &str, message: ChatMessage) {
        Self::sync_selection(state);
        if state
            .selected_agent
            .peek()
            .as_ref()
            .is_some_and(|a| a.id == agent)
        {
            state.chat_timeline.write().persisted(message);
            Self::publish(state);
        }
    }

    pub fn follow_run(
        mut state: AppState,
        client: CadeApiClient,
        agent: &str,
        conversation: Option<&str>,
        run: &str,
    ) {
        Self::sync_selection(state);
        let lease = state
            .chat_timeline
            .write()
            .follow_run(agent, conversation, run);
        if let Some(lease) = lease {
            let run = run.to_owned();
            Self::publish(state);
            spawn_forever(async move {
                Self::replay(state, client.clone(), lease, run).await;
                if state.chat_timeline.peek().owns(lease)
                    && state
                        .chat_timeline
                        .peek()
                        .outcome(lease)
                        .is_some_and(|s| !s.starts_with("transport_error:"))
                {
                    Self::refresh_history(state, client);
                }
            });
        }
    }

    async fn replay(
        mut state: AppState,
        client: CadeApiClient,
        lease: timeline::TurnLease,
        run: String,
    ) {
        for attempt in 0..3 {
            Self::sync_selection(state);
            if !state.chat_timeline.peek().owns(lease) || !state.chat_timeline.peek().is_loading() {
                return;
            }
            let cursor = state.chat_timeline.peek().cursor(lease);
            let result = crate::api::stream_run(&client.api_key, &run, cursor, |event| {
                Self::receive(state, lease, event)
            })
            .await;
            Self::sync_selection(state);
            if !state.chat_timeline.peek().owns(lease) || !state.chat_timeline.peek().is_loading() {
                return;
            }
            if attempt == 2 {
                state.chat_timeline.write().transport_failed(
                    lease,
                    &result
                        .err()
                        .unwrap_or_else(|| "Run stream closed before run_done.".into()),
                );
                Self::publish(state);
            } else {
                gloo_timers::future::TimeoutFuture::new(1000).await;
            }
        }
    }
}

// endregion: --- Types

// region:    --- Tests

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn test_apply_stream_event_assistant_delta() {
        let stream_id = "streaming-123";
        let mut messages = vec![ChatMessage {
            id: stream_id.to_string(),
            role: "assistant".to_string(),
            content: json!("Hello"),
            conversation_id: None,
        }];
        let mut reasoning_acc = String::new();

        let event = StreamEvent {
            message_type: "assistant_message".to_string(),
            data: json!({ "content": " world!" }),
        };

        ChatSessionCoordinator::apply_stream_event(
            &mut messages,
            stream_id,
            event,
            &mut reasoning_acc,
        );

        assert_eq!(messages[0].content, json!("Hello world!"));
    }

    #[test]
    fn test_apply_stream_event_reasoning_accumulation() {
        let stream_id = "streaming-123";
        let mut messages = vec![ChatMessage {
            id: stream_id.to_string(),
            role: "assistant".to_string(),
            content: json!("Answer"),
            conversation_id: None,
        }];
        let mut reasoning_acc = String::new();

        let event = StreamEvent {
            message_type: "reasoning_message".to_string(),
            data: json!({ "reasoning": "Thinking step 1..." }),
        };

        ChatSessionCoordinator::apply_stream_event(
            &mut messages,
            stream_id,
            event,
            &mut reasoning_acc,
        );

        assert!(
            messages[0]
                .content
                .as_str()
                .unwrap()
                .contains("<reasoning>")
        );
        assert!(
            messages[0]
                .content
                .as_str()
                .unwrap()
                .contains("Thinking step 1...")
        );
    }

    #[test]
    fn test_apply_stream_event_approval_and_widget_flow() {
        let stream_id = "streaming-approval-123";
        let mut messages = vec![ChatMessage {
            id: stream_id.to_string(),
            role: "assistant".to_string(),
            content: json!("Starting task..."),
            conversation_id: None,
        }];
        let mut reasoning_acc = String::new();

        // 1. Tool Executing
        let exec_event = StreamEvent {
            message_type: "tool_executing".to_string(),
            data: json!({ "name": "delete_file", "arguments": "{\"path\": \"old.txt\"}" }),
        };
        ChatSessionCoordinator::apply_stream_event(
            &mut messages,
            stream_id,
            exec_event,
            &mut reasoning_acc,
        );
        assert!(
            messages[0]
                .content
                .as_str()
                .unwrap()
                .contains("[Tool Executing: delete_file]")
        );

        // 2. Approval Required
        let appr_event = StreamEvent {
            message_type: "approval_required".to_string(),
            data: json!({ "id": "appr-999", "tool_name": "delete_file", "arguments": {"path": "old.txt"}, "reason": "Write requires approval" }),
        };
        ChatSessionCoordinator::apply_stream_event(
            &mut messages,
            stream_id,
            appr_event,
            &mut reasoning_acc,
        );
        assert!(
            messages[0]
                .content
                .as_str()
                .unwrap()
                .contains("[Approval Required: delete_file]")
        );
        assert!(
            messages[0]
                .content
                .as_str()
                .unwrap()
                .contains("(ID: appr-999)")
        );
        assert!(
            messages[0]
                .content
                .as_str()
                .unwrap()
                .contains("Write requires approval")
        );
        assert!(messages[0].content.as_str().unwrap().contains("old.txt"));

        // 3. Approval Resolved
        let resolved_event = StreamEvent {
            message_type: "approval_resolved".to_string(),
            data: json!({ "id": "appr-999", "status": "approved" }),
        };
        ChatSessionCoordinator::apply_stream_event(
            &mut messages,
            stream_id,
            resolved_event,
            &mut reasoning_acc,
        );
        assert!(
            messages[0]
                .content
                .as_str()
                .unwrap()
                .contains("[Approval Resolved: appr-999 -> Approved]")
        );

        // 4. Tool Completed with UI Widget
        let comp_event = StreamEvent {
            message_type: "tool_completed".to_string(),
            data: json!({ "tool_name": "delete_file", "output": "File removed", "is_error": false, "ui_resource_uri": "ui://widgets/status" }),
        };
        ChatSessionCoordinator::apply_stream_event(
            &mut messages,
            stream_id,
            comp_event,
            &mut reasoning_acc,
        );
        assert!(
            messages[0]
                .content
                .as_str()
                .unwrap()
                .contains("[Tool Completed: delete_file]")
        );
        assert!(
            messages[0]
                .content
                .as_str()
                .unwrap()
                .contains("[UI Widget Resource: ui://widgets/status]")
        );
    }

    #[test]
    fn canonical_run_approval_is_actionable_and_deduplicated_with_global_feed() {
        // Exact envelope fields produced by SseApprovalDelegate, decoded by the
        // same StreamEvent parser used by the browser's run SSE transport.
        let wire = json!({
            "message_type": "approval_required", "id": "app-real-42",
            "agent_id": "agent-1", "run_id": "run-1", "tool_call_id": "tc-1",
            "tool_name": "write_file", "arguments": {"path": "notes.txt", "content": "hello"},
            "reason": "Write requires approval"
        });
        let event: StreamEvent = serde_json::from_value(wire.clone()).unwrap();
        let request = event
            .approval_request()
            .expect("web can review the request");
        assert_eq!(request.id, "app-real-42");
        assert_eq!(request.tool_name, "write_file");
        assert_eq!(request.arguments["path"], "notes.txt");
        assert_eq!(request.reason, "Write requires approval");

        let mut pending = vec![json!({"id": "app-real-42", "agent_id": "agent-1"})];
        track_approval_event(&mut pending, &event);
        assert_eq!(pending.len(), 1, "global event and run event share an ID");
        assert_eq!(pending[0]["reason"], "Write requires approval");
        pending.clear();
        track_approval_event(&mut pending, &event);
        assert_eq!(pending[0]["id"], "app-real-42");
        assert_eq!(pending[0]["arguments"]["content"], "hello");

        let resolved: StreamEvent = serde_json::from_value(json!({
            "message_type": "approval_resolved", "id": "app-real-42"
        }))
        .unwrap();
        track_approval_event(&mut pending, &resolved);
        assert!(pending.is_empty());
    }

    #[test]
    fn queue_questions_do_not_get_tool_approval_controls() {
        assert!(!is_tool_approval(
            &json!({"tool_name": "ask_user_question", "id": "q-1"})
        ));
        assert!(is_tool_approval(
            &json!({"tool_name": "write_file", "id": "app-1", "reason": "Review write"})
        ));
    }

    #[test]
    fn test_chat_turn_outcome_assigned_conversation_id() {
        let outcome = ChatTurnOutcome::Completed {
            final_message_id: "msg-123".to_string(),
            content_length: 42,
            had_reasoning: true,
            assigned_conversation_id: Some("conv-auto-titled-456".to_string()),
        };

        if let ChatTurnOutcome::Completed {
            assigned_conversation_id,
            had_reasoning,
            ..
        } = outcome
        {
            assert_eq!(
                assigned_conversation_id.as_deref(),
                Some("conv-auto-titled-456")
            );
            assert!(had_reasoning);
        } else {
            panic!("Expected Completed variant");
        }
    }
}

// endregion: --- Tests
