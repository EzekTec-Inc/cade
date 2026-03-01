use anyhow::Result;
use serde_json::Value;

use super::{
    bash::BashTool,
    desktop::{DesktopCaptureTool, DesktopControlTool, DesktopListWindowsTool, DesktopNotifyTool},
    fs::{ApplyPatchTool, EditTool, ReadTool, WriteTool},
    search::{GlobTool, GrepTool},
};
use crate::toolsets::Toolset;

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
        "bash"        => BashTool::run(args).await,
        "read_file"   => ReadTool::run(args).await,
        "write_file"  => WriteTool::run(args).await,
        "edit_file"   => EditTool::run(args).await,
        "apply_patch" => ApplyPatchTool::run(args).await,
        "grep"        => GrepTool::run(args).await,
        "glob"        => GlobTool::run(args).await,
        // Desktop extensions
        "desktop_screenshot"   => DesktopCaptureTool::run(args).await,
        "desktop_list_windows" => DesktopListWindowsTool::run(args).await,
        "desktop_control"      => DesktopControlTool::run(args).await,
        "desktop_notify"       => DesktopNotifyTool::run(args).await,
        other => Err(anyhow::anyhow!("Unknown tool: '{other}'")),
    }
}

/// All tool JSON schemas for a given toolset.
pub fn schemas_for_toolset(toolset: Toolset) -> Vec<Value> {
    let desktop = vec![
        DesktopCaptureTool::schema(),
        DesktopListWindowsTool::schema(),
        DesktopControlTool::schema(),
        DesktopNotifyTool::schema(),
    ];
    let mut schemas = match toolset {
        Toolset::Codex => vec![
            BashTool::schema(),
            ReadTool::schema(),
            ApplyPatchTool::schema(), // patch-based edit + write
            GrepTool::schema(),
            GlobTool::schema(),
        ],
        _ => vec![
            BashTool::schema(),
            ReadTool::schema(),
            WriteTool::schema(),
            EditTool::schema(),       // string-replace
            GrepTool::schema(),
            GlobTool::schema(),
        ],
    };
    schemas.extend(desktop);
    schemas
}

/// Backwards-compat alias (Default toolset).
pub fn all_schemas() -> Vec<Value> {
    schemas_for_toolset(Toolset::Default)
}

/// Returns true if the tool can mutate state (used for permission gating)
pub fn is_write_tool(name: &str) -> bool {
    matches!(
        name,
        "bash" | "write_file" | "edit_file" | "apply_patch"
            | "desktop_control" | "desktop_screenshot"
    )
}
