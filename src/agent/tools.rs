use anyhow::Result;
use serde_json::json;

use super::client::{CreateToolRequest, LettaClient, ToolDef};

/// Register all CADE tools with the Letta server.
/// Returns a map of tool_name -> tool_id.
pub async fn register_cade_tools(client: &LettaClient) -> Result<Vec<ToolDef>> {
    // Get existing tools to avoid duplicates
    let existing = client.list_tools().await.unwrap_or_default();
    let existing_names: std::collections::HashSet<String> =
        existing.iter().map(|t| t.name.clone()).collect();

    let mut registered = Vec::new();

    for spec in tool_specs() {
        if existing_names.contains(&spec.name) {
            // Find existing tool id
            if let Some(t) = existing.iter().find(|t| t.name == spec.name) {
                registered.push(t.clone());
            }
            continue;
        }

        match client.create_tool(spec).await {
            Ok(tool) => registered.push(tool),
            Err(e) => tracing::warn!("Failed to register tool: {e}"),
        }
    }

    Ok(registered)
}

fn tool_specs() -> Vec<CreateToolRequest> {
    vec![
        CreateToolRequest {
            name: "bash".to_string(),
            description: "Execute a shell command and return its stdout/stderr. Use for running builds, tests, git commands, and other shell operations.".to_string(),
            source_code: "def bash(command: str) -> str:\n    import subprocess\n    result = subprocess.run(command, shell=True, capture_output=True, text=True, timeout=120)\n    out = result.stdout\n    if result.stderr:\n        out += '\\nSTDERR:\\n' + result.stderr\n    return out".to_string(),
            source_type: "python".to_string(),
            json_schema: json!({
                "name": "bash",
                "description": "Execute a shell command",
                "parameters": {
                    "type": "object",
                    "properties": {
                        "command": {"type": "string", "description": "Shell command to execute"}
                    },
                    "required": ["command"]
                }
            }),
        },
        CreateToolRequest {
            name: "read_file".to_string(),
            description: "Read file contents, optionally specifying line range with offset/limit.".to_string(),
            source_code: "def read_file(path: str, offset: int = 0, limit: int = 0) -> str:\n    with open(path, 'r', errors='replace') as f:\n        lines = f.readlines()\n    if limit > 0:\n        lines = lines[offset:offset+limit]\n    elif offset > 0:\n        lines = lines[offset:]\n    return ''.join(f'{i+offset+1}→{l}' for i, l in enumerate(lines))".to_string(),
            source_type: "python".to_string(),
            json_schema: json!({
                "name": "read_file",
                "description": "Read file contents",
                "parameters": {
                    "type": "object",
                    "properties": {
                        "path": {"type": "string", "description": "File path to read"},
                        "offset": {"type": "integer", "description": "Start line (0-based)", "default": 0},
                        "limit": {"type": "integer", "description": "Max lines to read (0=all)", "default": 0}
                    },
                    "required": ["path"]
                }
            }),
        },
        CreateToolRequest {
            name: "write_file".to_string(),
            description: "Write content to a file, creating parent directories if needed.".to_string(),
            source_code: "def write_file(path: str, content: str) -> str:\n    import os\n    os.makedirs(os.path.dirname(os.path.abspath(path)), exist_ok=True)\n    with open(path, 'w') as f:\n        f.write(content)\n    return f'Written {len(content)} bytes to {path}'".to_string(),
            source_type: "python".to_string(),
            json_schema: json!({
                "name": "write_file",
                "description": "Write content to a file",
                "parameters": {
                    "type": "object",
                    "properties": {
                        "path": {"type": "string", "description": "File path to write"},
                        "content": {"type": "string", "description": "File content"}
                    },
                    "required": ["path", "content"]
                }
            }),
        },
        CreateToolRequest {
            name: "edit_file".to_string(),
            description: "Replace an exact string in a file (str-replace). old_string must match exactly, including whitespace.".to_string(),
            source_code: "def edit_file(path: str, old_string: str, new_string: str) -> str:\n    with open(path, 'r') as f:\n        content = f.read()\n    count = content.count(old_string)\n    if count == 0:\n        return f'ERROR: old_string not found in {path}'\n    if count > 1:\n        return f'ERROR: old_string found {count} times in {path} (must be unique)'\n    new_content = content.replace(old_string, new_string, 1)\n    with open(path, 'w') as f:\n        f.write(new_content)\n    return f'Replaced 1 occurrence in {path}'".to_string(),
            source_type: "python".to_string(),
            json_schema: json!({
                "name": "edit_file",
                "description": "Replace exact string in file",
                "parameters": {
                    "type": "object",
                    "properties": {
                        "path": {"type": "string", "description": "File path"},
                        "old_string": {"type": "string", "description": "Exact text to replace"},
                        "new_string": {"type": "string", "description": "Replacement text"}
                    },
                    "required": ["path", "old_string", "new_string"]
                }
            }),
        },
        CreateToolRequest {
            name: "grep".to_string(),
            description: "Search file contents with a regex pattern. Returns matching lines with file paths and line numbers.".to_string(),
            source_code: "def grep(pattern: str, path: str = '.', include: str = '') -> str:\n    import re, os\n    results = []\n    for root, dirs, files in os.walk(path):\n        dirs[:] = [d for d in dirs if d not in ['.git','node_modules','target']]\n        for fname in files:\n            if include and not fname.endswith(tuple(include.split(','))):\n                continue\n            fpath = os.path.join(root, fname)\n            try:\n                with open(fpath, 'r', errors='replace') as f:\n                    for i, line in enumerate(f, 1):\n                        if re.search(pattern, line):\n                            results.append(f'{fpath}:{i}:{line.rstrip()}')\n            except Exception:\n                pass\n    return '\\n'.join(results[:200]) if results else 'No matches'".to_string(),
            source_type: "python".to_string(),
            json_schema: json!({
                "name": "grep",
                "description": "Search file contents with regex",
                "parameters": {
                    "type": "object",
                    "properties": {
                        "pattern": {"type": "string", "description": "Regex pattern to search for"},
                        "path": {"type": "string", "description": "Directory or file to search", "default": "."},
                        "include": {"type": "string", "description": "Comma-separated file extensions to filter (e.g. 'rs,toml')", "default": ""}
                    },
                    "required": ["pattern"]
                }
            }),
        },
        CreateToolRequest {
            name: "glob".to_string(),
            description: "Find files matching a glob pattern. Returns file paths sorted by modification time.".to_string(),
            source_code: "def glob(pattern: str, path: str = '.') -> str:\n    import glob as g, os\n    matches = g.glob(os.path.join(path, pattern), recursive=True)\n    matches.sort(key=lambda f: os.path.getmtime(f), reverse=True)\n    return '\\n'.join(matches[:500]) if matches else 'No files found'".to_string(),
            source_type: "python".to_string(),
            json_schema: json!({
                "name": "glob",
                "description": "Find files by glob pattern",
                "parameters": {
                    "type": "object",
                    "properties": {
                        "pattern": {"type": "string", "description": "Glob pattern (e.g. '**/*.rs')"},
                        "path": {"type": "string", "description": "Base directory", "default": "."}
                    },
                    "required": ["pattern"]
                }
            }),
        },
    ]
}
