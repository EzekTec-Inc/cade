use anyhow::Result;
use serde_json::Value;

use super::{bash::BashTool, fs::{EditTool, ReadTool, WriteTool}, search::{GlobTool, GrepTool}};

/// Result of executing a local tool
#[derive(Debug, Clone)]
pub struct ToolResult {
    pub tool_call_id: String,
    pub tool_name: String,
    pub output: String,
    pub is_error: bool,
}

/// Parse a tool_call_message from the agent response and dispatch locally.
///
/// `tool_call_id`  — the id field from the tool_call_message
/// `tool_name`     — the name of the tool to execute
/// `arguments`     — parsed JSON arguments for the tool
pub async fn dispatch(
    tool_call_id: String,
    tool_name: &str,
    arguments: &Value,
) -> ToolResult {
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
        "bash" => BashTool::run(args).await,
        "read_file" => ReadTool::run(args).await,
        "write_file" => WriteTool::run(args).await,
        "edit_file" => EditTool::run(args).await,
        "grep" => GrepTool::run(args).await,
        "glob" => GlobTool::run(args).await,
        other => Err(anyhow::anyhow!("Unknown tool: {other}")),
    }
}

/// All tool JSON schemas — used to register with Letta
pub fn all_schemas() -> Vec<Value> {
    vec![
        BashTool::schema(),
        ReadTool::schema(),
        WriteTool::schema(),
        EditTool::schema(),
        GrepTool::schema(),
        GlobTool::schema(),
    ]
}

/// Returns true if this is a write-capable tool (for permission gating)
pub fn is_write_tool(name: &str) -> bool {
    matches!(name, "bash" | "write_file" | "edit_file")
}
