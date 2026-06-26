use crate::Result;
use serde_json::{Value, json};
use std::path::{Path, PathBuf};

// -- P1-4: Filesystem sandbox default-on
//
// The sandbox is ACTIVE by default.  Every file-tool path is verified to
// resolve within the sandbox root.  The root is:
//
//   * `$CADE_FS_ROOT` — if set and non-empty after trim, canonicalized
//     (falls back to the raw value if the path does not exist yet).
//   * otherwise `std::env::current_dir()` captured once at first call.
//
// The only escape hatch is `CADE_FS_NO_SANDBOX=1` (exact match required
// so operators can't accidentally disable the sandbox with truthy-looking
// values like `0`, `true`, or empty).  When set, `fs_root()` returns
// `None` and all file-tool paths are accepted as before P1-4.

/// Policy function: pure, deterministic, unit-testable.  Takes the three
/// inputs (env `CADE_FS_ROOT`, env `CADE_FS_NO_SANDBOX`, current dir)
/// and returns the resolved sandbox root, or `None` if the sandbox is
/// explicitly disabled.
fn resolve_fs_root(
    env_root: Option<String>,
    no_sandbox: Option<String>,
    cwd: PathBuf,
) -> Option<PathBuf> {
    // Escape hatch: exact string "1" only.
    if matches!(no_sandbox.as_deref(), Some("1")) {
        return None;
    }

    if let Some(raw) = env_root {
        let trimmed = raw.trim();
        if !trimmed.is_empty() {
            return Some(std::fs::canonicalize(trimmed).unwrap_or_else(|_| PathBuf::from(trimmed)));
        }
    }

    Some(cwd)
}

/// Returns the filesystem sandbox root, or `None` when the sandbox is
/// explicitly disabled via `CADE_FS_NO_SANDBOX=1`.
///
/// The resolved root is cached in a process-global `OnceLock` so that
/// subsequent calls are cheap and the sandbox can't drift mid-process
/// (e.g. if cwd changes after a `cd`).
fn fs_root() -> Option<PathBuf> {
    use std::sync::OnceLock;
    static ROOT: OnceLock<Option<PathBuf>> = OnceLock::new();
    ROOT.get_or_init(|| {
        let cwd = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
        resolve_fs_root(
            std::env::var("CADE_FS_ROOT").ok(),
            std::env::var("CADE_FS_NO_SANDBOX").ok(),
            cwd,
        )
        .map(|p| std::fs::canonicalize(&p).unwrap_or(p))
    })
    .clone()
}

/// Verify that `raw_path` resolves to a location inside `root`.
/// For existing paths, follows symlinks via `canonicalize`.
/// For non-existing paths (e.g. write_file creating a new file), uses
/// lexical normalization to detect `..` escapes.
fn ensure_within_root(root: &Path, raw_path: &str) -> Result<()> {
    let p = Path::new(raw_path);
    let abs = if p.is_absolute() {
        p.to_path_buf()
    } else {
        root.join(p)
    };

    // Lexical normalization: resolve `.` and `..` components.
    let mut parts: Vec<std::path::Component> = Vec::new();
    for c in abs.components() {
        match c {
            std::path::Component::ParentDir => {
                if let Some(last) = parts.last() {
                    match last {
                        std::path::Component::Normal(_) => {
                            parts.pop();
                        }
                        _ => {
                            parts.push(std::path::Component::ParentDir);
                        }
                    }
                } else {
                    parts.push(std::path::Component::ParentDir);
                }
            }
            std::path::Component::CurDir => {}
            other => parts.push(other),
        }
    }
    let normalized: PathBuf = parts.iter().collect();

    // Now find the deepest existing ancestor of the normalized path
    let mut current = normalized.as_path();
    let mut non_existent = Vec::new();

    while !current.exists() {
        let Some(parent) = current.parent() else {
            break;
        };
        if let Some(name) = current.file_name() {
            non_existent.push(name.to_os_string());
        }
        current = parent;
    }

    let mut resolved = std::fs::canonicalize(current).unwrap_or_else(|_| current.to_path_buf());

    // Reconstruct the full path
    for comp in non_existent.into_iter().rev() {
        resolved.push(comp);
    }

    if !resolved.starts_with(root) {
        return Err(crate::Error::custom(format!(
            "path '{}' (resolved to '{}') is outside the allowed filesystem root '{}'",
            raw_path,
            resolved.display(),
            root.display()
        )));
    }
    Ok(())
}

