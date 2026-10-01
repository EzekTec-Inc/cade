//! Prepare filesystem arguments within Execution Scope without reinterpreting identities.
//! Resolution is lexical and idempotent: authorization and execution see the same value,
//! without blocking filesystem probes or resolving symlinks again after approval.
//! Ambiguous arguments need an identified filesystem operation; unknown values are preserved.

use serde_json::{Map, Value};
use std::path::Path;

// region:    --- Constants

/// Known JSON argument keys that represent filesystem paths across tools.
const PATH_KEYS: &[&str] = &[
    "path",
    "file_path",
    "filepath",
    "workspace_path",
    "workspacepath",
    "workspace",
    "dir",
    "directory",
    "cwd",
    "root",
    "project_path",
    "projectpath",
];

// endregion: --- Constants

// region:    --- Public Interface

/// Normalize MCP tool arguments by resolving relative filesystem paths
/// against the caller's workspace directory.
///
/// # Arguments
/// - `tool_name`: Name of the MCP tool being called (e.g. `cade-rag-mcp__semantic_search`).
/// - `arguments`: The raw JSON arguments object from the caller or LLM.
/// - `workspace_dir`: The active agent or session workspace root directory.
pub fn normalize_mcp_arguments(tool_name: &str, arguments: &Value, workspace_dir: &Path) -> Value {
    // Repository-content paths are relative to the remote repository, not this workspace.
    if arguments.get("owner").is_some() && arguments.get("repo").is_some() {
        return arguments.clone();
    }
    let operation = tool_name.rsplit("__").next().unwrap_or(tool_name);
    match arguments {
        Value::Object(map) => {
            let mut normalized = Map::new();
            for (key, val) in map {
                if is_path_key(key) || operation_path_key(operation, key, val) {
                    normalized.insert(key.clone(), normalize_path_value(val, workspace_dir));
                } else if key == "paths" || (key == "files" && operation == "read_multiple_files") {
                    normalized.insert(key.clone(), normalize_paths_array(val, workspace_dir));
                } else {
                    normalized.insert(key.clone(), val.clone());
                }
            }
            Value::Object(normalized)
        }
        other => other.clone(),
    }
}

// endregion: --- Public Interface

// region:    --- Support Helpers

fn is_path_key(key: &str) -> bool {
    let lower = key.to_ascii_lowercase();
    PATH_KEYS.iter().any(|k| lower == *k)
}

fn operation_path_key(operation: &str, key: &str, value: &Value) -> bool {
    match (operation, key) {
        (
            "copy_file" | "move_file" | "copy_directory" | "move_directory",
            "source" | "destination",
        )
        | ("extract_archive", "archive" | "destination")
        | ("create_archive", "output")
        | ("diff_files", "file1" | "file2") => true,
        ("activate_project", "project") => value.as_str().is_some_and(|s| {
            s == "."
                || s == ".."
                || s == "~"
                || s.starts_with("./")
                || s.starts_with("../")
                || s.starts_with("~/")
                || Path::new(s).is_absolute()
        }),
        _ => false,
    }
}

fn normalize_path_value(val: &Value, workspace_dir: &Path) -> Value {
    match val {
        Value::String(s) => Value::String(resolve_to_absolute_string(s, workspace_dir)),
        other => other.clone(),
    }
}

fn normalize_paths_array(val: &Value, workspace_dir: &Path) -> Value {
    match val {
        Value::Array(items) => {
            let updated: Vec<Value> = items
                .iter()
                .map(|item| match item {
                    Value::String(s) => Value::String(resolve_to_absolute_string(s, workspace_dir)),
                    other => other.clone(),
                })
                .collect();
            Value::Array(updated)
        }
        other => other.clone(),
    }
}

/// Convert a path string (dot, tilde, relative, or already absolute) into an absolute path string.
fn resolve_to_absolute_string(raw: &str, workspace_dir: &Path) -> String {
    // A URI or a foreign-platform absolute path must never acquire a local prefix.
    let bytes = raw.as_bytes();
    if raw.contains("://")
        || raw.starts_with(r"\\")
        || (bytes.len() >= 3
            && bytes[0].is_ascii_alphabetic()
            && bytes[1] == b':'
            && matches!(bytes[2], b'/' | b'\\'))
    {
        return raw.to_string();
    }
    if raw == "." || raw == "./" {
        return workspace_dir.to_string_lossy().into_owned();
    }
    // Empty arguments remain invalid rather than silently selecting the workspace.
    if raw.is_empty() {
        return String::new();
    }

    // 2. Handle tilde (home directory) expansion
    if let Some(stripped) = raw.strip_prefix("~/") {
        if let Some(home) = dirs::home_dir() {
            return home.join(stripped).to_string_lossy().into_owned();
        }
    } else if raw == "~"
        && let Some(home) = dirs::home_dir()
    {
        return home.to_string_lossy().into_owned();
    }

    let p = Path::new(raw);

    // 3. Already absolute: preserve intact without resolving system symlinks
    if p.is_absolute() {
        return raw.to_string();
    }

    // 4. Relative to workspace directory
    workspace_dir.join(p).to_string_lossy().into_owned()
}

// endregion: --- Support Helpers

