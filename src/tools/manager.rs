use anyhow::Result;
use serde_json::Value;

use super::{
    bash::BashTool,
    desktop::{DesktopCaptureTool, DesktopControlTool, DesktopListWindowsTool, DesktopNotifyTool},
    fs::{EditTool, ReadTool, WriteTool},
    search::{GlobTool, GrepTool},
};

/// Result of executing a local tool
#[derive(Debug, Clone)]
pub struct ToolResult {
    pub tool_call_id: String,
    pub tool_name: String,
    pub output: String,
    pub is_error: bool,
}

/// Dispatch a tool call by name to its local Rust implementation.
pub async fn dispatch(tool_call_id: String, tool_name: &str, arguments: &Value) -> ToolResult {
    let result = run_tool(tool_name, arguments).await;
    match result {
        Ok(output) => ToolResult {
            tool_call_id,
            tool_name: tool_name.to_string(),
            output,
            is_error: false,
        },
        Err(e) => ToolResult {
            tool_call_id,
            tool_name: tool_name.to_string(),
            output: format!("Error: {e}"),
            is_error: true,
        },
    }
}

async fn run_tool(name: &str, args: &Value) -> Result<String> {
    match name {
        // Core dev tools
        "bash"       => BashTool::run(args).await,
        "read_file"  => ReadTool::run(args).await,
        "write_file" => WriteTool::run(args).await,
        "edit_file"  => EditTool::run(args).await,
        "grep"       => GrepTool::run(args).await,
        "glob"       => GlobTool::run(args).await,
        // Desktop extensions
        "desktop_screenshot"   => DesktopCaptureTool::run(args).await,
        "desktop_list_windows" => DesktopListWindowsTool::run(args).await,
        "desktop_control"      => DesktopControlTool::run(args).await,
        "desktop_notify"       => DesktopNotifyTool::run(args).await,
        other => Err(anyhow::anyhow!("Unknown tool: '{other}'")),
    }
}

/// All tool JSON schemas — sent to Letta for registration
pub fn all_schemas() -> Vec<Value> {
    vec![
        // Core
        BashTool::schema(),
        ReadTool::schema(),
        WriteTool::schema(),
        EditTool::schema(),
        GrepTool::schema(),
        GlobTool::schema(),
        // Desktop
        DesktopCaptureTool::schema(),
        DesktopListWindowsTool::schema(),
        DesktopControlTool::schema(),
        DesktopNotifyTool::schema(),
    ]
}

/// Returns true if the tool can mutate state (used for permission gating)
pub fn is_write_tool(name: &str) -> bool {
    matches!(
        name,
        "bash" | "write_file" | "edit_file"
            | "desktop_control" | "desktop_screenshot"
    )
}
