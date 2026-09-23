//! Deep module for normalizing MCP tool arguments across client and server execution seams.
//!
//! # Architecture (Codebase Design)
//!
//! **Interface**:
//! - `normalize_mcp_arguments(tool_name, arguments, workspace_dir) -> Value`
//!
//! **Implementation (Depth)**:
//! - Hides the complexity of resolving relative, dot (`.`), tilde (`~`), and working-directory
//!   dependent paths passed to server-hosted and local MCP tools.
//! - Canonicalizes paths against the active agent workspace directory, guaranteeing that
//!   MCP background daemon processes (which may run with a different CWD) receive valid,
//!   absolute filesystem targets.

use serde_json::{Map, Value};
use std::path::Path;

// region:    --- Constants

/// Known JSON argument keys that represent filesystem paths across tools.
const PATH_KEYS: &[&str] = &[
    "path",
    "file_path",
    "filePath",
    "workspace_path",
    "workspacePath",
    "workspace",
    "dir",
    "directory",
    "cwd",
    "root",
    "project",
    "project_path",
    "projectPath",
    "destination",
    "source",
    "target",
];

// endregion: --- Constants

// region:    --- Public Interface

/// Normalize MCP tool arguments by canonicalizing relative filesystem paths
/// against the caller's workspace directory.
///
/// # Arguments
/// - `tool_name`: Name of the MCP tool being called (e.g. `cade-rag-mcp__semantic_search`).
/// - `arguments`: The raw JSON arguments object from the caller or LLM.
/// - `workspace_dir`: The active agent or session workspace root directory.
pub fn normalize_mcp_arguments(_tool_name: &str, arguments: &Value, workspace_dir: &Path) -> Value {
    match arguments {
        Value::Object(map) => {
            let mut normalized = Map::new();
            for (key, val) in map {
                if is_path_key(key) {
                    normalized.insert(key.clone(), normalize_path_value(val, workspace_dir));
                } else if key == "paths" || key == "files" {
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
    let trimmed = raw.trim();

    // 1. Handle dot and empty paths
    if trimmed.is_empty() || trimmed == "." || trimmed == "./" {
        return canonical_or_lossy(workspace_dir);
    }

    // 2. Handle tilde (home directory) expansion
    if let Some(stripped) = trimmed.strip_prefix("~/") {
        if let Some(home) = dirs::home_dir() {
            return canonical_or_lossy(&home.join(stripped));
        }
    } else if trimmed == "~"
        && let Some(home) = dirs::home_dir()
    {
        return canonical_or_lossy(&home);
    }

    let p = Path::new(trimmed);

    // 3. Already absolute: preserve intact without resolving system symlinks
    if p.is_absolute() {
        return trimmed.to_string();
    }

    // 4. Relative to workspace directory
    let joined = workspace_dir.join(p);
    canonical_or_lossy(&joined)
}

fn canonical_or_lossy(path: &Path) -> String {
    match path.canonicalize() {
        Ok(c) => c.to_string_lossy().into_owned(),
        Err(_) => path.to_string_lossy().into_owned(),
    }
}

// endregion: --- Support Helpers

// region:    --- Tests

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    type Result<T> = core::result::Result<T, Box<dyn std::error::Error>>;

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
        assert_eq!(normalized["path"], "/home/user/my-project");
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
        assert_eq!(normalized["path"], "/home/user/my-project/src/main.rs");
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
        assert_eq!(paths[0], "/home/user/my-project");
        assert_eq!(paths[1], "/home/user/my-project/src");
        assert_eq!(paths[2], "/home/user/my-project/tests");
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
