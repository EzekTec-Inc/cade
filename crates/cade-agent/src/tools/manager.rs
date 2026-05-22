use crate::Result;
use serde_json::Value;

#[cfg(feature = "desktop")]
use super::desktop::{
    DesktopCaptureTool, DesktopControlTool, DesktopListWindowsTool, DesktopNotifyTool,
};
use super::{
    ask::AskUserQuestionTool,
    bash::BashTool,
    fs::{ApplyPatchTool, EditTool, ReadTool, WriteTool},
    plan::{
        EnterPlanModeTool, ExitPlanModeTool, FinishTaskTool, SetPlanTool, TodoWriteTool,
        UpdatePlanTool,
    },
    search::{GlobTool, GrepTool},
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
    pub ui_resource_uri: Option<String>,
}

/// Dispatch a tool call by name to its local Rust implementation or to an MCP server.
pub async fn dispatch(
    tool_call_id: String,
    tool_name: &str,
    arguments: &Value,
    mcp: &McpManager,
    allowed_paths: Option<&[String]>,
) -> ToolResult {
    // RBAC path check for file I/O
    if let Some(allowed) = allowed_paths
        && matches!(
            tool_name,
            "read_file"
                | "ReadFileGemini"
                | "write_file"
                | "WriteFileGemini"
                | "edit_file"
                | "Replace"
                | "apply_patch"
                | "grep"
                | "SearchFileContent"
                | "glob"
                | "GlobGemini"
        )
    {
        let target_path = arguments["path"]
            .as_str()
            .or_else(|| arguments["file_path"].as_str())
            .unwrap_or("");
        if !target_path.is_empty() {
            // Ensure target_path resolves to under one of the allowed_paths
            let target_path = std::path::Path::new(target_path)
                .canonicalize()
                .unwrap_or_else(|_| std::path::PathBuf::from(target_path));
            let target_str = target_path.to_string_lossy().to_string();
            let is_allowed = allowed
                .iter()
                .any(|p| target_str.starts_with(p) || target_path.starts_with(p));
            if !is_allowed {
                return ToolResult {
                    tool_call_id,
                    tool_name: tool_name.to_string(),
                    output: format!(
                        "[Blocked by RBAC] Path '{}' is outside the allowed sandbox paths: {:?}",
                        target_path.display(),
                        allowed
                    ),
                    is_error: true,
                    ui_resource_uri: None,
                };
            }
        }
    }

    // Try native tools first, fall through to MCP
    let (output, is_error, ui_resource_uri) = match run_native_tool(tool_name, arguments).await {
        Some(Ok(out)) => (out, false, None),
        Some(Err(e)) => (format!("Error: {e}"), true, None),
        None => {
            // Not a native tool — try MCP servers
            match mcp.call_tool(tool_name, arguments).await {
                Some(Ok((out, err_flag, uri))) => (out, err_flag, uri),
                Some(Err(e)) => {
                    let msg = e.to_string();
                    // rmcp already formats errors as "Mcp error: -32XXX: ..." — avoid
                    // double-prefixing as "MCP error: Mcp error: -32XXX: ...".
                    if msg.starts_with("Mcp error:") || msg.starts_with("MCP error:") {
                        (msg, true, None)
                    } else {
                        (format!("MCP error: {msg}"), true, None)
                    }
                }
                None => (format!("Unknown tool: '{tool_name}'"), true, None),
            }
        }
    };

    ToolResult {
        tool_call_id,
        tool_name: tool_name.to_string(),
        output,
        is_error,
        ui_resource_uri,
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
        "EnterPlanMode" => Err(crate::Error::custom(
            "Permission denied: agent mode changes are disabled in settings.json",
        )),
        "ExitPlanMode" => Err(crate::Error::custom(
            "Permission denied: agent mode changes are disabled in settings.json. Please report your findings to the user and present them with summarized next steps based on your findings.",
        )),
        // TodoWrite — file persistence; SetPlan/UpdatePlan are intercepted in
        // try_native_intercept (they need TuiApp access) before reaching here.
        // WriteTodos is kept as a backward-compat alias for TodoWrite.
        "TodoWrite" | "WriteTodos" => TodoWriteTool::run(args).await,
        "finish_task" => {
            // Audit log generation — runs both client-side and headless.
            let summary = args
                .get("summary")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();
            let reason = args
                .get("reason")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();
            let git_output = std::process::Command::new("git")
                .args(["status", "--porcelain"])
                .output()
                .map(|o| String::from_utf8_lossy(&o.stdout).to_string())
                .unwrap_or_default();
            let files_modified = if git_output.trim().is_empty() {
                "None".to_string()
            } else {
                git_output
                    .lines()
                    .map(|l| format!("- {}", l.trim()))
                    .collect::<Vec<_>>()
                    .join("\n")
            };
            let timestamp = chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Secs, true);
            let log_entry = format!(
                "\n## {} — {}\n\n**Reason:** {}\n\n**Files modified:**\n{}\n\n---\n",
                timestamp, summary, reason, files_modified
            );
            let path = std::path::Path::new("CADE_AUDIT.md");
            let existing = std::fs::read_to_string(path)
                .unwrap_or_else(|_| "# CADE Audit Log\n\n".to_string());
            std::fs::write(path, format!("{}{}", existing, log_entry))
                .map(|_| "Task finished. Audit log appended to CADE_AUDIT.md.".to_string())
                .map_err(|e| crate::Error::custom(format!("Failed to write CADE_AUDIT.md: {e}")))
        }
        // Desktop extensions
        #[cfg(feature = "desktop")]
        "desktop_screenshot" => DesktopCaptureTool::run(args).await,
        #[cfg(feature = "desktop")]
        "desktop_list_windows" => DesktopListWindowsTool::run(args).await,
        #[cfg(feature = "desktop")]
        "desktop_control" => DesktopControlTool::run(args).await,
        #[cfg(feature = "desktop")]
        "desktop_notify" => DesktopNotifyTool::run(args).await,
        _other => return None,
    })
}