// -- Read

pub struct ReadTool;

impl ReadTool {
    pub async fn run(args: &Value) -> Result<String> {
        let path = args["path"]
            .as_str()
            .ok_or_else(|| crate::Error::custom("read_file: missing 'path'".to_string()))?;
        if let Some(root) = &fs_root() {
            ensure_within_root(root, path)?;
        }
        let offset = args["offset"].as_u64().unwrap_or(0) as usize;
        let limit = args["limit"].as_u64().unwrap_or(0) as usize;

        let content = std::fs::read_to_string(path)
            .map_err(|e| crate::Error::custom(format!("read {path}: {e}")))?;

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
        json!({
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

// -- Write

pub struct WriteTool;

impl WriteTool {
    pub async fn run(args: &Value) -> Result<String> {
        let path = args["path"]
            .as_str()
            .ok_or_else(|| crate::Error::custom("write_file: missing 'path'".to_string()))?;
        if let Some(root) = &fs_root() {
            ensure_within_root(root, path)?;
        }
        let content = args["content"]
            .as_str()
            .ok_or_else(|| crate::Error::custom("write_file: missing 'content'".to_string()))?;

        // Acquire exclusive write lock for file path to prevent concurrent write clobbering (ADR 6)
        let _lock = crate::tools::file_lock::FileLockManager::global()
            .acquire_lock(Path::new(path))
            .await;

        if let Some(parent) = Path::new(path).parent()
            && !parent.as_os_str().is_empty()
        {
            std::fs::create_dir_all(parent)
                .map_err(|e| crate::Error::custom(format!("create dirs for {path}: {e}")))?;
        }

        std::fs::write(path, content)
            .map_err(|e| crate::Error::custom(format!("write {path}: {e}")))?;

        Ok(format!("Written {} bytes to {path}", content.len()))
    }

    pub fn schema() -> Value {
        json!({
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

// -- Edit (str-replace)

pub struct EditTool;

impl EditTool {
    pub async fn run(args: &Value) -> Result<String> {
        let path = args["path"]
            .as_str()
            .ok_or_else(|| crate::Error::custom("edit_file: missing 'path'".to_string()))?;
        if let Some(root) = &fs_root() {
            ensure_within_root(root, path)?;
        }
        let old_string = args["old_string"]
            .as_str()
            .ok_or_else(|| crate::Error::custom("edit_file: missing 'old_string'".to_string()))?;
        let new_string = args["new_string"]
            .as_str()
            .ok_or_else(|| crate::Error::custom("edit_file: missing 'new_string'".to_string()))?;
        let replace_all = args["replace_all"].as_bool().unwrap_or(false);

        // Acquire exclusive write lock for file path to prevent concurrent write clobbering (ADR 6)
        let _lock = crate::tools::file_lock::FileLockManager::global()
            .acquire_lock(Path::new(path))
            .await;

        let content = std::fs::read_to_string(path)
            .map_err(|e| crate::Error::custom(format!("read {path}: {e}")))?;

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
            .map_err(|e| crate::Error::custom(format!("write {path}: {e}")))?;

        Ok(format!("Replaced {count} occurrence(s) in {path}"))
    }

    pub fn schema() -> Value {
        json!({
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

// -- ApplyPatch
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
            return Err(crate::Error::custom(format!(
                "apply_patch: absolute paths are not allowed in patch: '{p}'"
            )));
        }
        if p.len() >= 3 {
            let bytes = p.as_bytes();
            if bytes[1] == b':' && (bytes[2] == b'/' || bytes[2] == b'\\') {
                return Err(crate::Error::custom(format!(
                    "apply_patch: absolute Windows-style paths are not allowed in patch: '{p}'"
                )));
            }
        }
        if p.split(&['/', '\\'][..]).any(|seg| seg == "..") {
            return Err(crate::Error::custom(format!(
                "apply_patch: parent-directory segments ('..') are not allowed in patch path: '{p}'"
            )));
        }
    }
    Ok(())
}

fn extract_patch_paths(patch_str: &str) -> Vec<PathBuf> {
    let mut paths = Vec::new();
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
        let clean_path = if p.starts_with("a/") || p.starts_with("b/") {
            &p[2..]
        } else {
            p
        };
        let pbuf = PathBuf::from(clean_path);
        if !paths.contains(&pbuf) {
            paths.push(pbuf);
        }
    }
    paths
}

impl ApplyPatchTool {
    pub async fn run(args: &Value) -> Result<String> {
        let patch_str = args["patch"]
            .as_str()
            .ok_or_else(|| crate::Error::custom("apply_patch: missing 'patch'".to_string()))?;

        validate_patch_paths(patch_str)?;

        // Acquire exclusive write locks on all patch target paths to prevent concurrent write clobbering (ADR 6)
        let patch_paths = extract_patch_paths(patch_str);
        let mut _locks = Vec::new();
        for path in &patch_paths {
            let lock = crate::tools::file_lock::FileLockManager::global()
                .acquire_lock(path)
                .await;
            _locks.push(lock);
        }

        // On Windows, the `patch` utility is not available natively.
        // Users need Git for Windows (which bundles patch) or WSL.
        #[cfg(windows)]
        {
            // Check if `patch` is reachable before attempting
            let probe = tokio::process::Command::new("patch")
                .arg("--version")
                .output()
                .await;
            if probe.is_err() || !probe.unwrap().status.success() {
                return Err(crate::Error::custom(
                    "apply_patch: the `patch` utility was not found. \
                     Install Git for Windows (includes `patch`) or use WSL."
                        .to_string(),
                ));
            }
        }

        // Write patch to a tempfile then apply with `patch -p1`
        use std::io::Write;
        let mut tmp_file = tempfile::NamedTempFile::new().map_err(|e| {
            crate::Error::custom(format!("apply_patch: failed to create tempfile: {e}"))
        })?;
        tmp_file.write_all(patch_str.as_bytes()).map_err(|e| {
            crate::Error::custom(format!("apply_patch: failed to write tempfile: {e}"))
        })?;
        tmp_file.flush()?;

        let mut cmd = tokio::process::Command::new("patch");
        cade_core::agent_env::apply_agent_env(&mut cmd);
        let output = cmd
            .args(["-p1", "--input", tmp_file.path().to_str().unwrap_or("")])
            .output()
            .await
            .map_err(|e| {
                crate::Error::custom(format!(
                    "apply_patch: failed to run `patch` command (is it installed?): {e}"
                ))
            })?;

        let _ = tmp_file.close();

        if output.status.success() {
            let stdout = String::from_utf8_lossy(&output.stdout);
            Ok(format!("Patch applied successfully.\n{stdout}")
                .trim()
                .to_string())
        } else {
            let stderr = String::from_utf8_lossy(&output.stderr);
            let stdout = String::from_utf8_lossy(&output.stdout);
            Err(crate::Error::custom(format!(
                "patch failed:\n{stdout}{stderr}"
            )))
        }
    }

    pub fn schema() -> Value {
        json!({
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

// region:    --- Tests

#[cfg(test)]
mod tests {
    #[allow(unused)]
    type Result<T> = core::result::Result<T, Box<dyn std::error::Error>>; // For tests.

    use super::*;

    // -- validate_patch_paths

    #[test]
    fn patch_paths_normal() {
        let patch = "\
--- a/src/lib.rs
+++ b/src/lib.rs
@@ -1 +1 @@
-old
+new
";
        assert!(validate_patch_paths(patch).is_ok());
    }

    #[test]
    fn patch_paths_dev_null() {
        let patch = "\
--- /dev/null
+++ b/src/new.rs
@@ -0,0 +1 @@
+hello
";
        assert!(validate_patch_paths(patch).is_ok());
    }

    #[test]
    fn patch_paths_rejects_parent_dir() {
        let patch = "\
--- a/../secret
+++ b/../secret
";
        assert!(validate_patch_paths(patch).is_err());
    }

    #[test]
    fn patch_paths_rejects_absolute() {
        let patch = "\
--- /etc/passwd
+++ /etc/passwd
";
        assert!(validate_patch_paths(patch).is_err());
    }

    #[test]
    fn patch_paths_rejects_windows_absolute() {
        let patch = "\
--- C:\\Windows\\system32\\hosts
+++ C:\\Windows\\system32\\hosts
";
        assert!(validate_patch_paths(patch).is_err());
    }

    // -- ensure_within_root

    #[test]
    fn within_root_relative_ok() -> Result<()> {
        // -- Exec & Check
        let root = std::env::current_dir()?;
        assert!(ensure_within_root(&root, "src/main.rs").is_ok());
        Ok(())
    }

    #[test]
    fn within_root_absolute_inside_ok() -> Result<()> {
        // -- Setup & Fixtures
        let root = std::env::current_dir()?;
        let abs = root.join("src/main.rs");

        // -- Check
        let path_str = abs.to_str().ok_or("Should be valid UTF-8")?;
        assert!(ensure_within_root(&root, path_str).is_ok());
        Ok(())
    }

    #[test]
    fn within_root_parent_escape() -> Result<()> {
        // -- Exec & Check
        let root = std::env::current_dir()?;
        assert!(ensure_within_root(&root, "../../../etc/passwd").is_err());
        Ok(())
    }

    #[test]
    fn within_root_absolute_outside() -> Result<()> {
        // -- Exec & Check
        let root = std::env::current_dir()?;
        assert!(ensure_within_root(&root, "/etc/passwd").is_err());
        Ok(())
    }

    // -- P1-4: resolve_fs_root (default-on policy)

    #[test]
    fn p1_4_no_env_defaults_to_cwd() {
        let cwd = PathBuf::from("/tmp/fake-cwd");
        let got = resolve_fs_root(None, None, cwd.clone());
        assert_eq!(got, Some(cwd));
    }

    #[test]
    fn p1_4_no_sandbox_env_disables_sandbox() {
        let cwd = PathBuf::from("/tmp/fake-cwd");
        let got = resolve_fs_root(None, Some("1".into()), cwd);
        assert_eq!(got, None);
    }

    #[test]
    fn p1_4_no_sandbox_env_zero_does_not_disable() {
        // Only the exact string "1" opts out; other values (0, true, "")
        // are NOT escape hatches so operators can't accidentally disable
        // the sandbox with a truthy-looking value.
        let cwd = PathBuf::from("/tmp/fake-cwd");
        assert_eq!(
            resolve_fs_root(None, Some("0".into()), cwd.clone()),
            Some(cwd.clone())
        );
        assert_eq!(
            resolve_fs_root(None, Some("".into()), cwd.clone()),
            Some(cwd.clone())
        );
        assert_eq!(
            resolve_fs_root(None, Some("true".into()), cwd.clone()),
            Some(cwd)
        );
    }

    #[test]
    fn p1_4_explicit_root_overrides_cwd() {
        let cwd = PathBuf::from("/tmp/should-not-be-used");
        let explicit = std::env::current_dir().unwrap(); // pick a real dir for canonicalize
        let got = resolve_fs_root(Some(explicit.display().to_string()), None, cwd);
        assert!(got.is_some());
        let got = got.unwrap();
        // Canonicalized form must start with the same real path.
        assert!(
            got == explicit || explicit.canonicalize().map(|c| c == got).unwrap_or(false),
            "expected {got:?} to equal {explicit:?} or its canonical form"
        );
    }

    #[test]
    fn p1_4_explicit_empty_root_falls_back_to_cwd() {
        let cwd = PathBuf::from("/tmp/fake-cwd");
        let got = resolve_fs_root(Some("   ".into()), None, cwd.clone());
        assert_eq!(
            got,
            Some(cwd),
            "whitespace-only CADE_FS_ROOT must fall back to cwd, not disable sandbox"
        );
    }

    #[test]
    fn p1_4_no_sandbox_wins_over_explicit_root() {
        let cwd = PathBuf::from("/tmp/fake-cwd");
        let got = resolve_fs_root(Some("/some/root".into()), Some("1".into()), cwd);
        assert_eq!(
            got, None,
            "CADE_FS_NO_SANDBOX=1 must disable the sandbox even when CADE_FS_ROOT is set"
        );
    }
}

// endregion: --- Tests
