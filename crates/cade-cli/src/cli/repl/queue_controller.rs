use parking_lot::Mutex;
use std::collections::VecDeque;
use std::fmt;
use std::sync::Arc;

/// Represents an item waiting in the follow-up queue.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct QueuedItem {
    /// 1-based display index for CLI interactions.
    pub index: usize,
    /// Prompt content.
    pub text: String,
}

/// Instantaneous snapshot of all interactive queues.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct QueueSnapshot {
    /// Active steering message (if one is pending to interrupt/redirect the current turn).
    pub steering: Option<String>,
    /// Pending follow-up messages waiting to be executed in sequence.
    pub followups: Vec<QueuedItem>,
}

impl QueueSnapshot {
    /// Returns true if neither steering nor follow-ups are queued.
    pub fn is_empty(&self) -> bool {
        self.steering.is_none() && self.followups.is_empty()
    }

    /// Total count of queued items (steering + follow-ups).
    pub fn total_count(&self) -> usize {
        (if self.steering.is_some() { 1 } else { 0 }) + self.followups.len()
    }
}

/// Errors arising from interactive queue manipulations.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum QueueError {
    /// Requested 1-based index was out of range.
    IndexOutOfBounds { index: usize, total: usize },
    /// Follow-up queue was empty when an operation was requested.
    QueueEmpty,
    /// Steering message was not present.
    NoSteeringPending,
}

impl fmt::Display for QueueError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::IndexOutOfBounds { index, total } => {
                if *total == 0 {
                    write!(f, "Queue is empty; cannot drop item #{index}")
                } else {
                    write!(
                        f,
                        "Index #{index} is out of bounds (current queue has {total} item{})",
                        if *total == 1 { "" } else { "s" }
                    )
                }
            }
            Self::QueueEmpty => write!(f, "Follow-up queue is empty"),
            Self::NoSteeringPending => write!(f, "No steering message is currently queued"),
        }
    }
}

impl std::error::Error for QueueError {}

/// Deep module encapsulating interactive steering and follow-up queues.
///
/// Hides locking, 1-based indexing calculations, bounds checking, and presentation
/// behind a small, robust interface.
#[derive(Clone)]
pub struct ReplQueueController {
    steering: Arc<Mutex<Option<String>>>,
    followup: Arc<Mutex<VecDeque<String>>>,
}

impl ReplQueueController {
    /// Create a controller over existing shared queue locks.
    pub fn new(
        steering: Arc<Mutex<Option<String>>>,
        followup: Arc<Mutex<VecDeque<String>>>,
    ) -> Self {
        Self { steering, followup }
    }

    /// Create an isolated in-memory controller for testing or standalone usage.
    pub fn isolated() -> Self {
        Self {
            steering: Arc::new(Mutex::new(None)),
            followup: Arc::new(Mutex::new(VecDeque::new())),
        }
    }

    /// Enqueue a follow-up prompt.
    pub fn push_followup(&self, text: impl Into<String>) {
        self.followup.lock().push_back(text.into());
    }

    /// Set an immediate steering message.
    pub fn set_steering(&self, text: impl Into<String>) {
        *self.steering.lock() = Some(text.into());
    }

    /// Capture an immutable point-in-time snapshot of all queues.
    pub fn snapshot(&self) -> QueueSnapshot {
        let steering = self.steering.lock().clone();
        let followup_guard = self.followup.lock();
        let followups = followup_guard
            .iter()
            .enumerate()
            .map(|(i, text)| QueuedItem {
                index: i + 1,
                text: text.clone(),
            })
            .collect();

        QueueSnapshot {
            steering,
            followups,
        }
    }

    /// Drop a follow-up item by 1-based index. Returns the dropped message text.
    pub fn drop_followup(&self, index_1_based: usize) -> Result<String, QueueError> {
        let mut guard = self.followup.lock();
        let total = guard.len();

        if total == 0 {
            return Err(QueueError::QueueEmpty);
        }

        if index_1_based == 0 || index_1_based > total {
            return Err(QueueError::IndexOutOfBounds {
                index: index_1_based,
                total,
            });
        }

        let zero_based_index = index_1_based - 1;
        let removed = guard.remove(zero_based_index).expect("bounds checked");
        Ok(removed)
    }

    /// Drop the pending steering message. Returns the removed steering text.
    pub fn drop_steering(&self) -> Result<String, QueueError> {
        let mut guard = self.steering.lock();
        guard.take().ok_or(QueueError::NoSteeringPending)
    }

    /// Pop the most recently added follow-up item (LIFO).
    pub fn pop_followup(&self) -> Option<String> {
        self.followup.lock().pop_back()
    }

