use anyhow::Result;
use globset::{Glob, GlobSetBuilder};
use regex::Regex;
use serde_json::{Value, json};
use std::path::Path;
use walkdir::WalkDir;

// ── Skip dirs common in Rust/JS/Python projects ───────────────────────────────
const SKIP_DIRS: &[&str] = &["target", "node_modules", ".git", ".hg", "__pycache__", ".venv", "dist", "build"];

fn should_skip(name: &str) -> bool {
    SKIP_DIRS.contains(&name)
}

// ── Grep ─────────────────────────────────────────────────────────────────────

pub struct GrepTool;

impl GrepTool {
    pub async fn run(args: &Value) -> Result<String> {
        let pattern = args["pattern"]
            .as_str()
            .ok_or_else(|| anyhow::anyhow!("grep: missing 'pattern'"))?;
        let search_path = args["path"].as_str().unwrap_or(".");
        let include = args["include"].as_str().unwrap_or("");
        let case_insensitive = args["case_insensitive"].as_bool().unwrap_or(false);
        let context_lines = args["context"].as_u64().unwrap_or(0) as usize;

        let re = if case_insensitive {
            Regex::new(&format!("(?i){pattern}"))
        } else {
            Regex::new(pattern)
        }
        .map_err(|e| anyhow::anyhow!("Invalid regex '{pattern}': {e}"))?;

        // Build extension filter from include param (e.g. "*.rs,*.toml")
        let ext_filter: Vec<&str> = if include.is_empty() {
            vec![]
        } else {
            include.split(',').map(str::trim).collect()
        };

        let root = Path::new(search_path);
        let mut matches: Vec<String> = Vec::new();
        let mut total_files = 0usize;

        for entry in WalkDir::new(root)
            .into_iter()
            .filter_entry(|e| {
                let name = e.file_name().to_str().unwrap_or("");
                !should_skip(name)
            })
            .filter_map(|e| e.ok())
            .filter(|e| e.file_type().is_file())
        {
            let path = entry.path();

            // Extension filter
            if !ext_filter.is_empty() {
                let fname = path.file_name().and_then(|n| n.to_str()).unwrap_or("");
                let matches_filter = ext_filter.iter().any(|pat| {
                    if pat.starts_with('*') {
                        fname.ends_with(&pat[1..])
                    } else {
                        fname == *pat
                    }
                });
                if !matches_filter {
                    continue;
                }
            }

            total_files += 1;
            let content = match std::fs::read_to_string(path) {
                Ok(c) => c,
                Err(_) => continue, // skip binary / unreadable
            };

            let lines: Vec<&str> = content.lines().collect();
            for (i, line) in lines.iter().enumerate() {
                if re.is_match(line) {
                    let path_str = path.display().to_string();

                    if context_lines > 0 {
                        let start = i.saturating_sub(context_lines);
                        let end = (i + context_lines + 1).min(lines.len());
                        for (j, ctx_line) in lines[start..end].iter().enumerate() {
                            let lineno = start + j + 1;
                            let sep = if start + j == i { ':' } else { '-' };
                            matches.push(format!("{path_str}{sep}{lineno}{sep}{ctx_line}"));
                        }
                        matches.push("--".to_string());
                    } else {
                        matches.push(format!("{}:{}:{}", path_str, i + 1, line));
                    }

                    if matches.len() >= 500 {
                        matches.push(format!("... (truncated, searched {total_files} files)"));
                        return Ok(matches.join("\n"));
                    }
                }
            }
        }

        if matches.is_empty() {
            Ok(format!("No matches for '{pattern}' (searched {total_files} files)"))
        } else {
            Ok(matches.join("\n"))
        }
    }