fn rename_schema(mut schema: Value, new_name: &str) -> Value {
    if let Some(obj) = schema.as_object_mut() {
        obj.insert("name".to_string(), Value::String(new_name.to_string()));
    }
    schema
}

/// Maps model-specific tool aliases to their canonical native names.
pub fn canonical_name(name: &str) -> &str {
    match name {
        "RunShellCommand" => "bash",
        "ReadFileGemini" => "read_file",
        "WriteFileGemini" => "write_file",
        "Replace" => "edit_file",
        "SearchFileContent" => "grep",
        "GlobGemini" => "glob",
        "WriteTodos" => "TodoWrite",
        _ => name,
    }
}

/// Strip the MCP server prefix from a tool name.
///
/// `"developer__write_file"` → `"write_file"`, `"write_file"` → `"write_file"`.
///
/// M6: uses `find` (FIRST `__`) not `rfind` so MCP tools whose base name
/// itself contains `__` round-trip correctly — the `__` separator we want
/// to strip is always the server-name/tool-name boundary, which is the
/// first occurrence.
pub fn strip_mcp_prefix(name: &str) -> &str {
    if let Some(pos) = name.find("__") {
        &name[pos + 2..]
    } else {
        name
    }
}

/// Returns `true` if `name` is a file-editing tool — native, Gemini alias,
/// or MCP-prefixed.  Used for `recent_edits` tracking.
pub fn is_file_edit_tool(name: &str) -> bool {
    // Resolve Gemini/Codex aliases first, then strip MCP prefix.
    let base = strip_mcp_prefix(canonical_name(name));
    matches!(
        base,
        "write_file"
            | "edit_file"
            | "apply_patch"
            | "apply_edit"
            | "replace_in_file"
            | "edit_block"
    )
}

