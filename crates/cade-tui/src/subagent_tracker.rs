use std::time::Instant;

#[derive(Debug, Clone)]
pub struct SubagentTracker {
    pub task_id: String,
    pub mode: String,
    pub started: Instant,
    pub tool_calls: u32,
    pub output_lines: u32,
    /// Name of the tool currently being executed (None when idle/between calls).
    pub current_tool: Option<String>,
}

impl SubagentTracker {
    pub fn new(task_id: String, mode: String) -> Self {
        Self {
            task_id,
            mode,
            started: Instant::now(),
            tool_calls: 0,
            output_lines: 0,
            current_tool: None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn new_tracker_has_no_current_tool() {
        let t = SubagentTracker::new("t1".into(), "worker".into());
        assert!(t.current_tool.is_none());
        assert_eq!(t.tool_calls, 0);
        assert_eq!(t.output_lines, 0);
    }

    #[test]
    fn current_tool_tracks_active_call() {
        let mut t = SubagentTracker::new("t2".into(), "build".into());
        t.current_tool = Some("bash".into());
        t.tool_calls += 1;
        assert_eq!(t.current_tool.as_deref(), Some("bash"));
        assert_eq!(t.tool_calls, 1);

        // Cleared after tool finishes
        t.current_tool = None;
        assert!(t.current_tool.is_none());
    }
}
