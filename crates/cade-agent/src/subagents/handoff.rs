use serde::{Deserialize, Serialize};
use std::collections::{HashMap, VecDeque};
use std::fmt;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, OnceLock};

/// Canonical closure reasons enforcing OpenRig-style hot-potato task handoff contracts.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ClosureReason {
    /// Work was handed off to a downstream specialist agent or seat.
    HandedOffTo,
    /// Work is blocked on an external resource, dependency, or approval.
    BlockedOn,
    /// Work was denied by policy, security review, or guardian.
    Denied,
    /// Work was explicitly canceled by human user or supervisor.
    Canceled,
    /// Task has concluded cleanly with no follow-on steps required.
    NoFollowOn,
    /// Work required higher authorization and was escalated to supervisor or human.
    Escalation,
    /// Task was superseded by a newer, higher-priority requirement or rewrite.
    Superseded,
}

impl ClosureReason {
    /// Returns true if this closure reason represents an active baton pass
    /// that strictly requires a `closure_target`.
    pub fn requires_target(&self) -> bool {
        matches!(
            self,
            Self::HandedOffTo | Self::BlockedOn | Self::Escalation | Self::Superseded
        )
    }

    /// Parse from loose string input (case-insensitive, kebab or snake case).
    pub fn from_str_loose(s: &str) -> Option<Self> {
        let normalized = s.trim().to_lowercase().replace('-', "_");
        match normalized.as_str() {
            "handed_off_to" | "handed_off" | "handoff" => Some(Self::HandedOffTo),
            "blocked_on" | "blocked" => Some(Self::BlockedOn),
            "denied" | "rejected" => Some(Self::Denied),
            "canceled" | "cancelled" | "abort" => Some(Self::Canceled),
            "no_follow_on" | "no_followon" | "completed" | "done" => Some(Self::NoFollowOn),
            "escalation" | "escalated" | "escalate" => Some(Self::Escalation),
            "superseded" | "replaced" => Some(Self::Superseded),
            _ => None,
        }
    }
}

impl fmt::Display for ClosureReason {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::HandedOffTo => write!(f, "handed_off_to"),
            Self::BlockedOn => write!(f, "blocked_on"),
            Self::Denied => write!(f, "denied"),
            Self::Canceled => write!(f, "canceled"),
            Self::NoFollowOn => write!(f, "no_follow_on"),
            Self::Escalation => write!(f, "escalation"),
            Self::Superseded => write!(f, "superseded"),
        }
    }
}

/// Request payload attempting to close or hand off a task.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ClosureRequest {
    /// Target state: "done", "blocked", "failed", "canceled".
    pub state: String,
    /// Justification for closing the task.
    pub closure_reason: Option<ClosureReason>,
    /// Target seat, agent, or dependency when handing off or escalating.
    pub closure_target: Option<String>,
    /// Summary of what was achieved.
    pub summary: String,
    /// Optional context payload passed to the successor task.
    pub payload: Option<serde_json::Value>,
}

impl ClosureRequest {
    pub fn done_no_follow_on(summary: impl Into<String>) -> Self {
        Self {
            state: "done".to_string(),
            closure_reason: Some(ClosureReason::NoFollowOn),
            closure_target: None,
            summary: summary.into(),
            payload: None,
        }
    }

    pub fn handoff_to(
        target: impl Into<String>,
        summary: impl Into<String>,
        payload: Option<serde_json::Value>,
    ) -> Self {
        Self {
            state: "done".to_string(),
            closure_reason: Some(ClosureReason::HandedOffTo),
            closure_target: Some(target.into()),
            summary: summary.into(),
            payload,
        }
    }

    pub fn escalate_to(
        target: impl Into<String>,
        reason: impl Into<String>,
        payload: Option<serde_json::Value>,
    ) -> Self {
        Self {
            state: "done".to_string(),
            closure_reason: Some(ClosureReason::Escalation),
            closure_target: Some(target.into()),
            summary: reason.into(),
            payload,
        }
    }
}

/// Cryptographically traceable receipt returned when a task closure is accepted.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ClosureReceipt {
    pub task_id: String,
    pub closed_at_ms: u64,
    pub closure_reason: ClosureReason,
    pub closure_target: Option<String>,
    /// Unique ID of the successor task created if this closure was a handoff or escalation.
    pub successor_task_id: Option<String>,
}