/// All tool JSON schemas for a given toolset.
pub fn schemas_for_toolset(toolset: Toolset, allow_agent_mode_changes: bool) -> Vec<Value> {
    #[cfg(feature = "desktop")]
    let desktop = vec![
        DesktopCaptureTool::schema(),
        DesktopListWindowsTool::schema(),
        DesktopControlTool::schema(),
        DesktopNotifyTool::schema(),
    ];
    #[cfg(not(feature = "desktop"))]
    let desktop: Vec<Value> = vec![];
    let mut schemas = match toolset {
        Toolset::Codex => vec![
            BashTool::schema(),
            ReadTool::schema(),
            ApplyPatchTool::schema(), // patch-based edit + write
            GrepTool::schema(),
            GlobTool::schema(),
            SetPlanTool::schema(),
            UpdatePlanTool::schema(),
            FinishTaskTool::schema(),
        ],
        Toolset::Gemini => vec![
            rename_schema(BashTool::schema(), "RunShellCommand"),
            rename_schema(ReadTool::schema(), "ReadFileGemini"),
            rename_schema(WriteTool::schema(), "WriteFileGemini"),
            rename_schema(EditTool::schema(), "Replace"),
            rename_schema(GrepTool::schema(), "SearchFileContent"),
            rename_schema(GlobTool::schema(), "GlobGemini"),
            SetPlanTool::schema(),
            UpdatePlanTool::schema(),
            FinishTaskTool::schema(),
        ],
        _ => vec![
            BashTool::schema(),
            ReadTool::schema(),
            WriteTool::schema(),
            EditTool::schema(), // string-replace
            GrepTool::schema(),
            GlobTool::schema(),
            SetPlanTool::schema(),
            UpdatePlanTool::schema(),
            FinishTaskTool::schema(),
        ],
    };
    if allow_agent_mode_changes {
        schemas.push(EnterPlanModeTool::schema());
        schemas.push(ExitPlanModeTool::schema());
    }
    schemas.extend(desktop);
    // AskUserQuestion is intercepted before dispatch — but the schema must still be
    // sent to the LLM so it knows the tool exists.
    schemas.push(AskUserQuestionTool::schema());
    schemas
}

/// Backwards-compat alias (Default toolset).
pub fn all_schemas(allow_agent_mode_changes: bool) -> Vec<Value> {
    schemas_for_toolset(Toolset::Default, allow_agent_mode_changes)
}

/// Filter schemas to only the named tools. Names are case-insensitive.
/// Used to implement `--tools "bash,read_file"`.
pub fn schemas_for_names(
    toolset: Toolset,
    names: &[String],
    allow_agent_mode_changes: bool,
) -> Vec<Value> {
    let lower: std::collections::HashSet<String> = names.iter().map(|n| n.to_lowercase()).collect();
    schemas_for_toolset(toolset, allow_agent_mode_changes)
        .into_iter()
        .filter(|s| {
            s["name"]
                .as_str()
                .map(|n| lower.contains(&n.to_lowercase()))
                .unwrap_or(false)
        })
        .collect()
}

