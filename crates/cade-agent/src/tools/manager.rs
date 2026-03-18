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

// endregion: --- Tests

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

// region:    --- Tests

#[cfg(test)]
mod tests {
    #[allow(unused)]
    type Result<T> = core::result::Result<T, Box<dyn std::error::Error>>; // For tests.

    use super::*;
    use serde_json::json;

    // -- schemas_for_toolset

    #[test]
    fn default_toolset_has_bash_and_edit_file() {
        let schemas = schemas_for_toolset(Toolset::Default);
        let names: Vec<&str> = schemas.iter()
            .filter_map(|s| s["name"].as_str())
            .collect();
        assert!(names.contains(&"bash"), "missing bash in {names:?}");
        assert!(names.contains(&"edit_file"), "missing edit_file in {names:?}");
        assert!(names.contains(&"read_file"), "missing read_file in {names:?}");
        assert!(names.contains(&"write_file"), "missing write_file in {names:?}");
        assert!(names.contains(&"grep"), "missing grep in {names:?}");
        assert!(names.contains(&"glob"), "missing glob in {names:?}");
    }

    #[test]
    fn codex_toolset_has_apply_patch() {
        let schemas = schemas_for_toolset(Toolset::Codex);
        let names: Vec<&str> = schemas.iter()
            .filter_map(|s| s["name"].as_str())
            .collect();
        assert!(names.contains(&"apply_patch"), "missing apply_patch in {names:?}");
        assert!(!names.contains(&"write_file"), "should not have write_file in Codex toolset");
    }

    #[test]
    fn gemini_toolset_has_renamed_tools() {
        let schemas = schemas_for_toolset(Toolset::Gemini);
        let names: Vec<&str> = schemas.iter()
            .filter_map(|s| s["name"].as_str())
            .collect();
        assert!(names.contains(&"RunShellCommand"), "missing RunShellCommand in {names:?}");
        assert!(names.contains(&"Replace"), "missing Replace (Gemini edit) in {names:?}");
        assert!(names.contains(&"ReadFileGemini"), "missing ReadFileGemini in {names:?}");
    }

    #[test]
    fn all_toolsets_include_ask_user_question() {
        for ts in [Toolset::Default, Toolset::Codex, Toolset::Gemini] {
            let schemas = schemas_for_toolset(ts);
            let names: Vec<&str> = schemas.iter()
                .filter_map(|s| s["name"].as_str())
                .collect();
            assert!(names.contains(&"ask_user_question"), "missing ask_user_question in {ts} toolset");
        }
    }

    #[test]
    fn all_toolsets_include_desktop_tools() {
        for ts in [Toolset::Default, Toolset::Codex, Toolset::Gemini] {
            let schemas = schemas_for_toolset(ts);
            let names: Vec<&str> = schemas.iter()
                .filter_map(|s| s["name"].as_str())
                .collect();
            assert!(names.contains(&"desktop_screenshot"), "missing desktop_screenshot in {ts}");
            assert!(names.contains(&"desktop_list_windows"), "missing desktop_list_windows in {ts}");
        }
    }

    // -- all_schemas

    #[test]
    fn all_schemas_is_default_toolset() {
        let all = all_schemas();
        let default = schemas_for_toolset(Toolset::Default);
        assert_eq!(all.len(), default.len());
    }

    // -- schemas_for_names

    #[test]
    fn schemas_for_names_filters() {
        let names = vec!["bash".to_string(), "grep".to_string()];
        let schemas = schemas_for_names(Toolset::Default, &names);
        assert_eq!(schemas.len(), 2);
        let schema_names: Vec<&str> = schemas.iter()
            .filter_map(|s| s["name"].as_str())
            .collect();
        assert!(schema_names.contains(&"bash"));
        assert!(schema_names.contains(&"grep"));
    }

    #[test]
    fn schemas_for_names_case_insensitive() {
        let names = vec!["BASH".to_string()];
        let schemas = schemas_for_names(Toolset::Default, &names);
        assert_eq!(schemas.len(), 1);
        assert_eq!(schemas[0]["name"].as_str(), Some("bash"));
    }

    #[test]
    fn schemas_for_names_empty() {
        let schemas = schemas_for_names(Toolset::Default, &[]);
        assert!(schemas.is_empty());
    }

    // -- is_native_write_tool

    #[test]
    fn write_tools_identified() {
        assert!(is_native_write_tool("bash"));
        assert!(is_native_write_tool("write_file"));
        assert!(is_native_write_tool("edit_file"));
        assert!(is_native_write_tool("apply_patch"));
        assert!(is_native_write_tool("desktop_control"));
    }

    #[test]
    fn read_tools_not_write() {
        assert!(!is_native_write_tool("read_file"));
        assert!(!is_native_write_tool("grep"));
        assert!(!is_native_write_tool("glob"));
        assert!(!is_native_write_tool("AskUserQuestion"));
    }

    // -- run_native_tool

    #[tokio::test]
    async fn native_tool_unknown_returns_none() {
        let result = run_native_tool("nonexistent_tool", &json!({})).await;
        assert!(result.is_none());
    }

    #[tokio::test]
    async fn native_tool_bash_runs() {
        let args = json!({"command": "echo hello"});
        let result = run_native_tool("bash", &args).await;
        assert!(result.is_some());
        let output = result.unwrap().unwrap();
        assert!(output.contains("hello"), "got: {output}");
    }

    #[tokio::test]
    async fn native_tool_grep_runs() {
        let args = json!({"pattern": "fn main", "path": ".", "include": "*.rs"});
        let result = run_native_tool("grep", &args).await;
        assert!(result.is_some());
        // Should either find matches or report no matches
        let output = result.unwrap().unwrap();
        assert!(!output.is_empty());
    }

    #[tokio::test]
    async fn native_tool_glob_runs() {
        let args = json!({"pattern": "**/*.toml"});
        let result = run_native_tool("glob", &args).await;
        assert!(result.is_some());
        let output = result.unwrap().unwrap();
        assert!(output.contains("Cargo.toml"), "got: {output}");
    }

    // -- Schema validation

    #[test]
    fn all_schemas_have_name_and_description() {
        for ts in [Toolset::Default, Toolset::Codex, Toolset::Gemini] {
            for schema in schemas_for_toolset(ts) {
                let name = schema["name"].as_str();
                assert!(name.is_some(), "schema missing name: {schema}");
                let desc = schema["description"].as_str();
                assert!(desc.is_some(), "schema '{}' missing description", name.unwrap_or("?"));
            }
        }
    }
}
