use std::time::Instant;

/// Upper bound on buffered transcript lines kept per subagent so a
/// long-running background agent can't grow memory without limit.
pub const MAX_TRANSCRIPT_LINES: usize = 2000;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SubagentStatus {
    Running,
    Completed { finished_at: Instant },
    Failed { finished_at: Instant, error: String },
}

#[derive(Debug, Clone)]
pub struct SubagentTracker {
    pub task_id: String,
    pub mode: String,
    pub started: Instant,
    pub tool_calls: u32,
    pub output_lines: u32,
    /// Name of the tool currently being executed (None when idle/between calls).
    pub current_tool: Option<String>,
    /// Last output emitted by this subagent (most recent first), for the
    /// navigable child-session inspector overlay.
    pub transcript: Vec<String>,
    pub status: SubagentStatus,
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
            transcript: Vec::new(),
            status: SubagentStatus::Running,
        }
    }

    /// Buffer one output line, dropping the oldest once the cap is hit.
    pub fn push_output(&mut self, line: String) {
        self.output_lines += 1;
        self.transcript.insert(0, line);
        if self.transcript.len() > MAX_TRANSCRIPT_LINES {
            self.transcript.truncate(MAX_TRANSCRIPT_LINES);
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
        assert!(t.transcript.is_empty());
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

    #[test]
    fn push_output_buffers_newest_first() {
        let mut t = SubagentTracker::new("t3".into(), "worker".into());
        t.push_output("first".into());
        t.push_output("second".into());
        assert_eq!(t.output_lines, 2);
        // Newest line is at the front for cheap cap truncation.
        assert_eq!(t.transcript.first().map(|s| s.as_str()), Some("second"));
        assert_eq!(t.transcript.last().map(|s| s.as_str()), Some("first"));
    }

    #[test]
    fn push_output_caps_transcript() {
        let mut t = SubagentTracker::new("t4".into(), "worker".into());
        for i in 0..(MAX_TRANSCRIPT_LINES + 50) {
            t.push_output(format!("line {i}"));
        }
        assert_eq!(t.transcript.len(), MAX_TRANSCRIPT_LINES);
        assert_eq!(t.output_lines, (MAX_TRANSCRIPT_LINES + 50) as u32);
    }
}