/// Returns true if the tool is an MCP tool that can mutate state.
pub async fn is_mcp_write_tool(name: &str, mcp: &McpManager) -> bool {
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
        let schemas = schemas_for_toolset(Toolset::Default, false);
        let names: Vec<&str> = schemas.iter().filter_map(|s| s["name"].as_str()).collect();
        assert!(names.contains(&"bash"), "missing bash in {names:?}");
        assert!(
            names.contains(&"edit_file"),
            "missing edit_file in {names:?}"
        );
        assert!(
            names.contains(&"read_file"),
            "missing read_file in {names:?}"
        );
        assert!(
            names.contains(&"write_file"),
            "missing write_file in {names:?}"
        );
        assert!(names.contains(&"grep"), "missing grep in {names:?}");
        assert!(names.contains(&"glob"), "missing glob in {names:?}");
    }

    #[test]
    fn codex_toolset_has_apply_patch() {
        let schemas = schemas_for_toolset(Toolset::Codex, false);
        let names: Vec<&str> = schemas.iter().filter_map(|s| s["name"].as_str()).collect();
        assert!(
            names.contains(&"apply_patch"),
            "missing apply_patch in {names:?}"
        );
        assert!(
            !names.contains(&"write_file"),
            "should not have write_file in Codex toolset"
        );
    }

    #[test]
    fn gemini_toolset_has_renamed_tools() {
        let schemas = schemas_for_toolset(Toolset::Gemini, false);
        let names: Vec<&str> = schemas.iter().filter_map(|s| s["name"].as_str()).collect();
        assert!(
            names.contains(&"RunShellCommand"),
            "missing RunShellCommand in {names:?}"
        );
        assert!(
            names.contains(&"Replace"),
            "missing Replace (Gemini edit) in {names:?}"
        );
        assert!(
            names.contains(&"ReadFileGemini"),
            "missing ReadFileGemini in {names:?}"
        );
    }

    #[test]
    fn all_toolsets_include_ask_user_question() {
        for ts in [Toolset::Default, Toolset::Codex, Toolset::Gemini] {
            let schemas = schemas_for_toolset(ts, false);
            let names: Vec<&str> = schemas.iter().filter_map(|s| s["name"].as_str()).collect();
            assert!(
                names.contains(&"ask_user_question"),
                "missing ask_user_question in {ts} toolset"
            );
        }
    }

    #[test]
    fn all_toolsets_include_desktop_tools() {
        for ts in [Toolset::Default, Toolset::Codex, Toolset::Gemini] {
            let schemas = schemas_for_toolset(ts, false);
            let names: Vec<&str> = schemas.iter().filter_map(|s| s["name"].as_str()).collect();
            assert!(
                names.contains(&"desktop_screenshot"),
                "missing desktop_screenshot in {ts}"
            );
            assert!(
                names.contains(&"desktop_list_windows"),
                "missing desktop_list_windows in {ts}"
            );
        }
    }

    // -- all_schemas

    #[test]
    fn all_schemas_is_default_toolset() {
        let all = all_schemas(false);
        let default = schemas_for_toolset(Toolset::Default, false);
        assert_eq!(all.len(), default.len());
    }

    // -- schemas_for_names

    #[test]
    fn schemas_for_names_filters() {
        let names = vec!["bash".to_string(), "grep".to_string()];
        let schemas = schemas_for_names(Toolset::Default, &names, false);
        assert_eq!(schemas.len(), 2);
        let schema_names: Vec<&str> = schemas.iter().filter_map(|s| s["name"].as_str()).collect();
        assert!(schema_names.contains(&"bash"));
        assert!(schema_names.contains(&"grep"));
    }

    #[test]
    fn schemas_for_names_case_insensitive() {
        let names = vec!["BASH".to_string()];
        let schemas = schemas_for_names(Toolset::Default, &names, false);
        assert_eq!(schemas.len(), 1);
        assert_eq!(schemas[0]["name"].as_str(), Some("bash"));
    }

    #[test]
    fn schemas_for_names_empty() {
        let schemas = schemas_for_names(Toolset::Default, &[], false);
        assert!(schemas.is_empty());
    }

    // -- run_native_tool

    #[tokio::test]
    async fn native_tool_unknown_returns_none() {
        let result = run_native_tool("nonexistent_tool", &json!({})).await;
        assert!(result.is_none());
    }

    #[tokio::test]
    async fn native_tool_bash_runs() -> Result<()> {
        // -- Exec
        let args = json!({"command": "echo hello"});
        let result = run_native_tool("bash", &args).await;

        // -- Check
        let output = result.ok_or("Should return Some")??;
        assert!(output.contains("hello"), "got: {output}");

        Ok(())
    }

    #[tokio::test]
    async fn native_tool_grep_runs() -> Result<()> {
        // -- Exec
        let args = json!({"pattern": "fn main", "path": ".", "include": "*.rs"});
        let result = run_native_tool("grep", &args).await;

        // -- Check
        let output = result.ok_or("Should return Some")??;
        assert!(!output.is_empty());

        Ok(())
    }

    #[tokio::test]
    async fn native_tool_glob_runs() -> Result<()> {
        // -- Exec
        let args = json!({"pattern": "**/*.toml"});
        let result = run_native_tool("glob", &args).await;

        // -- Check
        let output = result.ok_or("Should return Some")??;
        assert!(output.contains("Cargo.toml"), "got: {output}");

        Ok(())
    }

    // -- is_file_edit_tool

    #[test]
    fn is_file_edit_native_names() {
        assert!(is_file_edit_tool("write_file"));
        assert!(is_file_edit_tool("edit_file"));
        assert!(is_file_edit_tool("apply_patch"));
        assert!(is_file_edit_tool("Replace"));
        assert!(is_file_edit_tool("WriteFileGemini"));
    }

    #[test]
    fn is_file_edit_mcp_prefixed_names() {
        assert!(is_file_edit_tool("developer__write_file"));
        assert!(is_file_edit_tool("developer__replace_in_file"));
        assert!(is_file_edit_tool("desktop-commander__write_file"));
        assert!(is_file_edit_tool("desktop-commander__edit_block"));
        assert!(is_file_edit_tool("cade-ide-mcp__apply_edit"));
    }

    #[test]
    fn is_file_edit_rejects_non_edit_tools() {
        assert!(!is_file_edit_tool("read_file"));
        assert!(!is_file_edit_tool("bash"));
        assert!(!is_file_edit_tool("developer__read_file"));
        assert!(!is_file_edit_tool("grep"));
        assert!(!is_file_edit_tool("run_subagent"));
    }

    // -- strip_mcp_prefix (M6)

    #[test]
    fn strip_mcp_prefix_basic() {
        assert_eq!(strip_mcp_prefix("developer__write_file"), "write_file");
    }

    #[test]
    fn strip_mcp_prefix_no_prefix_passes_through() {
        assert_eq!(strip_mcp_prefix("write_file"), "write_file");
    }

    /// M6: `strip_mcp_prefix` must strip only the FIRST `__` (the
    /// server-name / tool-name boundary), so MCP tools whose base name
    /// itself contains `__` round-trip correctly.  Using `rfind` would
    /// drop everything but the trailing segment.
    #[test]
    fn strip_mcp_prefix_handles_double_underscore_in_tool_name() {
        // hypothetical MCP tool whose base name contains `__`
        assert_eq!(strip_mcp_prefix("server__nested__tool"), "nested__tool");
    }

    // -- Schema validation

    #[test]
    fn all_schemas_have_name_and_description() {
        for ts in [Toolset::Default, Toolset::Codex, Toolset::Gemini] {
            for schema in schemas_for_toolset(ts, false) {
                let name = schema["name"].as_str();
                assert!(name.is_some(), "schema missing name: {schema}");
                let desc = schema["description"].as_str();
                assert!(
                    desc.is_some(),
                    "schema '{}' missing description",
                    name.unwrap_or("?")
                );
            }
        }
    }

    // -- Bug 7: dispatch does NOT handle interactive-only tools
    // This proves the old headless fallback was broken — `dispatch()` returns
    // "Unknown tool" for run_subagent because it's not in `run_native_tool`.
    // The headless code now intercepts these BEFORE calling dispatch.

    #[tokio::test]
    async fn dispatch_returns_unknown_for_run_subagent() {
        let mcp = crate::mcp::McpManager::empty();
        let result = dispatch(
            "tc_1".into(),
            "run_subagent",
            &serde_json::json!({"prompt": "test"}),
            &mcp,
            None,
        )
        .await;
        assert!(result.is_error, "run_subagent should error in dispatch");
        assert!(
            result.output.contains("Unknown tool"),
            "expected 'Unknown tool', got: {}",
            result.output
        );
    }

    #[tokio::test]
    async fn dispatch_returns_unknown_for_ask_user_question() {
        let mcp = crate::mcp::McpManager::empty();
        let result = dispatch(
            "tc_2".into(),
            "ask_user_question",
            &serde_json::json!({}),
            &mcp,
            None,
        )
        .await;
        assert!(result.is_error);
        assert!(result.output.contains("Unknown tool"));
    }
}
