use anyhow::Result;
use serde_json::Value;

use super::{
    ask::AskUserQuestionTool,
    bash::BashTool,
    desktop::{DesktopCaptureTool, DesktopControlTool, DesktopListWindowsTool, DesktopNotifyTool},
    fs::{ApplyPatchTool, EditTool, ReadTool, WriteTool},
    search::{GlobTool, GrepTool},
    plan::{EnterPlanModeTool, ExitPlanModeTool, TodoWriteTool, UpdatePlanTool, WriteTodosTool},
};
use crate::mcp::McpManager;
use cade_core::toolsets::Toolset;

/// Result of executing a local tool
#[derive(Debug, Clone)]
pub struct ToolResult {
    pub tool_call_id: String,
    pub tool_name: String,
    pub output: String,
    pub is_error: bool,
}

/// Dispatch a tool call by name to its local Rust implementation or to an MCP server.
pub async fn dispatch(
    tool_call_id: String,
    tool_name: &str,
    arguments: &Value,
    mcp: &McpManager,
) -> ToolResult {
    // Try native tools first, fall through to MCP
    let (output, is_error) = match run_native_tool(tool_name, arguments).await {
        Some(Ok(out))  => (out, false),
        Some(Err(e))   => (format!("Error: {e}"), true),
        None => {
            // Not a native tool — try MCP servers
            match mcp.call_tool(tool_name, arguments).await {
                Some(Ok((out, err_flag))) => (out, err_flag),
                Some(Err(e))             => {
                    let msg = e.to_string();
                    // rmcp already formats errors as "Mcp error: -32XXX: ..." — avoid
                    // double-prefixing as "MCP error: Mcp error: -32XXX: ...".
                    if msg.starts_with("Mcp error:") || msg.starts_with("MCP error:") {
                        (msg, true)
                    } else {
                        (format!("MCP error: {msg}"), true)
                    }
                },
                None => (format!("Unknown tool: '{tool_name}'"), true),
            }
        }
    };

    ToolResult {
        tool_call_id,
        tool_name: tool_name.to_string(),
        output,
        is_error,
    }
}

/// Run a native (built-in Rust) tool. Returns None if the tool name is unknown.
async fn run_native_tool(name: &str, args: &Value) -> Option<Result<String>> {
    Some(match name {
        // Core dev tools
        "bash" | "RunShellCommand" => BashTool::run(args).await,
        "read_file" | "ReadFileGemini" => ReadTool::run(args).await,
        "write_file" | "WriteFileGemini" => WriteTool::run(args).await,
        "edit_file" | "Replace" => EditTool::run(args).await,
        "apply_patch" => ApplyPatchTool::run(args).await,
        "grep" | "SearchFileContent" => GrepTool::run(args).await,
        "glob" | "GlobGemini" => GlobTool::run(args).await,
        "EnterPlanMode" => Ok("Plan mode entered. File modifications are now blocked. Use ExitPlanMode to resume normal operation.".to_string()),
        "ExitPlanMode" => Ok("Plan mode exited. Normal operation resumed.".to_string()),
        "TodoWrite" | "UpdatePlan" | "WriteTodos" => TodoWriteTool::run(args).await,
        // Desktop extensions
        "desktop_screenshot"   => DesktopCaptureTool::run(args).await,
        "desktop_list_windows" => DesktopListWindowsTool::run(args).await,
        "desktop_control"      => DesktopControlTool::run(args).await,
        "desktop_notify"       => DesktopNotifyTool::run(args).await,
        _other => return None,
    })
}

fn rename_schema(mut schema: Value, new_name: &str) -> Value {
    if let Some(obj) = schema.as_object_mut() {
        obj.insert("name".to_string(), Value::String(new_name.to_string()));
    }
    schema
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
            EnterPlanModeTool::schema(),
            ExitPlanModeTool::schema(),
            UpdatePlanTool::schema(),
        ],
        Toolset::Gemini => vec![
            rename_schema(BashTool::schema(), "RunShellCommand"),
            rename_schema(ReadTool::schema(), "ReadFileGemini"),
            rename_schema(WriteTool::schema(), "WriteFileGemini"),
            rename_schema(EditTool::schema(), "Replace"),
            rename_schema(GrepTool::schema(), "SearchFileContent"),
            rename_schema(GlobTool::schema(), "GlobGemini"),
            EnterPlanModeTool::schema(),
            ExitPlanModeTool::schema(),
            WriteTodosTool::schema(),
        ],
        _ => vec![
            BashTool::schema(),
            ReadTool::schema(),
            WriteTool::schema(),
            EditTool::schema(),       // string-replace
            GrepTool::schema(),
            GlobTool::schema(),
            EnterPlanModeTool::schema(),
            ExitPlanModeTool::schema(),
            TodoWriteTool::schema(),
        ],
    };
    schemas.extend(desktop);
    // AskUserQuestion is intercepted before dispatch — but the schema must still be
    // sent to the LLM so it knows the tool exists.
    schemas.push(AskUserQuestionTool::schema());
    schemas
}

/// Backwards-compat alias (Default toolset).
pub fn all_schemas() -> Vec<Value> {
    schemas_for_toolset(Toolset::Default)
}

/// Filter schemas to only the named tools. Names are case-insensitive.
/// Used to implement `--tools "bash,read_file"`.
pub fn schemas_for_names(toolset: Toolset, names: &[String]) -> Vec<Value> {
    let lower: std::collections::HashSet<String> =
        names.iter().map(|n| n.to_lowercase()).collect();
    schemas_for_toolset(toolset)
        .into_iter()
        .filter(|s| {
            s["name"]
                .as_str()
                .map(|n| lower.contains(&n.to_lowercase()))
                .unwrap_or(false)
        })
        .collect()
}

/// Returns true if the native tool can mutate state (used for permission gating).
pub fn is_native_write_tool(name: &str) -> bool {
    matches!(
        name,
        "bash" | "write_file" | "edit_file" | "apply_patch"
            | "desktop_control" | "desktop_screenshot"
    )
}

/// Returns true if the tool (native or MCP) can mutate state.
pub async fn is_write_tool(name: &str, mcp: &McpManager) -> bool {
    if is_native_write_tool(name) {
        return true;
    }
    if mcp.owns_tool(name).await {
        return mcp.is_write_tool(name).await;
    }
    false
}
