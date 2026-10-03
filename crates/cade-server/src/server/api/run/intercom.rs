use parking_lot::Mutex;
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, VecDeque};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, OnceLock};

/// State of an intercom message in the mailbox.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum MessageStatus {
    Unread,
    Answered,
}

/// A routed message between agents or between a subagent and its supervisor.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IntercomMessage {
    pub id: String,
    pub from: String,
    pub to: String,
    pub body: String,
    pub kind: String, // "send", "ask", or "reply"
    pub reply_to: Option<String>,
    pub timestamp_ms: u64,
    pub status: MessageStatus,
}

/// Deep module managing bidirectional inter-agent communication, mailboxes,
/// and failure recovery reports.
///
/// Hides internal queues, message correlation, locks, and formatting behind
/// a concise operational interface.
#[derive(Default)]
pub struct IntercomHub {
    counter: AtomicU64,
    /// Mailboxes keyed by recipient ID (e.g. "supervisor", "agent-123").
    mailboxes: Mutex<HashMap<String, VecDeque<IntercomMessage>>>,
    /// All messages ever recorded in this session, keyed by message ID.
    history: Mutex<HashMap<String, IntercomMessage>>,
}

impl IntercomHub {
    /// Return the global singleton hub shared across subagent execution threads.
    pub fn global() -> &'static Arc<Self> {
        static HUB: OnceLock<Arc<IntercomHub>> = OnceLock::new();
        HUB.get_or_init(|| Arc::new(Self::default()))
    }

    /// Primary interface method: process an incoming intercom tool invocation.
    pub fn dispatch(
        &self,
        action: &str,
        caller: &str,
        to: &str,
        message: &str,
        reply_to: &str,
    ) -> String {
        let normalized_action = action.trim().to_lowercase();
        let target = if to.trim().is_empty() {
            "supervisor"
        } else {
            to.trim()
        };

        match normalized_action.as_str() {
            "send" | "ask" => {
                let is_ask = normalized_action == "ask";
                let msg = self.enqueue(caller, target, message, is_ask, None);
                let verb = if is_ask { "Question asked" } else { "Message sent" };
                format!("{verb} to '{target}' [id: {}]: \"{}\"", msg.id, msg.body)
            }
            "reply" => {
                if reply_to.trim().is_empty() {
                    return "Error: 'replyTo' parameter is required when action is 'reply'".to_string();
                }
                match self.reply(caller, reply_to.trim(), message) {
                    Ok(msg) => format!(
                        "Replied to message '{}' -> '{}' [id: {}]: \"{}\"",
                        reply_to.trim(),
                        msg.to,
                        msg.id,
                        msg.body
                    ),
                    Err(err) => format!("Error replying to '{}': {err}", reply_to.trim()),
                }
            }
            "pending" => {
                // If caller is supervisor (or asks without specific to), return all messages addressed to caller or supervisor.
                let pending = self.fetch_pending(if caller.trim().is_empty() {
                    "supervisor"
                } else {
                    caller.trim()
                });

                if pending.is_empty() {
                    return "[] (No pending messages)".to_string();
                }

                serde_json::to_string_pretty(&pending).unwrap_or_else(|_| "[]".to_string())
            }
            "list" => {
                let channels = self.active_channels();
                if channels.is_empty() {
                    return "[] (No active intercom channels)".to_string();
                }
                serde_json::to_string(&channels).unwrap_or_else(|_| "[]".to_string())
            }
            "status" => {
                let (routes, unread) = self.stats();
                format!(
                    "Intercom channel: connected. Active participants: {routes}. Unread messages: {unread}."
                )
            }
            other => format!("Unsupported action '{other}'. Supported: send, ask, reply, pending, list, status."),
        }
    }

    /// Enqueue a message into the recipient's mailbox.
    pub fn enqueue(
        &self,
        from: &str,
        to: &str,
        body: &str,
        is_ask: bool,
        reply_to: Option<String>,
    ) -> IntercomMessage {
        let seq = self.counter.fetch_add(1, Ordering::Relaxed) + 1;
        let id = format!("msg-{seq}");
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_millis() as u64)
            .unwrap_or(0);

        let kind = if reply_to.is_some() {
            "reply"
        } else if is_ask {
            "ask"
        } else {
            "send"
        };

        let message = IntercomMessage {
            id: id.clone(),
            from: from.trim().to_string(),
            to: to.trim().to_string(),
            body: body.trim().to_string(),
            kind: kind.to_string(),
            reply_to,
            timestamp_ms: now,
            status: MessageStatus::Unread,
        };

        let mut mailboxes = self.mailboxes.lock();
        mailboxes
            .entry(message.to.clone())
            .or_default()
            .push_back(message.clone());

        let mut history = self.history.lock();
        history.insert(id, message.clone());

        message
    }

    /// Reply to an existing message by ID.
    pub fn reply(
        &self,
        from: &str,
        reply_to_id: &str,
        body: &str,
    ) -> Result<IntercomMessage, String> {
        let recipient = {
            let mut history = self.history.lock();
            let original = history
                .get_mut(reply_to_id)
                .ok_or_else(|| format!("Message '{reply_to_id}' not found in intercom history"))?;

            original.status = MessageStatus::Answered;
            original.from.clone()
        };

        // Enqueue response to the original sender
        Ok(self.enqueue(from, &recipient, body, false, Some(reply_to_id.to_string())))
    }

    /// Fetch all pending (unread) messages for a given recipient.
    pub fn fetch_pending(&self, recipient: &str) -> Vec<IntercomMessage> {
        let mut mailboxes = self.mailboxes.lock();
        let mut results = Vec::new();

        let recipient_key = recipient.trim();
        if let Some(queue) = mailboxes.get_mut(recipient_key) {
            while let Some(mut msg) = queue.pop_front() {
                msg.status = MessageStatus::Answered;
                results.push(msg);
            }
        }

        // If recipient is "supervisor", also check if there are unrouted questions
        if recipient_key == "supervisor"
            && let Some(queue) = mailboxes.get_mut("")
        {
            while let Some(mut msg) = queue.pop_front() {
                msg.status = MessageStatus::Answered;
                results.push(msg);
            }
        }

        results
    }

    /// Return all active participants/channels with traffic.
    pub fn active_channels(&self) -> Vec<String> {
        let history = self.history.lock();
        let mut channels = std::collections::BTreeSet::new();
        for msg in history.values() {
            if !msg.from.is_empty() {
                channels.insert(msg.from.clone());
            }
            if !msg.to.is_empty() {
                channels.insert(msg.to.clone());
            }
        }
        channels.into_iter().collect()
    }

    /// Get current metrics: (unique participants count, unread count).
    pub fn stats(&self) -> (usize, usize) {
        let mailboxes = self.mailboxes.lock();
        let unread: usize = mailboxes.values().map(|q| q.len()).sum();
        let participants = mailboxes.len();
        (participants, unread)
    }

    /// Clear all mailboxes and history (useful for tests or clean session restart).
    #[allow(dead_code)]
    pub fn reset(&self) {
        self.mailboxes.lock().clear();
        self.history.lock().clear();
        self.counter.store(0, Ordering::Relaxed);
    }

    /// Generate a structured timeout recovery report when a subagent times out.
    pub fn generate_timeout_report(
        subagent_id: &str,
        wall_clock_secs: u64,
        partial_events_count: usize,
        error_detail: Option<&str>,
    ) -> String {
        let error_msg = error_detail.unwrap_or("Subagent exceeded maximum execution timeout");
        format!(
            "⚠️ SubagentTimeoutReport:\n\
             - Subagent ID: {subagent_id}\n\
             - Duration: {wall_clock_secs}s before interruption\n\
             - Partial Events Captured: {partial_events_count}\n\
             - Status: Gracefully halted to preserve host stability\n\
             - Diagnostic: {error_msg}\n\
             - Action: Inspect captured events or retry with a higher token/time budget."
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_intercom_dispatch_send_and_pending() {
        let hub = IntercomHub::default();

        let res = hub.dispatch("send", "child-agent", "supervisor", "Hello supervisor", "");
        assert!(res.contains("Message sent to 'supervisor'"));

        let pending_json = hub.dispatch("pending", "supervisor", "", "", "");
        assert!(pending_json.contains("Hello supervisor"));
        assert!(pending_json.contains("child-agent"));

        // Second pending call should now be empty because messages were retrieved
        let second_pending = hub.dispatch("pending", "supervisor", "", "", "");
        assert_eq!(second_pending, "[] (No pending messages)");
    }

    #[test]
    fn test_intercom_ask_and_reply_routing() {
        let hub = IntercomHub::default();

        hub.dispatch("ask", "worker-1", "supervisor", "Need database password?", "");

        let pending = hub.fetch_pending("supervisor");
        assert_eq!(pending.len(), 1);
        let msg_id = pending[0].id.clone();
        assert_eq!(pending[0].kind, "ask");

        // Supervisor replies
        let reply_res = hub.dispatch("reply", "supervisor", "", "Use environment var DB_PASS", &msg_id);
        assert!(reply_res.contains("Replied to message"));
        assert!(reply_res.contains("worker-1"));

        // Worker checks pending and receives the supervisor's answer
        let worker_pending = hub.fetch_pending("worker-1");
        assert_eq!(worker_pending.len(), 1);
        assert_eq!(worker_pending[0].body, "Use environment var DB_PASS");
        assert_eq!(worker_pending[0].reply_to.as_deref(), Some(msg_id.as_str()));
    }

    #[test]
    fn test_active_channels_and_status() {
        let hub = IntercomHub::default();
        hub.dispatch("send", "agent-a", "agent-b", "ping", "");

        let channels = hub.active_channels();
        assert!(channels.contains(&"agent-a".to_string()));
        assert!(channels.contains(&"agent-b".to_string()));

        let status = hub.dispatch("status", "", "", "", "");
        assert!(status.contains("Active participants"));
    }

    #[test]
    fn test_timeout_report_generation() {
        let report = IntercomHub::generate_timeout_report("agent-searcher", 30, 12, None);
        assert!(report.contains("agent-searcher"));
        assert!(report.contains("30s"));
        assert!(report.contains("12"));
        assert!(report.contains("SubagentTimeoutReport"));
    }
}
