//! Pure, provider-independent authority for the browser's selected timeline.
use cade_api_types::{ChatMessage, StreamEvent};
use std::collections::{BTreeSet, HashMap, HashSet};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TurnLease {
    epoch: u64,
    turn: u64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct HistoryTicket {
    epoch: u64,
    request: u64,
    revision: u64,
}

#[derive(Clone)]
struct Turn {
    lease: TurnLease,
    run_id: Option<String>,
    message_id: String,
    user_id: Option<String>,
    baseline: HashSet<String>,
    reasoning: String,
    seen: BTreeSet<i64>,
    local: bool,
    terminal: bool,
    outcome: Option<String>,
    projection_reconciled: bool,
}

#[derive(Clone, Default)]
pub struct ChatTimeline {
    agent: String,
    conversation: Option<String>,
    session: String,
    epoch: u64,
    next_turn: u64,
    history_request: u64,
    revision: u64,
    messages: Vec<ChatMessage>,
    awaiting_assignment: Vec<ChatMessage>,
    turn: Option<Turn>,
    completed: HashSet<String>,
    resolved_requests: HashSet<String>,
}

impl ChatTimeline {
    /// Navigation invalidates every outstanding callback, even A → B → A.
    /// Adoption of the POST's assigned conversation is done by `apply`, so it
    /// does not look like navigation or abort its own stream.
    pub fn select(&mut self, agent: String, conversation: Option<String>, session: String) -> bool {
        if self.agent == agent && self.conversation == conversation && self.session == session {
            return false;
        }
        self.agent = agent;
        self.conversation = conversation;
        if self.session != session {
            self.resolved_requests.clear();
        }
        self.session = session;
        self.epoch += 1;
        self.revision += 1;
        self.messages.clear();
        self.awaiting_assignment.clear();
        self.turn = None;
        self.completed.clear();
        true
    }

    pub fn messages(&self) -> &[ChatMessage] {
        &self.messages
    }
    pub fn epoch(&self) -> u64 {
        self.epoch
    }
    pub fn same_session(&self, session: &str) -> bool {
        self.session == session
    }
    pub fn conversation(&self) -> Option<&str> {
        self.conversation.as_deref()
    }
    pub fn run_id(&self) -> Option<&str> {
        self.turn.as_ref().and_then(|t| t.run_id.as_deref())
    }
    pub fn is_loading(&self) -> bool {
        self.turn.as_ref().is_some_and(|t| !t.terminal)
    }
    pub fn is_live_message(&self, id: &str) -> bool {
        self.turn
            .as_ref()
            .is_some_and(|t| !t.terminal && t.message_id == id)
    }
    pub fn owns(&self, lease: TurnLease) -> bool {
        self.turn.as_ref().is_some_and(|t| t.lease == lease)
    }
    pub fn cursor(&self, lease: TurnLease) -> Option<i64> {
        self.turn
            .as_ref()
            .filter(|t| t.lease == lease)
            .and_then(|t| t.seen.last().copied())
    }
    pub fn outcome(&self, lease: TurnLease) -> Option<&str> {
        self.turn
            .as_ref()
            .filter(|t| t.lease == lease)
            .and_then(|t| t.outcome.as_deref())
    }

    pub fn pending_event(&mut self, pending: &mut Vec<serde_json::Value>, event: &StreamEvent) {
        if let Some(id) = event.approval_id() {
            if matches!(event.msg_type(), "approval_resolved" | "question_resolved") {
                self.resolved_requests.insert(id.to_owned());
            }
            if self.resolved_requests.contains(id) {
                pending.retain(|row| row["id"].as_str() != Some(id));
                return;
            }
        }
        super::track_approval_event(pending, event);
    }

    pub fn pending_snapshot(
        &self,
        rows: Vec<serde_json::Value>,
        previous: &[serde_json::Value],
    ) -> Vec<serde_json::Value> {
        rows.into_iter()
            .filter(|row| {
                row["id"]
                    .as_str()
                    .is_some_and(|id| !self.resolved_requests.contains(id))
            })
            .map(|mut row| {
                if let Some(live) = previous.iter().find(|live| live["id"] == row["id"])
                    && let (Some(fields), Some(details)) = (row.as_object_mut(), live.as_object())
                {
                    fields.extend(details.clone());
                }
                row
            })
            .collect()
    }

    pub fn begin_turn(&mut self, prompt: &str) -> Result<TurnLease, String> {
        if self.agent.is_empty() || prompt.trim().is_empty() {
            return Err("Select an agent and enter a message.".into());
        }
        if self.is_loading() {
            return Err("A run is already active in this conversation.".into());
        }
        self.awaiting_assignment.clear();
        let lease = self.start(None, true);
        let id = format!("optimistic-user-{}-{}", lease.epoch, lease.turn);
        self.messages.push(ChatMessage {
            id: id.clone(),
            role: "user".into(),
            content: prompt.trim().into(),
            conversation_id: self.conversation.clone(),
        });
        self.turn.as_mut().unwrap().user_id = Some(id);
        Ok(lease)
    }

    /// POST reserves ownership before its first await. The global notification
    /// must never open a second reader for that local turn, nor change selection.
    pub fn follow_run(
        &mut self,
        agent: &str,
        conversation: Option<&str>,
        run: &str,
    ) -> Option<TurnLease> {
        if run.is_empty()
            || self.agent != agent
            || self.conversation.as_deref() != conversation
            || self.is_loading()
            || self.completed.contains(run)
        {
            return None;
        }
        self.awaiting_assignment.clear();
        let user = self.messages.iter().rposition(|m| m.role == "user");
        if let Some(index) = user {
            self.messages.truncate(index + 1);
        }
        let user_id = user.map(|index| self.messages[index].id.clone());
        let lease = self.start(Some(run.to_owned()), false);
        self.turn.as_mut().unwrap().user_id = user_id;
        Some(lease)
    }

    fn start(&mut self, run_id: Option<String>, local: bool) -> TurnLease {
        self.next_turn += 1;
        self.revision += 1;
        let lease = TurnLease {
            epoch: self.epoch,
            turn: self.next_turn,
        };
        self.turn = Some(Turn {
            lease,
            run_id,
            message_id: format!("streaming-{}-{}", lease.epoch, lease.turn),
            user_id: None,
            baseline: self.messages.iter().map(|m| m.id.clone()).collect(),
            reasoning: String::new(),
            seen: BTreeSet::new(),
            local,
            terminal: false,
            outcome: None,
            projection_reconciled: false,
        });
        lease
    }

    pub fn apply(&mut self, lease: TurnLease, event: StreamEvent) -> bool {
        let mut adopted = false;
        let Some(turn) = self
            .turn
            .as_mut()
            .filter(|t| t.lease == lease && !t.terminal)
        else {
            return false;
        };
        if event.data["agent_id"]
            .as_str()
            .is_some_and(|id| id != self.agent)
        {
            return false;
        }
        if let Some(run) = event.run_id()
            && turn.run_id.as_deref().is_some_and(|id| id != run)
        {
            return false;
        }
        if let Some(conv) = event.conversation_id() {
            if turn.local && self.conversation.is_none() && event.msg_type() == "stream_start" {
                self.conversation = Some(conv.to_owned());
                adopted = true;
                // Unscoped history is not part of the server-assigned thread.
                self.messages.retain(|message| {
                    turn.user_id.as_deref() == Some(message.id.as_str())
                        || message.id == turn.message_id
                });
                for message in &mut self.messages {
                    message.conversation_id = self.conversation.clone();
                }
                turn.baseline.clear();
            } else if self.conversation.as_deref() != Some(conv) {
                return false;
            }
        }
        if let Some(seq) = event.seq_id()
            && !turn.seen.insert(seq)
        {
            return false;
        }
        if let Some(run) = event.run_id() {
            turn.run_id = Some(run.to_owned());
        }
        if event.is_terminal() {
            turn.terminal = true;
            turn.outcome = Some(event.data["status"].as_str().unwrap_or("done").to_owned());
            if let Some(run) = &turn.run_id {
                self.completed.insert(run.clone());
            }
        } else if matches!(
            event.msg_type(),
            "assistant_message"
                | "thought"
                | "reasoning_message"
                | "tool_call_message"
                | "tool_executing"
                | "tool_result_message"
                | "tool_completed"
                | "error"
        ) {
            super::ChatSessionCoordinator::apply_stream_event(
                &mut self.messages,
                &turn.message_id,
                event,
                &mut turn.reasoning,
            );
            if let Some(message) = self.messages.iter_mut().find(|m| m.id == turn.message_id) {
                message.conversation_id = self.conversation.clone();
            }
        }
        self.revision += 1;
        if adopted {
            for message in std::mem::take(&mut self.awaiting_assignment) {
                self.persisted(message);
            }
        }
        true
    }

    pub fn transport_failed(&mut self, lease: TurnLease, error: &str) {
        if !self.owns(lease) {
            return;
        }
        let event = StreamEvent {
            message_type: "error".into(),
            data: serde_json::json!({"error": error}),
        };
        self.apply(lease, event);
        if let Some(turn) = &mut self.turn {
            turn.terminal = true;
            turn.outcome = Some(format!("transport_error: {error}"));
        }
    }

    pub fn history_ticket(&mut self) -> HistoryTicket {
        self.history_request += 1;
        HistoryTicket {
            epoch: self.epoch,
            request: self.history_request,
            revision: self.revision,
        }
    }

    pub fn reconcile_history(&mut self, ticket: HistoryTicket, rows: Vec<ChatMessage>) -> bool {
        if ticket.epoch != self.epoch || ticket.request != self.history_request {
            return false;
        }
        if !self.is_loading() && ticket.revision == self.revision {
            self.messages = unique_rows(rows, self.conversation.as_deref());
            // Keep the lease/outcome until the transport owner returns. A
            // terminal history response must not turn completion into cancel.
            if let Some(turn) = &mut self.turn {
                turn.projection_reconciled = true;
                turn.baseline = self.messages.iter().map(|m| m.id.clone()).collect();
            }
        } else if self.is_loading() {
            // A snapshot fetched before the latest event may fill the persisted
            // prefix, but cannot replace optimistic or newly persisted turns.
            let rows = unique_rows(rows, self.conversation.as_deref());
            let turn = self.turn.as_ref().unwrap();
            let user = turn
                .user_id
                .as_ref()
                .and_then(|id| self.messages.iter().find(|m| &m.id == id));
            let anchor = user
                .and_then(|user| {
                    rows.iter().rposition(|row| {
                        row.id == user.id
                            || (!turn.baseline.contains(&row.id)
                                && row.role == "user"
                                && row.text() == user.text())
                    })
                })
                .or_else(|| {
                    if !turn.local {
                        rows.iter().rposition(|row| row.role == "user")
                    } else {
                        None
                    }
                });
            let prefix_end = anchor.unwrap_or(rows.len());
            let prefix = rows[..prefix_end].to_vec();
            let current_user = turn.user_id.clone();
            let live_id = turn.message_id.clone();
            let old_baseline = turn.baseline.clone();
            let mut baseline: Vec<_> = prefix.iter().map(|m| m.id.clone()).collect();
            baseline.extend(turn.baseline.iter().cloned());
            self.turn.as_mut().unwrap().baseline.extend(baseline);
            let mut existing = std::mem::take(&mut self.messages);
            self.messages = prefix;
            existing.retain(|m| {
                (!old_baseline.contains(&m.id)
                    || current_user.as_deref() == Some(m.id.as_str())
                    || m.id == live_id)
                    && !self.messages.iter().any(|row| row.id == m.id)
            });
            self.messages.extend(existing);
            if let Some(index) = anchor {
                self.persisted(rows[index].clone());
                if !self.turn.as_ref().unwrap().local {
                    self.turn.as_mut().unwrap().user_id = Some(rows[index].id.clone());
                }
            }
        } else {
            // A newer global persistence notification wins over this snapshot.
            for row in rows {
                if !self.messages.iter().any(|m| m.id == row.id) {
                    self.persisted(row);
                }
            }
        }
        true
    }

    pub fn persisted(&mut self, row: ChatMessage) {
        if row.conversation_id != self.conversation {
            // User persistence can precede stream_start. Hold possible local
            // echoes until the POST itself supplies authoritative scope.
            if self.conversation.is_none()
                && self
                    .turn
                    .as_ref()
                    .is_some_and(|turn| turn.local && !turn.terminal)
                && row.role == "user"
                && self
                    .turn
                    .as_ref()
                    .and_then(|turn| turn.user_id.as_ref())
                    .is_some_and(|id| {
                        self.messages
                            .iter()
                            .any(|message| &message.id == id && message.text() == row.text())
                    })
                && !self
                    .awaiting_assignment
                    .iter()
                    .any(|message| message.id == row.id)
            {
                self.awaiting_assignment.push(row);
            }
            return;
        }
        if let Some(index) = self.messages.iter().position(|m| m.id == row.id) {
            self.messages[index] = row;
            self.revision += 1;
            return;
        }
        if let Some(turn) = &mut self.turn {
            if let Some(user_id) = &turn.user_id
                && user_id.starts_with("optimistic-user-")
                && !turn.baseline.contains(&row.id)
                && let Some(index) = self
                    .messages
                    .iter()
                    .position(|m| &m.id == user_id && row.role == "user" && m.text() == row.text())
            {
                turn.user_id = Some(row.id.clone());
                self.messages[index] = row;
                self.revision += 1;
                return;
            }
            // Current run output is represented by one live projection until a
            // terminal history snapshot supplies its durable multi-row form.
            if !turn.projection_reconciled
                && !turn.baseline.contains(&row.id)
                && matches!(row.role.as_str(), "assistant" | "tool")
            {
                return;
            }
        }
        let insertion = self
            .turn
            .as_ref()
            .and_then(|t| self.messages.iter().position(|m| m.id == t.message_id))
            .unwrap_or(self.messages.len());
        self.messages.insert(insertion, row);
        self.revision += 1;
    }
}

fn unique_rows(rows: Vec<ChatMessage>, conversation: Option<&str>) -> Vec<ChatMessage> {
    let mut seen = HashSet::new();
    rows.into_iter()
        .filter(|m| m.conversation_id.as_deref() == conversation && seen.insert(m.id.clone()))
        .collect()
}

/// Cache entries are valid for their complete source, not merely their ID.
/// Scope/lifetime pruning happens when the coordinator publishes a timeline.
#[derive(Clone, Default, PartialEq)]
pub struct ParsedMessageCache {
    entries: HashMap<String, (String, String, Option<String>)>,
}

impl ParsedMessageCache {
    pub fn retain_messages(&mut self, messages: &[ChatMessage]) {
        self.entries.retain(|id, (source, _, _)| {
            messages.iter().any(|m| &m.id == id && m.text() == *source)
        });
    }

    pub fn parse(&mut self, id: &str, source: &str, live: bool) -> (String, Option<String>) {
        if !live
            && let Some((cached_source, text, reasoning)) = self.entries.get(id)
            && cached_source == source
        {
            return (text.clone(), reasoning.clone());
        }
        let parsed = source
            .find("<reasoning>")
            .and_then(|start| {
                let end = source[start + 11..].find("</reasoning>")? + start + 11;
                Some((
                    format!("{}{}", &source[..start], &source[end + 12..])
                        .trim()
                        .to_owned(),
                    Some(source[start + 11..end].trim().to_owned()),
                ))
            })
            .unwrap_or_else(|| (source.to_owned(), None));
        if !live {
            self.entries.insert(
                id.to_owned(),
                (source.to_owned(), parsed.0.clone(), parsed.1.clone()),
            );
        } else {
            self.entries.remove(id);
        }
        parsed
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn selected(conversation: Option<&str>) -> ChatTimeline {
        let mut timeline = ChatTimeline::default();
        timeline.select(
            "agent-a".into(),
            conversation.map(str::to_owned),
            "browser-session".into(),
        );
        timeline
    }
    fn event(value: serde_json::Value) -> StreamEvent {
        serde_json::from_value(value).unwrap()
    }
    fn row(id: &str, role: &str, text: &str, conversation: &str) -> ChatMessage {
        serde_json::from_value(
            json!({"id":id,"role":role,"content":{"content":text},"conversation_id":conversation}),
        )
        .unwrap()
    }

    #[test]
    fn coordinator_production_post_and_global_follow_have_one_authority() {
        let mut timeline = selected(None);
        let lease = timeline.begin_turn("hello").unwrap();
        // The server persists user input, then emits run_started, before the
        // POST reader sees stream_start. Neither event starts another reader.
        timeline.persisted(row("user-db", "user", "hello", "conv-new"));
        let global = event(
            json!({"event_type":"run_started","seq":23,"data":{"agent_id":"agent-a","conversation_id":"conv-new","run_id":"run-a"}}),
        );
        assert!(
            timeline
                .follow_run(
                    global.data["agent_id"].as_str().unwrap(),
                    global.conversation_id(),
                    global.run_id().unwrap()
                )
                .is_none()
        );
        assert!(timeline.apply(
            lease,
            event(
                json!({"message_type":"stream_start","run_id":"run-a","conversation_id":"conv-new"})
            )
        ));
        assert_eq!(timeline.conversation(), Some("conv-new"));
        assert!(
            !timeline.select(
                "agent-a".into(),
                Some("conv-new".into()),
                "browser-session".into()
            ),
            "assigned conversation is not navigation"
        );
        assert!(timeline.apply(lease, event(json!({"message_type":"assistant_message","run_id":"run-a","seq_id":1,"content":"Hello"}))));
        timeline.persisted(row("assistant-db", "assistant", "Hello", "conv-new"));
        assert_eq!(timeline.messages().len(), 2);
        assert_eq!(timeline.messages()[0].id, "user-db");
        assert_eq!(timeline.messages()[1].text(), "Hello");
        assert!(
            timeline
                .follow_run("agent-a", Some("conv-new"), "run-a")
                .is_none()
        );
        timeline.apply(
            lease,
            event(json!({"message_type":"run_done","run_id":"run-a","seq_id":2,"status":"done"})),
        );
        let ticket = timeline.history_ticket();
        let rows = cade_api_types::decode_list::<ChatMessage>(&json!({"messages":[row("user-db","user","hello","conv-new"),row("assistant-db","assistant","Hello","conv-new")],"has_more":false}).to_string(), "messages").unwrap();
        assert!(timeline.reconcile_history(ticket, rows));
        assert_eq!(timeline.messages()[1].id, "assistant-db");
        assert!(
            timeline.owns(lease),
            "terminal persistence cannot invalidate its transport owner"
        );
        assert_eq!(timeline.outcome(lease), Some("done"));
        assert!(
            timeline
                .follow_run("agent-a", Some("conv-new"), "run-a")
                .is_none(),
            "finished run notifications cannot reopen the run"
        );
    }

    #[test]
    fn coordinator_replay_deduplicates_run_sequence_and_rejects_foreign_scope() {
        let mut timeline = selected(Some("conv-a"));
        let lease = timeline
            .follow_run("agent-a", Some("conv-a"), "run-a")
            .unwrap();
        let first =
            json!({"message_type":"assistant_message","run_id":"run-a","seq_id":1,"content":"A"});
        assert!(timeline.apply(lease, event(first.clone())));
        assert!(!timeline.apply(lease, event(first)));
        assert!(!timeline.apply(lease, event(json!({"message_type":"assistant_message","run_id":"run-other","seq_id":2,"content":"wrong"}))));
        assert!(!timeline.apply(lease, event(json!({"message_type":"assistant_message","run_id":"run-a","conversation_id":"conv-other","seq_id":2,"content":"wrong"}))));
        assert!(!timeline.apply(lease, event(json!({"message_type":"assistant_message","run_id":"run-a","agent_id":"agent-other","seq_id":2,"content":"wrong"}))));
        assert!(timeline.apply(lease, event(json!({"message_type":"assistant_message","run_id":"run-a","seq_id":2,"content":"B"}))));
        assert_eq!(timeline.cursor(lease), Some(2));
        assert_eq!(timeline.messages()[0].text(), "AB");
        timeline.apply(lease, event(json!({"message_type":"finish_reason","reason":"tool_calls","run_id":"run-a","seq_id":3})));
        assert!(
            timeline.is_loading(),
            "provider finish reasons are not run completion"
        );
        timeline.apply(
            lease,
            event(
                json!({"message_type":"run_done","run_id":"run-a","seq_id":4,"status":"cancelled"}),
            ),
        );
        assert!(!timeline.is_loading());
        assert_eq!(timeline.outcome(lease), Some("cancelled"));
        assert!(!timeline.apply(lease, event(json!({"message_type":"assistant_message","run_id":"run-a","seq_id":5,"content":"late"}))));
    }

    #[test]
    fn coordinator_navigation_invalidates_history_and_callbacks_even_after_return() {
        let mut timeline = selected(Some("conv-a"));
        let old_history = timeline.history_ticket();
        let old_run = timeline
            .follow_run("agent-a", Some("conv-a"), "run-a")
            .unwrap();
        assert!(
            timeline
                .follow_run("agent-a", Some("conv-b"), "run-b")
                .is_none()
        );
        timeline.select(
            "agent-a".into(),
            Some("conv-b".into()),
            "browser-session".into(),
        );
        timeline.select(
            "agent-a".into(),
            Some("conv-a".into()),
            "browser-session".into(),
        );
        assert!(
            !timeline
                .reconcile_history(old_history, vec![row("old", "assistant", "old", "conv-a")])
        );
        assert!(!timeline.apply(old_run, event(json!({"message_type":"assistant_message","run_id":"run-a","seq_id":1,"content":"old"}))));
        assert!(timeline.messages().is_empty());
        let history = timeline.history_ticket();
        timeline.select(
            "agent-a".into(),
            Some("conv-a".into()),
            "different-auth-session".into(),
        );
        assert!(!timeline.reconcile_history(history, vec![row("old", "user", "old", "conv-a")]));
    }

    #[test]
    fn coordinator_unscoped_selection_cannot_accumulate_another_conversation() {
        let mut timeline = selected(None);
        timeline.persisted(row("foreign", "assistant", "unrelated", "conv-other"));
        assert!(timeline.messages().is_empty());
        let lease = timeline.begin_turn("same prompt").unwrap();
        timeline.persisted(row("foreign-user", "user", "same prompt", "conv-other"));
        timeline.persisted(row("own-user", "user", "same prompt", "conv-own"));
        timeline.apply(lease, event(json!({"message_type":"stream_start","conversation_id":"conv-own","run_id":"run-own"})));
        assert_eq!(timeline.messages().len(), 1);
        assert_eq!(timeline.messages()[0].id, "own-user");
        assert_eq!(
            timeline.messages()[0].conversation_id.as_deref(),
            Some("conv-own")
        );
    }

    #[test]
    fn coordinator_history_during_stream_preserves_prefix_and_live_projection() {
        let mut timeline = selected(Some("conv-a"));
        let ticket = timeline.history_ticket();
        let lease = timeline.begin_turn("new prompt").unwrap();
        timeline.apply(
            lease,
            event(
                json!({"message_type":"stream_start","run_id":"run-a","conversation_id":"conv-a"}),
            ),
        );
        timeline.apply(lease, event(json!({"message_type":"assistant_message","run_id":"run-a","seq_id":1,"content":"Live answer"})));
        timeline.reconcile_history(
            ticket,
            vec![
                row("old-user", "user", "earlier", "conv-a"),
                row("old-answer", "assistant", "earlier answer", "conv-a"),
                row("new-user", "user", "new prompt", "conv-a"),
                row("new-answer", "assistant", "Live answer", "conv-a"),
            ],
        );
        assert_eq!(
            timeline
                .messages()
                .iter()
                .map(|m| m.text())
                .collect::<Vec<_>>(),
            vec!["earlier", "earlier answer", "new prompt", "Live answer"]
        );
        assert_eq!(timeline.messages()[2].id, "new-user");
        assert!(timeline.is_live_message(&timeline.messages()[3].id));
    }

    #[test]
    fn coordinator_external_replay_and_history_share_one_current_turn() {
        let mut timeline = selected(Some("conv-a"));
        let ticket = timeline.history_ticket();
        let lease = timeline
            .follow_run("agent-a", Some("conv-a"), "run-a")
            .unwrap();
        timeline.apply(lease, event(json!({"message_type":"assistant_message","run_id":"run-a","seq_id":1,"content":"Partial"})));
        timeline.reconcile_history(
            ticket,
            vec![
                row("old-user", "user", "previous", "conv-a"),
                row("old-answer", "assistant", "previous answer", "conv-a"),
                row("current-user", "user", "current", "conv-a"),
                row("partial-db", "assistant", "Partial", "conv-a"),
            ],
        );
        assert_eq!(
            timeline
                .messages()
                .iter()
                .filter(|m| m.role == "assistant")
                .count(),
            2
        );
        assert_eq!(timeline.messages().last().unwrap().text(), "Partial");
    }

    #[test]
    fn coordinator_latest_history_request_wins_and_same_id_live_update_is_not_reverted() {
        let mut timeline = selected(Some("conv-a"));
        let older = timeline.history_ticket();
        let latest = timeline.history_ticket();
        assert!(!timeline.reconcile_history(older, vec![row("m", "assistant", "old", "conv-a")]));
        timeline.persisted(row("m", "assistant", "updated", "conv-a"));
        assert!(timeline.reconcile_history(latest, vec![row("m", "assistant", "old", "conv-a")]));
        assert_eq!(timeline.messages()[0].text(), "updated");
    }

    #[test]
    fn coordinator_compaction_during_run_removes_archived_prefix_without_erasing_live_output() {
        let mut timeline = selected(Some("conv-a"));
        let ticket = timeline.history_ticket();
        timeline.reconcile_history(
            ticket,
            vec![
                row("archived-user", "user", "old", "conv-a"),
                row("archived-answer", "assistant", "old answer", "conv-a"),
            ],
        );
        let lease = timeline.begin_turn("current").unwrap();
        timeline.apply(lease, event(json!({"message_type":"assistant_message","run_id":"run-a","seq_id":1,"content":"Still streaming"})));
        let compacted = timeline.history_ticket();
        timeline.reconcile_history(
            compacted,
            vec![
                row("summary", "system", "Previous turns summarized", "conv-a"),
                row("current-user", "user", "current", "conv-a"),
            ],
        );
        assert_eq!(timeline.messages()[0].id, "summary");
        assert_eq!(timeline.messages()[1].id, "current-user");
        assert_eq!(timeline.messages().len(), 3);
        assert_eq!(
            timeline.messages().last().unwrap().text(),
            "Still streaming"
        );
    }

    #[test]
    fn coordinator_nested_tools_and_transport_failure_preserve_real_output() {
        let mut timeline = selected(Some("conv-a"));
        let lease = timeline.begin_turn("inspect").unwrap();
        timeline.apply(lease, event(json!({"message_type":"tool_call_message","run_id":"run-a","seq_id":1,"tool_call":{"id":"tc-a","name":"read_file","arguments":{"path":"notes.txt"}}})));
        timeline.apply(lease, event(json!({"message_type":"tool_result_message","run_id":"run-a","seq_id":2,"tool_result":{"tool_call_id":"tc-a","tool_name":"read_file","output":"actual output","is_error":false}})));
        timeline.transport_failed(lease, "connection interrupted");
        let text = timeline.messages().last().unwrap().text();
        assert!(
            text.contains("notes.txt")
                && text.contains("actual output")
                && text.contains("connection interrupted")
        );
        assert!(
            timeline
                .outcome(lease)
                .unwrap()
                .starts_with("transport_error:")
        );
    }

    #[test]
    fn coordinator_resolved_questions_cannot_be_resurrected_by_delayed_run_or_queue() {
        let mut timeline = selected(None);
        let question = event(
            json!({"type":"question_required","id":"q-a","agent_id":"agent-a","questions":[{"header":"Scope","question":"Which scope?","options":[{"label":"Local"}]}]}),
        );
        let mut pending = vec![];
        timeline.pending_event(&mut pending, &question);
        timeline.pending_event(&mut pending, &event(json!({"event_type":"approval_resolved","id":"q-a","status":"approved:{\"Scope\":\"Local\"}"})));
        timeline.pending_event(&mut pending, &question);
        assert!(pending.is_empty());
        assert!(
            timeline
                .pending_snapshot(vec![question.data.clone()], &[])
                .is_empty()
        );
        timeline.select("agent-b".into(), None, "browser-session".into());
        assert!(
            timeline
                .pending_snapshot(vec![question.data.clone()], &[])
                .is_empty()
        );
        timeline.select("agent-b".into(), None, "new-session".into());
        assert_eq!(timeline.pending_snapshot(vec![question.data], &[]).len(), 1);
    }

    #[test]
    fn live_cache_validates_source_and_prunes_navigation_compaction_and_live_entries() {
        let mut cache = ParsedMessageCache::default();
        assert_eq!(cache.parse("m", "old", false).0, "old");
        assert_eq!(cache.parse("m", "new", false).0, "new");
        assert_eq!(
            cache.parse("m", "<reasoning>Think</reasoning>Answer", true),
            ("Answer".into(), Some("Think".into()))
        );
        assert!(
            cache.entries.is_empty(),
            "live sources are never cached by ID"
        );
        cache.parse("m", "settled", false);
        cache.retain_messages(&[row("m", "assistant", "changed", "conv-a")]);
        assert!(cache.entries.is_empty());
        cache.parse("m", "settled", false);
        cache.retain_messages(&[]);
        assert!(cache.entries.is_empty());
    }
}