    /// Clear all queues (both steering and follow-ups).
    /// Returns `(dropped_steering, dropped_followup_count)`.
    pub fn clear(&self) -> (Option<String>, usize) {
        let steering = self.steering.lock().take();
        let mut followup_guard = self.followup.lock();
        let count = followup_guard.len();
        followup_guard.clear();
        (steering, count)
    }

    /// Format a snapshot into a clean terminal report.
    pub fn format_snapshot(&self, snapshot: &QueueSnapshot) -> String {
        if snapshot.is_empty() {
            return "No items currently queued. (Type while the agent is running to queue follow-ups or steering)."
                .to_string();
        }

        let mut lines = Vec::new();

        if let Some(ref steer) = snapshot.steering {
            lines.push("⚡ Pending Steering (will interrupt current turn):".to_string());
            lines.push(format!("   [Steering] {}", steer.trim()));
            lines.push(String::new());
        }

        if !snapshot.followups.is_empty() {
            lines.push(format!(
                "📋 Follow-up Queue ({} item{} in execution order):",
                snapshot.followups.len(),
                if snapshot.followups.len() == 1 { "" } else { "s" }
            ));
            for item in &snapshot.followups {
                let preview = item.text.replace('\n', " ");
                let truncated = if preview.len() > 100 {
                    format!("{}...", &preview[..97])
                } else {
                    preview
                };
                lines.push(format!("   {}. {}", item.index, truncated));
            }
        }

        lines.join("\n")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_empty_queue_snapshot() {
        let controller = ReplQueueController::isolated();
        let snap = controller.snapshot();
        assert!(snap.is_empty());
        assert_eq!(snap.total_count(), 0);
        assert!(controller.format_snapshot(&snap).contains("No items"));
    }

    #[test]
    fn test_push_and_snapshot_followups() {
        let controller = ReplQueueController::isolated();
        controller.push_followup("first task");
        controller.push_followup("second task");

        let snap = controller.snapshot();
        assert!(!snap.is_empty());
        assert_eq!(snap.total_count(), 2);
        assert_eq!(snap.followups.len(), 2);
        assert_eq!(snap.followups[0].index, 1);
        assert_eq!(snap.followups[0].text, "first task");
        assert_eq!(snap.followups[1].index, 2);
        assert_eq!(snap.followups[1].text, "second task");
    }

    #[test]
    fn test_drop_followup_bounds_and_removal() {
        let controller = ReplQueueController::isolated();
        controller.push_followup("one");
        controller.push_followup("two");
        controller.push_followup("three");

        // 0 is out of bounds
        assert_eq!(
            controller.drop_followup(0),
            Err(QueueError::IndexOutOfBounds { index: 0, total: 3 })
        );
        // 4 is out of bounds
        assert_eq!(
            controller.drop_followup(4),
            Err(QueueError::IndexOutOfBounds { index: 4, total: 3 })
        );

        // Drop item #2 ("two")
        let removed = controller.drop_followup(2).expect("valid index");
        assert_eq!(removed, "two");

        // Snapshot now has 2 items ("one" and "three"), re-indexed to 1 and 2
        let snap = controller.snapshot();
        assert_eq!(snap.followups.len(), 2);
        assert_eq!(snap.followups[0].index, 1);
        assert_eq!(snap.followups[0].text, "one");
        assert_eq!(snap.followups[1].index, 2);
        assert_eq!(snap.followups[1].text, "three");
    }

    #[test]
    fn test_pop_followup() {
        let controller = ReplQueueController::isolated();
        assert_eq!(controller.pop_followup(), None);

        controller.push_followup("first");
        controller.push_followup("second");

        assert_eq!(controller.pop_followup(), Some("second".to_string()));
        assert_eq!(controller.pop_followup(), Some("first".to_string()));
        assert_eq!(controller.pop_followup(), None);
    }

    #[test]
    fn test_steering_and_clear() {
        let controller = ReplQueueController::isolated();
        controller.set_steering("halt current search");
        controller.push_followup("re-run with tests");

        let snap = controller.snapshot();
        assert_eq!(snap.steering.as_deref(), Some("halt current search"));
        assert_eq!(snap.total_count(), 2);

        let formatted = controller.format_snapshot(&snap);
        assert!(formatted.contains("Pending Steering"));
        assert!(formatted.contains("Follow-up Queue"));

        let (steer, followup_count) = controller.clear();
        assert_eq!(steer, Some("halt current search".to_string()));
        assert_eq!(followup_count, 1);

        assert!(controller.snapshot().is_empty());
    }

    #[test]
    fn test_drop_steering() {
        let controller = ReplQueueController::isolated();
        assert_eq!(controller.drop_steering(), Err(QueueError::NoSteeringPending));

        controller.set_steering("redirect");
        assert_eq!(controller.drop_steering(), Ok("redirect".to_string()));
        assert_eq!(controller.drop_steering(), Err(QueueError::NoSteeringPending));
    }
}
