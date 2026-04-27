//! Live tool-output blocks for [`super::SessionState`].

use super::*;

impl SessionState {
    /// Begin a new live-output block for a tool call.
    pub fn begin_live_output(&mut self, call_id: &str, tool_name: &str) {
        if let Self::Connected { live_outputs, .. } = self {
            live_outputs.push(LiveOutputBlock {
                call_id: call_id.to_string(),
                tool_name: tool_name.to_string(),
                lines: Vec::new(),
                done: false,
                max_visible: 8,
            });
        }
    }

    /// Append a line to an existing live-output block.
    pub fn append_live_output(&mut self, call_id: &str, line: String) {
        if let Self::Connected { live_outputs, .. } = self {
            if let Some(block) = live_outputs.iter_mut().find(|b| b.call_id == call_id) {
                block.lines.push(line);
            }
        }
    }

    /// Mark a live-output block as finished.
    pub fn finish_live_output(&mut self, call_id: &str) {
        if let Self::Connected { live_outputs, .. } = self {
            if let Some(block) = live_outputs.iter_mut().find(|b| b.call_id == call_id) {
                block.done = true;
            }
        }
    }

    /// Read-only access to live output blocks.
    pub fn live_outputs(&self) -> &[LiveOutputBlock] {
        if let Self::Connected { live_outputs, .. } = self {
            live_outputs
        } else {
            &[]
        }
    }
}