    pub fn schema() -> Value {
        json!({
            "name": "grep",
            "description": "Search file contents using a regex pattern. Returns matching lines with file:line:content format. Skips target/, node_modules/, .git/.",
            "parameters": {
                "type": "object",
                "properties": {
                    "pattern":          { "type": "string",  "description": "Regex pattern to search for" },
                    "path":             { "type": "string",  "description": "Directory or file to search (default '.')" },
                    "include":          { "type": "string",  "description": "Comma-separated extensions to include (e.g. '*.rs,*.toml')" },
                    "case_insensitive": { "type": "boolean", "description": "Case-insensitive match (default false)" },
                    "context":          { "type": "integer", "description": "Lines of context around each match (default 0)" }
                },
                "required": ["pattern"]
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
    use std::fs;

    // ── GrepTool ──────────────────────────────────────────────────────────

    #[tokio::test]
    async fn grep_finds_pattern_in_files() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("test.rs");
        fs::write(&file, "fn main() {\n    println!(\"hello\");\n}\n").unwrap();

        let args = json!({
            "pattern": "fn main",
            "path": dir.path().to_str().unwrap()
        });
        let output = GrepTool::run(&args).await.unwrap();
        assert!(output.contains("fn main"), "got: {output}");
        assert!(output.contains(":1:"), "should show line number, got: {output}");
    }

    #[tokio::test]
    async fn grep_no_matches() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("test.txt");
        fs::write(&file, "nothing interesting here").unwrap();

        let args = json!({
            "pattern": "nonexistent_pattern_xyz",
            "path": dir.path().to_str().unwrap()
        });
        let output = GrepTool::run(&args).await.unwrap();
        assert!(output.contains("No matches"), "got: {output}");
    }

    #[tokio::test]
    async fn grep_case_insensitive() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("test.txt");
        fs::write(&file, "Hello World").unwrap();

        let args = json!({
            "pattern": "hello",
            "path": dir.path().to_str().unwrap(),
            "case_insensitive": true
        });
        let output = GrepTool::run(&args).await.unwrap();
        assert!(output.contains("Hello World"), "got: {output}");
    }

    #[tokio::test]
    async fn grep_with_include_filter() {
        let dir = tempfile::tempdir().unwrap();
        fs::write(dir.path().join("match.rs"), "fn test()").unwrap();
        fs::write(dir.path().join("skip.txt"), "fn test()").unwrap();

        let args = json!({
            "pattern": "fn test",
            "path": dir.path().to_str().unwrap(),
            "include": "*.rs"
        });
        let output = GrepTool::run(&args).await.unwrap();
        assert!(output.contains("match.rs"), "got: {output}");
        assert!(!output.contains("skip.txt"), "should not include .txt files, got: {output}");
    }

    #[tokio::test]
    async fn grep_with_context_lines() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("test.rs");
        fs::write(&file, "line1\nline2\ntarget\nline4\nline5\n").unwrap();

        let args = json!({
            "pattern": "target",
            "path": dir.path().to_str().unwrap(),
            "context": 1
        });
        let output = GrepTool::run(&args).await.unwrap();
        assert!(output.contains("line2"), "should have context before, got: {output}");
        assert!(output.contains("line4"), "should have context after, got: {output}");
    }

    #[tokio::test]
    async fn grep_invalid_regex() {
        let args = json!({
            "pattern": "[invalid",
            "path": "."
        });
        let result = GrepTool::run(&args).await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn grep_skips_target_dir() {
        let dir = tempfile::tempdir().unwrap();
        let target_dir = dir.path().join("target");
        fs::create_dir_all(&target_dir).unwrap();
        fs::write(target_dir.join("build_output.txt"), "fn main").unwrap();
        fs::write(dir.path().join("src.rs"), "fn main").unwrap();

        let args = json!({
            "pattern": "fn main",
            "path": dir.path().to_str().unwrap()
        });
        let output = GrepTool::run(&args).await.unwrap();
        assert!(output.contains("src.rs"), "should find src.rs, got: {output}");
        assert!(!output.contains("target/"), "should skip target/, got: {output}");
    }

    // ── GrepTool::schema ──────────────────────────────────────────────────

    #[test]
    fn grep_schema_valid() {
        let schema = GrepTool::schema();
        assert_eq!(schema["name"], "grep");
        assert!(schema["description"].as_str().unwrap().len() > 10);
        assert!(schema["parameters"]["properties"]["pattern"].is_object());
    }
}

// endregion: --- Tests

// ── Glob ─────────────────────────────────────────────────────────────────────

pub struct GlobTool;

impl GlobTool {
    pub async fn run(args: &Value) -> Result<String> {
        let pattern = args["pattern"]
            .as_str()
            .ok_or_else(|| anyhow::anyhow!("glob: missing 'pattern'"))?;
        let search_path = args["path"].as_str().unwrap_or(".");
        let limit = args["limit"].as_u64().unwrap_or(500) as usize;

        let glob = Glob::new(pattern)
            .map_err(|e| anyhow::anyhow!("Invalid glob '{pattern}': {e}"))?;
        let mut builder = GlobSetBuilder::new();
        builder.add(glob);
        let globset = builder.build()?;

        let root = Path::new(search_path);
        let mut matches: Vec<(std::time::SystemTime, String)> = Vec::new();

        for entry in WalkDir::new(root)
            .into_iter()
            .filter_entry(|e| {
                let name = e.file_name().to_str().unwrap_or("");
                !should_skip(name)
            })
            .filter_map(|e| e.ok())
            .filter(|e| e.file_type().is_file())
        {
            let path = entry.path();
            // Match against the path relative to root OR just the filename
            let rel = path.strip_prefix(root).unwrap_or(path);
            if globset.is_match(rel) || globset.is_match(path.file_name().unwrap_or_default()) {
                let mtime = entry
                    .metadata()
                    .ok()
                    .and_then(|m| m.modified().ok())
                    .unwrap_or(std::time::SystemTime::UNIX_EPOCH);
                matches.push((mtime, path.display().to_string()));
            }
        }

        // Sort by modification time, newest first
        matches.sort_by(|a, b| b.0.cmp(&a.0));

        let result: Vec<String> = matches.into_iter().map(|(_, p)| p).take(limit).collect();

        if result.is_empty() {
            Ok(format!("No files matching '{pattern}'"))
        } else {
            Ok(result.join("\n"))
        }
    }

