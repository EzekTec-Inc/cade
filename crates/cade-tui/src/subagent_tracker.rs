use std::time::Instant;

#[derive(Debug, Clone)]
pub struct SubagentTracker {
    pub task_id: String,
    pub mode: String,
    pub started: Instant,
    pub tool_calls: u32,
    pub output_lines: u32,
}

impl SubagentTracker {
    pub fn new(task_id: String, mode: String) -> Self {
        Self {
            task_id,
            mode,
            started: Instant::now(),
            tool_calls: 0,
            output_lines: 0,
        }
    }
}