/// Errors returned when a closure request violates hot-potato contract invariants.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ClosureError {
    TaskNotFound(String),
    TaskAlreadyClosed {
        task_id: String,
        existing_reason: ClosureReason,
    },
    MissingClosureReason,
    InvalidClosureReason(String),
    MissingClosureTarget {
        reason: ClosureReason,
    },
}

impl fmt::Display for ClosureError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::TaskNotFound(id) => write!(f, "Task '{id}' was not found in task coordinator"),
            Self::TaskAlreadyClosed {
                task_id,
                existing_reason,
            } => {
                write!(
                    f,
                    "Task '{task_id}' is already closed with reason: {existing_reason}"
                )
            }
            Self::MissingClosureReason => {
                write!(
                    f,
                    "Hot-potato contract violation: state='done' requires an explicit closure_reason"
                )
            }
            Self::InvalidClosureReason(raw) => {
                write!(f, "Unrecognized closure_reason: '{raw}'")
            }
            Self::MissingClosureTarget { reason } => {
                write!(
                    f,
                    "Hot-potato contract violation: closure_reason='{reason}' strictly requires a closure_target"
                )
            }
        }
    }
}

impl std::error::Error for ClosureError {}

/// State of a managed task in the coordinator ledger.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HandoffTask {
    pub id: String,
    pub assigned_to: String,
    pub title: String,
    pub status: String, // "pending", "in_progress", "done"
    pub created_at_ms: u64,
    pub closed_at_ms: Option<u64>,
    pub closure_reason: Option<ClosureReason>,
    pub closure_target: Option<String>,
    pub predecessor_task_id: Option<String>,
    pub successor_task_id: Option<String>,
    pub payload: Option<serde_json::Value>,
}

/// Deep module orchestrating task lifecycles, handoffs, and hot-potato contracts.
///
/// Hides state transactions, target verification, successor creation, and inboxes
/// behind a small, robust interface.
#[derive(Default)]
pub struct TaskHandoffCoordinator {
    counter: AtomicU64,
    tasks: Mutex<HashMap<String, HandoffTask>>,
    /// Mailbox inboxes keyed by seat or agent ID (`reviewer`, `supervisor`, `seat:architect`).
    inboxes: Mutex<HashMap<String, VecDeque<String>>>,
}

