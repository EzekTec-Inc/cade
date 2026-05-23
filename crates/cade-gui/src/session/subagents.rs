//! Subagent-card tracking state for [`super::SessionState`].

use super::*;

impl SessionState {
    /// A subagent started running.
    pub fn on_subagent_started(&mut self, id: &str, task: &str, mode: &str, model: &str) {
        if let Self::Connected(session) = self {
            let crate::session::ConnectedSession { subagent_cards, .. } = &mut **session;
            subagent_cards.push(SubagentCardState {
                subagent_id: id.to_string(),
                task: task.to_string(),
                mode: mode.to_string(),
                model: model.to_string(),
                status: "running".to_string(),
                elapsed_secs: 0,
                tool_calls: 0,
                output_lines: 0,
                result_preview: String::new(),
                is_error: false,
            });
        }
    }

    /// A running subagent sent a progress update.
    pub fn on_subagent_progress(
        &mut self,
        id: &str,
        _status: &str,
        tool_calls: u32,
        output_lines: u32,
        elapsed: u32,
    ) {
        if let Self::Connected(session) = self {
            let crate::session::ConnectedSession { subagent_cards, .. } = &mut **session;
            if let Some(card) = subagent_cards.iter_mut().find(|c| c.subagent_id == id) {
                card.tool_calls = tool_calls;
                card.output_lines = output_lines;
                card.elapsed_secs = elapsed;
            }
        }
    }

    /// A subagent finished (success or error).
    pub fn on_subagent_complete(
        &mut self,
        id: &str,
        _status: &str,
        result_preview: &str,
        elapsed: u32,
        is_error: bool,
    ) {
        if let Self::Connected(session) = self {
            let crate::session::ConnectedSession { subagent_cards, .. } = &mut **session;
            if let Some(card) = subagent_cards.iter_mut().find(|c| c.subagent_id == id) {
                card.status = if is_error { "error" } else { "complete" }.to_string();
                card.elapsed_secs = elapsed;
                card.result_preview = result_preview.to_string();
                card.is_error = is_error;
            }
        }
    }
}
