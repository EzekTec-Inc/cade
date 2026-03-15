use anyhow::{Context, Result};
use serde_json::Value;
use std::path::{Path, PathBuf};

// ── SEC-A: Opt-in filesystem sandboxing ───────────────────────────────────────

fn fs_root() -> Option<PathBuf> {
    std::env::var("CADE_FS_ROOT").ok().and_then(|v| {
        let v = v.trim().to_string();
        if v.is_empty() { return None; }
        Some(std::fs::canonicalize(&v).unwrap_or_else(|_| PathBuf::from(v)))
    })
}

fn ensure_within_root(root: &Path, raw_path: &str) -> Result<()> {
    let p = Path::new(raw_path);
    let abs = if p.is_absolute() { p.to_path_buf() } else { root.join(p) };
    let mut parts: Vec<std::path::Component> = Vec::new();
    for c in abs.components() {
        match c {
            std::path::Component::ParentDir => { parts.pop(); }
            std::path::Component::CurDir => {}
            other => parts.push(other),
        }
    }
    let normalized: PathBuf = parts.iter().collect();
    let resolved = std::fs::canonicalize(&normalized).unwrap_or(normalized);
    if !resolved.starts_with(root) {
        anyhow::bail!("path '{}' is outside the allowed filesystem root '{}'", raw_path, root.display());
    }
    Ok(())
}

// ── Read ─────────────────────────────────────────────────────────────────────

pub struct ReadTool;

impl ReadTool {
    pub async fn run(args: &Value) -> Result<String> {
        let path = args["path"]
            .as_str()
            .ok_or_else(|| anyhow::anyhow!("read_file: missing 'path'"))?;
        if let Some(ref root) = fs_root() { ensure_within_root(root, path)?; }
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
        if let Some(ref root) = fs_root() { ensure_within_root(root, path)?; }
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
        if let Some(ref root) = fs_root() { ensure_within_root(root, path)?; }
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

// ── ApplyPatch ────────────────────────────────────────────────────────────────
// Unified-diff based editing optimised for OpenAI (Codex/GPT) models which are
// trained to produce patch output rather than string-replace pairs.

pub struct ApplyPatchTool;

/// Validate that all file paths in a unified diff stay within the project and
/// do not use absolute or parent-directory (`..`) segments.  This prevents a
/// malicious or buggy patch from writing outside the working directory when
/// `patch -p1` is invoked.
fn validate_patch_paths(patch_str: &str) -> Result<()> {
    for line in patch_str.lines() {
        let path_opt = if let Some(rest) = line.strip_prefix("--- ") {
            rest.split_whitespace().next()
        } else if let Some(rest) = line.strip_prefix("+++ ") {
            rest.split_whitespace().next()
        } else {
            None
        };

        let Some(path) = path_opt else { continue };
        let p = path.trim();
        if p.is_empty() || p == "/dev/null" {
            continue;
        }

        // Disallow absolute paths and any `..` segment (path traversal).
        if p.starts_with('/') {
            anyhow::bail!("apply_patch: absolute paths are not allowed in patch: '{p}'");
        }
        if p.len() >= 3 {
            let bytes = p.as_bytes();
            if bytes[1] == b':' && (bytes[2] == b'/' || bytes[2] == b'\\') {
                anyhow::bail!("apply_patch: absolute Windows-style paths are not allowed in patch: '{p}'");
            }
        }
        if p.split(&['/','\\'][..]).any(|seg| seg == "..") {
            anyhow::bail!("apply_patch: parent-directory segments ('..') are not allowed in patch path: '{p}'");
        }
    }
    Ok(())
}

impl ApplyPatchTool {
    pub async fn run(args: &Value) -> Result<String> {
        let patch_str = args["patch"]
            .as_str()
            .ok_or_else(|| anyhow::anyhow!("apply_patch: missing 'patch'"))?;

        validate_patch_paths(patch_str)?;

        // Write patch to a tempfile then apply with `patch -p1`
        let tmp = std::env::temp_dir().join(format!("cade-patch-{}.diff",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .subsec_nanos()
        ));
        std::fs::write(&tmp, patch_str)
            .with_context(|| "apply_patch: failed to write tempfile")?;

        let output = tokio::process::Command::new("patch")
            .args(["-p1", "--input", tmp.to_str().unwrap_or("")])
            .output()
            .await
            .with_context(|| "apply_patch: failed to run `patch` command (is it installed?)")?;

        let _ = std::fs::remove_file(&tmp);

        if output.status.success() {
            let stdout = String::from_utf8_lossy(&output.stdout);
            Ok(format!("Patch applied successfully.\n{stdout}").trim().to_string())
        } else {
            let stderr = String::from_utf8_lossy(&output.stderr);
            let stdout = String::from_utf8_lossy(&output.stdout);
            anyhow::bail!("patch failed:\n{stdout}{stderr}")
        }
    }

    pub fn schema() -> Value {
        serde_json::json!({
            "name": "apply_patch",
            "description": "Apply a unified diff patch to modify one or more files. \
Use this to edit files by providing a standard unified diff (output of `diff -u`). \
Supports multi-file patches and new file creation. Preferred for large or complex edits \
where string-replace would be fragile.",
            "parameters": {
                "type": "object",
                "properties": {
                    "patch": {
                        "type": "string",
                        "description": "Unified diff patch string. Format:\n--- a/path/to/file\n+++ b/path/to/file\n@@ -N,M +N,M @@\n context lines\n-removed line\n+added line"
                    }
                },
                "required": ["patch"]
            }
        })
    }
}
