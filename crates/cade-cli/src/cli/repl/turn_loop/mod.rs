use super::{Repl, ToolPreflightResult};
use super::{fmt_tok_short, fmt_window_tokens_short, short_mode_label};

#[derive(Default, Debug)]
pub(crate) struct TurnStats {
    pub reads: u32,
    pub edits: u32,
    pub cmds: u32,
}

/// Current wall-clock time as milliseconds since the Unix epoch.
pub(crate) fn now_epoch_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}

/// Build a `ToolPreflightResult::Blocked` with the given error message.
pub(crate) fn blocked_result(
    call_id: &str,
    tool_name: &str,
    output: impl Into<String>,
) -> ToolPreflightResult {
    ToolPreflightResult::Blocked(cade_agent::tools::ToolResult {
        tool_call_id: call_id.to_string(),
        tool_name: tool_name.to_string(),
        output: output.into(),
        is_error: true,
ui_resource_uri: None,
    })
}

pub mod agent;
pub mod env_context;
pub mod stream;