impl TaskHandoffCoordinator {
    /// Global singleton instance for server/agent run coordination.
    pub fn global() -> &'static Arc<Self> {
        static COORDINATOR: OnceLock<Arc<TaskHandoffCoordinator>> = OnceLock::new();
        COORDINATOR.get_or_init(|| Arc::new(Self::default()))
    }

    /// Register a new task assigned to an agent or seat.
    pub fn register_task(
        &self,
        assigned_to: impl Into<String>,
        title: impl Into<String>,
        payload: Option<serde_json::Value>,
    ) -> HandoffTask {
        let seq = self.counter.fetch_add(1, Ordering::Relaxed) + 1;
        let task_id = format!("task-{seq}");
        let now = Self::current_time_ms();

        let assigned_str = assigned_to.into();
        let task = HandoffTask {
            id: task_id.clone(),
            assigned_to: assigned_str.clone(),
            title: title.into(),
            status: "pending".to_string(),
            created_at_ms: now,
            closed_at_ms: None,
            closure_reason: None,
            closure_target: None,
            predecessor_task_id: None,
            successor_task_id: None,
            payload,
        };

        self.tasks.lock().unwrap().insert(task_id.clone(), task.clone());
        self.inboxes
            .lock()
            .unwrap()
            .entry(assigned_str)
            .or_default()
            .push_back(task_id);

        task
    }

    /// Validate a closure request against the hot-potato invariants.
    pub fn validate_closure(
        &self,
        task_id: &str,
        request: &ClosureRequest,
    ) -> Result<ClosureReason, ClosureError> {
        let tasks = self.tasks.lock().unwrap();
        let task = tasks
            .get(task_id)
            .ok_or_else(|| ClosureError::TaskNotFound(task_id.to_string()))?;

        if task.status == "done"
            && let Some(existing) = task.closure_reason
        {
            return Err(ClosureError::TaskAlreadyClosed {
                task_id: task_id.to_string(),
                existing_reason: existing,
            });
        }

        if request.state.eq_ignore_ascii_case("done") {
            let reason = request
                .closure_reason
                .ok_or(ClosureError::MissingClosureReason)?;

            if reason.requires_target() {
                let target = request.closure_target.as_deref().unwrap_or("").trim();
                if target.is_empty() {
                    return Err(ClosureError::MissingClosureTarget { reason });
                }
            }
            Ok(reason)
        } else {
            // For non-done states (e.g. failed/canceled), default to Canceled or Denied if not specified
            Ok(request.closure_reason.unwrap_or(ClosureReason::Canceled))
        }
    }

    /// Execute a verified task closure and atomically create/enqueue successor if handed off.
    pub fn close_task(
        &self,
        task_id: &str,
        request: ClosureRequest,
    ) -> Result<ClosureReceipt, ClosureError> {
        let verified_reason = self.validate_closure(task_id, &request)?;
        let now = Self::current_time_ms();

        let mut successor_id: Option<String> = None;

        // Atomically create successor if handing off or escalating
        if matches!(
            verified_reason,
            ClosureReason::HandedOffTo | ClosureReason::Escalation
        ) {
            let target = request
                .closure_target
                .as_deref()
                .unwrap_or("supervisor")
                .trim();
            let seq = self.counter.fetch_add(1, Ordering::Relaxed) + 1;
            let next_id = format!("task-{seq}");

            let next_task = HandoffTask {
                id: next_id.clone(),
                assigned_to: target.to_string(),
                title: format!("Follow-on from {task_id}: {}", request.summary),
                status: "pending".to_string(),
                created_at_ms: now,
                closed_at_ms: None,
                closure_reason: None,
                closure_target: None,
                predecessor_task_id: Some(task_id.to_string()),
                successor_task_id: None,
                payload: request.payload.clone(),
            };

            self.tasks.lock().unwrap().insert(next_id.clone(), next_task);
            self.inboxes
                .lock()
                .unwrap()
                .entry(target.to_string())
                .or_default()
                .push_back(next_id.clone());

            successor_id = Some(next_id);
        }

        // Close the current task
        let mut tasks = self.tasks.lock().unwrap();
        if let Some(task) = tasks.get_mut(task_id) {
            task.status = "done".to_string();
            task.closed_at_ms = Some(now);
            task.closure_reason = Some(verified_reason);
            task.closure_target = request.closure_target.clone();
            task.successor_task_id = successor_id.clone();
        }

        Ok(ClosureReceipt {
            task_id: task_id.to_string(),
            closed_at_ms: now,
            closure_reason: verified_reason,
            closure_target: request.closure_target,
            successor_task_id: successor_id,
        })
    }

    /// Retrieve all pending tasks for a given agent or seat inbox.
    pub fn pending_inbox(&self, seat_or_agent_id: &str) -> Vec<HandoffTask> {
        let inboxes = self.inboxes.lock().unwrap();
        let tasks = self.tasks.lock().unwrap();

        let Some(queue) = inboxes.get(seat_or_agent_id) else {
            return Vec::new();
        };

        queue
            .iter()
            .filter_map(|id| tasks.get(id))
            .filter(|t| t.status == "pending")
            .cloned()
            .collect()
    }

    /// Look up task status by ID.
    pub fn get_task(&self, task_id: &str) -> Option<HandoffTask> {
        self.tasks.lock().unwrap().get(task_id).cloned()
    }

    /// Reset internal state (for isolated unit tests).
    #[allow(dead_code)]
    pub fn reset(&self) {
        self.tasks.lock().unwrap().clear();
        self.inboxes.lock().unwrap().clear();
        self.counter.store(0, Ordering::Relaxed);
    }

    fn current_time_ms() -> u64 {
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_millis() as u64)
            .unwrap_or(0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn test_closure_reason_loose_parsing() {
        assert_eq!(
            ClosureReason::from_str_loose("handed_off_to"),
            Some(ClosureReason::HandedOffTo)
        );
        assert_eq!(
            ClosureReason::from_str_loose("handed-off"),
            Some(ClosureReason::HandedOffTo)
        );
        assert_eq!(
            ClosureReason::from_str_loose("BLOCKED_ON"),
            Some(ClosureReason::BlockedOn)
        );
        assert_eq!(
            ClosureReason::from_str_loose("no-followon"),
            Some(ClosureReason::NoFollowOn)
        );
        assert_eq!(
            ClosureReason::from_str_loose("done"),
            Some(ClosureReason::NoFollowOn)
        );
        assert_eq!(
            ClosureReason::from_str_loose("escalated"),
            Some(ClosureReason::Escalation)
        );
    }

    #[test]
    fn test_hot_potato_requires_closure_reason() {
        let coordinator = TaskHandoffCoordinator::default();
        let task = coordinator.register_task("worker-1", "Implement auth module", None);

        let req = ClosureRequest {
            state: "done".to_string(),
            closure_reason: None, // Missing!
            closure_target: None,
            summary: "Done without reason".to_string(),
            payload: None,
        };

        let err = coordinator.close_task(&task.id, req).unwrap_err();
        assert_eq!(err, ClosureError::MissingClosureReason);
    }

    #[test]
    fn test_hot_potato_requires_target_when_handing_off() {
        let coordinator = TaskHandoffCoordinator::default();
        let task = coordinator.register_task("worker-1", "Implement parser", None);

        let req = ClosureRequest {
            state: "done".to_string(),
            closure_reason: Some(ClosureReason::HandedOffTo),
            closure_target: None, // Missing target!
            summary: "Handing off".to_string(),
            payload: None,
        };

        let err = coordinator.close_task(&task.id, req).unwrap_err();
        assert_eq!(
            err,
            ClosureError::MissingClosureTarget {
                reason: ClosureReason::HandedOffTo
            }
        );
    }

    #[test]
    fn test_clean_closure_no_follow_on() {
        let coordinator = TaskHandoffCoordinator::default();
        let task = coordinator.register_task("worker-1", "Fix typo in docs", None);

        let req = ClosureRequest::done_no_follow_on("Documentation typo corrected");
        let receipt = coordinator.close_task(&task.id, req).unwrap();

        assert_eq!(receipt.task_id, task.id);
        assert_eq!(receipt.closure_reason, ClosureReason::NoFollowOn);
        assert_eq!(receipt.successor_task_id, None);

        let updated = coordinator.get_task(&task.id).unwrap();
        assert_eq!(updated.status, "done");
        assert_eq!(updated.closure_reason, Some(ClosureReason::NoFollowOn));
    }

    #[test]
    fn test_atomic_handoff_creates_successor_in_target_inbox() {
        let coordinator = TaskHandoffCoordinator::default();
        let task = coordinator.register_task("worker-dev", "Build user profile API", None);

        let payload = json!({ "endpoints": ["/api/v1/profile"] });
        let req = ClosureRequest::handoff_to("seat:reviewer", "Code written, ready for review", Some(payload.clone()));

        let receipt = coordinator.close_task(&task.id, req).unwrap();
        assert_eq!(receipt.closure_reason, ClosureReason::HandedOffTo);
        assert_eq!(receipt.closure_target.as_deref(), Some("seat:reviewer"));

        let succ_id = receipt.successor_task_id.expect("successor created");
        assert_ne!(succ_id, task.id);

        // Successor task must be in reviewer's inbox
        let reviewer_inbox = coordinator.pending_inbox("seat:reviewer");
        assert_eq!(reviewer_inbox.len(), 1);
        assert_eq!(reviewer_inbox[0].id, succ_id);
        assert_eq!(reviewer_inbox[0].predecessor_task_id.as_deref(), Some(task.id.as_str()));
        assert_eq!(reviewer_inbox[0].payload, Some(payload));
    }

    #[test]
    fn test_cannot_close_already_closed_task() {
        let coordinator = TaskHandoffCoordinator::default();
        let task = coordinator.register_task("worker-1", "Task to run once", None);

        coordinator
            .close_task(&task.id, ClosureRequest::done_no_follow_on("Done first time"))
            .unwrap();

        let second_attempt = coordinator.close_task(
            &task.id,
            ClosureRequest::done_no_follow_on("Done second time"),
        );
        assert!(matches!(
            second_attempt,
            Err(ClosureError::TaskAlreadyClosed { .. })
        ));
    }
}