    pub fn schema() -> Value {
        json!({
            "name": "glob",
            "description": "Find files matching a glob pattern (e.g. '**/*.rs'). Returns paths sorted by modification time (newest first). Skips target/, node_modules/, .git/.",
            "parameters": {
                "type": "object",
                "properties": {
                    "pattern": { "type": "string",  "description": "Glob pattern (e.g. '**/*.rs', '*.toml', 'src/**/*.ts')" },
                    "path":    { "type": "string",  "description": "Base directory to search (default '.')" },
                    "limit":   { "type": "integer", "description": "Max results to return (default 500)" }
                },
                "required": ["pattern"]
            }
        })
    }
}

// region:    --- Tests (glob)

#[cfg(test)]
mod glob_tests {
    #[allow(unused)]
    type Result<T> = core::result::Result<T, Box<dyn std::error::Error>>; // For tests.

    use super::*;
    use std::fs;

    #[tokio::test]
    async fn glob_finds_files() {
        let dir = tempfile::tempdir().unwrap();
        fs::write(dir.path().join("file1.rs"), "").unwrap();
        fs::write(dir.path().join("file2.rs"), "").unwrap();
        fs::write(dir.path().join("file3.txt"), "").unwrap();

        let args = json!({
            "pattern": "*.rs",
            "path": dir.path().to_str().unwrap()
        });
        let output = GlobTool::run(&args).await.unwrap();
        assert!(output.contains("file1.rs"), "got: {output}");
        assert!(output.contains("file2.rs"), "got: {output}");
        assert!(!output.contains("file3.txt"), "should not include .txt, got: {output}");
    }

    #[tokio::test]
    async fn glob_no_matches() {
        let dir = tempfile::tempdir().unwrap();
        fs::write(dir.path().join("file.txt"), "").unwrap();

        let args = json!({
            "pattern": "*.xyz",
            "path": dir.path().to_str().unwrap()
        });
        let output = GlobTool::run(&args).await.unwrap();
        assert!(output.contains("No files"), "got: {output}");
    }

    #[tokio::test]
    async fn glob_recursive_pattern() {
        let dir = tempfile::tempdir().unwrap();
        let sub = dir.path().join("sub");
        fs::create_dir_all(&sub).unwrap();
        fs::write(sub.join("nested.rs"), "").unwrap();

        let args = json!({
            "pattern": "**/*.rs",
            "path": dir.path().to_str().unwrap()
        });
        let output = GlobTool::run(&args).await.unwrap();
        assert!(output.contains("nested.rs"), "got: {output}");
    }

    #[tokio::test]
    async fn glob_respects_limit() {
        let dir = tempfile::tempdir().unwrap();
        for i in 0..10 {
            fs::write(dir.path().join(format!("file{i}.txt")), "").unwrap();
        }

        let args = json!({
            "pattern": "*.txt",
            "path": dir.path().to_str().unwrap(),
            "limit": 3
        });
        let output = GlobTool::run(&args).await.unwrap();
        let lines: Vec<&str> = output.lines().collect();
        assert_eq!(lines.len(), 3, "expected 3 results, got: {output}");
    }

    #[tokio::test]
    async fn glob_skips_node_modules() {
        let dir = tempfile::tempdir().unwrap();
        let nm = dir.path().join("node_modules");
        fs::create_dir_all(&nm).unwrap();
        fs::write(nm.join("dep.js"), "").unwrap();
        fs::write(dir.path().join("app.js"), "").unwrap();

        let args = json!({
            "pattern": "**/*.js",
            "path": dir.path().to_str().unwrap()
        });
        let output = GlobTool::run(&args).await.unwrap();
        assert!(output.contains("app.js"), "got: {output}");
        assert!(!output.contains("node_modules"), "should skip node_modules, got: {output}");
    }

    #[tokio::test]
    async fn glob_invalid_pattern() {
        let args = json!({"pattern": "[invalid"});
        let result = GlobTool::run(&args).await;
        assert!(result.is_err());
    }

    #[test]
    fn glob_schema_valid() {
        let schema = GlobTool::schema();
        assert_eq!(schema["name"], "glob");
        assert!(schema["parameters"]["properties"]["pattern"].is_object());
    }
}

// endregion: --- Tests (glob)