// region:    --- Tests

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    type Result<T> = core::result::Result<T, Box<dyn std::error::Error>>;

    #[test]
    fn preparation_preserves_identities_and_resolves_only_filesystem_arguments() {
        let workspace = std::env::temp_dir().join("cade-argument-scope");
        for (tool, args) in [
            (
                "git__reset",
                json!({"target": "HEAD~1", "files": ["src/lib.rs"]}),
            ),
            (
                "message_agent",
                json!({"target": "helper", "message": "hello"}),
            ),
            (
                "facts__insert",
                json!({"source": "Agent", "target": "Run", "project": "cade"}),
            ),
            (
                "serena__activate_project",
                json!({"project": "registered-name"}),
            ),
            (
                "github__get_file_contents",
                json!({"owner": "example", "repo": "project", "path": "src/lib.rs"}),
            ),
        ] {
            assert_eq!(
                normalize_mcp_arguments(tool, &args, &workspace),
                args,
                "{tool}"
            );
        }
        let args = json!({"filePath": "src/main.rs", "workspacePath": "."});
        let resolved = normalize_mcp_arguments("files__read_file", &args, &workspace);
        assert_eq!(
            resolved["filePath"],
            workspace.join("src/main.rs").to_string_lossy().as_ref()
        );
        assert_eq!(
            resolved["workspacePath"],
            workspace.to_string_lossy().as_ref()
        );
        let resolved = normalize_mcp_arguments(
            "desktop__copy_file",
            &json!({"source": "a", "destination": "b"}),
            &workspace,
        );
        assert_eq!(
            resolved["source"],
            workspace.join("a").to_string_lossy().as_ref()
        );
        assert_eq!(
            resolved["destination"],
            workspace.join("b").to_string_lossy().as_ref()
        );
        let resolved = normalize_mcp_arguments(
            "serena__activate_project",
            &json!({"project": "./src"}),
            &workspace,
        );
        assert_eq!(
            resolved["project"],
            workspace.join("./src").to_string_lossy().as_ref()
        );
    }

    #[test]
    fn preparation_preserves_uris_foreign_absolute_paths_and_filename_whitespace() {
        let workspace = std::env::temp_dir().join("cade-argument-scope");
        for path in [
            "https://example.test/a",
            "file:///workspace/a",
            r"C:\work\file.txt",
            r"\\host\share\file.txt",
        ] {
            let args = json!({"path": path});
            assert_eq!(
                normalize_mcp_arguments("desktop__read_file", &args, &workspace),
                args
            );
        }
        let args = json!({"path": " name with spaces "});
        let prepared = normalize_mcp_arguments("read_file", &args, &workspace);
        assert_eq!(
            prepared["path"],
            workspace
                .join(" name with spaces ")
                .to_string_lossy()
                .as_ref()
        );
        assert_eq!(
            normalize_mcp_arguments("read_file", &prepared, &workspace),
            prepared
        );
    }

    #[test]
    fn test_mcp_normalize_dot_path_resolves_to_workspace() -> Result<()> {
        // -- Setup & Fixtures
        let workspace = Path::new("/home/user/my-project");
        let args = json!({
            "path": ".",
            "query": "authentication flow"
        });

        // -- Exec
        let normalized = normalize_mcp_arguments("cade-rag-mcp__semantic_search", &args, workspace);

        // -- Check
        assert_eq!(normalized["path"], workspace.to_string_lossy().as_ref());
        assert_eq!(normalized["query"], "authentication flow");
        Ok(())
    }

    #[test]
    fn test_mcp_normalize_relative_subpath() -> Result<()> {
        // -- Setup & Fixtures
        let workspace = Path::new("/home/user/my-project");
        let args = json!({
            "path": "src/main.rs",
            "limit": 10
        });

        // -- Exec
        let normalized = normalize_mcp_arguments("serena__read_file", &args, workspace);

        // -- Check
        assert_eq!(
            normalized["path"],
            workspace.join("src/main.rs").to_string_lossy().as_ref()
        );
        assert_eq!(normalized["limit"], 10);
        Ok(())
    }

    #[test]
    fn test_mcp_normalize_preserves_absolute_path() -> Result<()> {
        // -- Setup & Fixtures
        let workspace = Path::new("/home/user/my-project");
        let args = json!({
            "path": "/etc/hosts",
            "recursive": false
        });

        // -- Exec
        let normalized = normalize_mcp_arguments("desktop__list_dir", &args, workspace);

        // -- Check
        assert_eq!(normalized["path"], "/etc/hosts");
        assert_eq!(normalized["recursive"], false);
        Ok(())
    }

    #[test]
    fn test_mcp_normalize_paths_array() -> Result<()> {
        // -- Setup & Fixtures
        let workspace = Path::new("/home/user/my-project");
        let args = json!({
            "paths": [".", "src", "tests"],
            "format": "tar.gz"
        });

        // -- Exec
        let normalized = normalize_mcp_arguments("desktop__create_archive", &args, workspace);

        // -- Check
        let paths = normalized["paths"]
            .as_array()
            .ok_or("paths should be array")?;
        assert_eq!(paths.len(), 3);
        assert_eq!(paths[0], workspace.to_string_lossy().as_ref());
        assert_eq!(paths[1], workspace.join("src").to_string_lossy().as_ref());
        assert_eq!(paths[2], workspace.join("tests").to_string_lossy().as_ref());
        Ok(())
    }

    #[test]
    fn test_mcp_normalize_non_object_passthrough() -> Result<()> {
        // -- Setup & Fixtures
        let workspace = Path::new("/home/user/my-project");
        let args = json!("just a string");

        // -- Exec
        let normalized = normalize_mcp_arguments("some_tool", &args, workspace);

        // -- Check
        assert_eq!(normalized, "just a string");
        Ok(())
    }
}

// endregion: --- Tests
