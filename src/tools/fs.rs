use anyhow::{Context, Result};
use serde_json::Value;
use std::path::Path;

// ── Read ─────────────────────────────────────────────────────────────────────

pub struct ReadTool;

impl ReadTool {
    pub async fn run(args: &Value) -> Result<String> {
        let path = args["path"]
            .as_str()
            .ok_or_else(|| anyhow::anyhow!("read_file: missing 'path'"))?;
        let offset = args["offset"].as_u64().unwrap_or(0) as usize;
        let limit = args["limit"].as_u64().unwrap_or(0) as usize;

        let content = std::fs::read_to_string(path)
            .with_context(|| format!("read {path}"))?;

        let lines: Vec<&str> = content.lines().collect();
        let total = lines.len();

        let start = offset;
        let end = if limit > 0 {
            (start + limit).min(total)
        } else {
            total
        };

        let selected = &lines[start.min(total)..end];
        let numbered: String = selected
            .iter()
            .enumerate()
            .map(|(i, line)| format!("{:>4}→{}\n", start + i + 1, line))
            .collect();

        Ok(format!("{numbered}[{total} lines total]"))
    }

    pub fn schema() -> Value {
        serde_json::json!({
            "name": "read_file",
            "description": "Read a file's contents with line numbers. Optionally specify offset/limit for large files.",
            "parameters": {
                "type": "object",
                "properties": {
                    "path":   { "type": "string",  "description": "Absolute or relative file path" },
                    "offset": { "type": "integer", "description": "Start line (0-based, default 0)" },
                    "limit":  { "type": "integer", "description": "Max lines to read (0 = all)" }
                },
                "required": ["path"]
            }
        })
    }
}

// ── Write ─────────────────────────────────────────────────────────────────────

pub struct WriteTool;

impl WriteTool {
    pub async fn run(args: &Value) -> Result<String> {
        let path = args["path"]
            .as_str()
            .ok_or_else(|| anyhow::anyhow!("write_file: missing 'path'"))?;
        let content = args["content"]
            .as_str()
            .ok_or_else(|| anyhow::anyhow!("write_file: missing 'content'"))?;

        if let Some(parent) = Path::new(path).parent() {
            if !parent.as_os_str().is_empty() {
                std::fs::create_dir_all(parent)
                    .with_context(|| format!("create dirs for {path}"))?;
            }
        }

        std::fs::write(path, content)
            .with_context(|| format!("write {path}"))?;

        Ok(format!("Written {} bytes to {path}", content.len()))
    }

    pub fn schema() -> Value {
        serde_json::json!({
            "name": "write_file",
            "description": "Write content to a file. Creates parent directories if needed. Overwrites existing content.",
            "parameters": {
                "type": "object",
                "properties": {
                    "path":    { "type": "string", "description": "File path to write" },
                    "content": { "type": "string", "description": "File content to write" }
                },
                "required": ["path", "content"]
            }
        })
    }
}

// ── Edit (str-replace) ────────────────────────────────────────────────────────

pub struct EditTool;

impl EditTool {
    pub async fn run(args: &Value) -> Result<String> {
        let path = args["path"]
            .as_str()
            .ok_or_else(|| anyhow::anyhow!("edit_file: missing 'path'"))?;
        let old_string = args["old_string"]
            .as_str()
            .ok_or_else(|| anyhow::anyhow!("edit_file: missing 'old_string'"))?;
        let new_string = args["new_string"]
            .as_str()
            .ok_or_else(|| anyhow::anyhow!("edit_file: missing 'new_string'"))?;
        let replace_all = args["replace_all"].as_bool().unwrap_or(false);

        let content = std::fs::read_to_string(path)
            .with_context(|| format!("read {path}"))?;

        let count = content.matches(old_string).count();
        if count == 0 {
            return Ok(format!("ERROR: old_string not found in {path}"));
        }
        if count > 1 && !replace_all {
            return Ok(format!(
                "ERROR: old_string appears {count} times in {path}. \
                 Make it unique or set replace_all=true."
            ));
        }

        let new_content = if replace_all {
            content.replace(old_string, new_string)
        } else {
            content.replacen(old_string, new_string, 1)
        };

        std::fs::write(path, &new_content)
            .with_context(|| format!("write {path}"))?;

        Ok(format!("Replaced {count} occurrence(s) in {path}"))
    }

    pub fn schema() -> Value {
        serde_json::json!({
            "name": "edit_file",
            "description": "Replace an exact string in a file (str-replace). old_string must match exactly, including whitespace and indentation. Must be unique unless replace_all=true.",
            "parameters": {
                "type": "object",
                "properties": {
                    "path":        { "type": "string",  "description": "File path" },
                    "old_string":  { "type": "string",  "description": "Exact string to replace" },
                    "new_string":  { "type": "string",  "description": "Replacement string" },
                    "replace_all": { "type": "boolean", "description": "Replace all occurrences (default false)" }
                },
                "required": ["path", "old_string", "new_string"]
            }
        })
    }
}
