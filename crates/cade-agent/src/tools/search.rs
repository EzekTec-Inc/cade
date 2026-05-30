use crate::Result;
use globset::{Glob, GlobSetBuilder};
use ignore::{WalkBuilder, WalkState};
use regex::Regex;
use serde_json::{Value, json};
use std::path::Path;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

// -- Grep

pub struct GrepTool;

impl GrepTool {
    pub async fn run(args: &Value) -> Result<String> {
        let pattern = args["pattern"]
            .as_str()
            .ok_or_else(|| crate::Error::custom("grep: missing 'pattern'"))?;
        let search_path = args["path"].as_str().unwrap_or(".");
        let include = args["include"].as_str().unwrap_or("");
        let case_insensitive = args["case_insensitive"].as_bool().unwrap_or(false);
        let context_lines = args["context"].as_u64().unwrap_or(0) as usize;

        let re = if case_insensitive {
            Regex::new(&format!("(?i){pattern}"))
        } else {
            Regex::new(pattern)
        }
        .map_err(|e| crate::Error::custom(format!("Invalid regex '{pattern}': {e}")))?;

        // Build extension filter from include param (e.g. "*.rs,*.toml")
        let ext_filter: Vec<String> = if include.is_empty() {
            vec![]
        } else {
            include.split(',').map(|s| s.trim().to_string()).collect()
        };

        let root = search_path.to_string();

        let result = tokio::task::spawn_blocking(move || {
            let (tx, rx) = std::sync::mpsc::channel();
            let total_files = Arc::new(AtomicUsize::new(0));
            let re = Arc::new(re);
            let ext_filter = Arc::new(ext_filter);

            WalkBuilder::new(&root)
                .hidden(false)
                .filter_entry(|e| e.file_name() != ".git")
                .build_parallel()
                .run(|| {
                    let tx = tx.clone();
                    let total_files = Arc::clone(&total_files);
                    let re = Arc::clone(&re);
                    let ext_filter = Arc::clone(&ext_filter);
                    Box::new(move |result| {
                        if let Ok(entry) = result
                            && entry.file_type().is_some_and(|ft| ft.is_file())
                        {
                            let path = entry.path();

                            // Extension filter
                            if !ext_filter.is_empty() {
                                let fname = path.file_name().and_then(|n| n.to_str()).unwrap_or("");
                                let matches_filter = ext_filter.iter().any(|pat| {
                                    if let Some(stripped) = pat.strip_prefix('*') {
                                        fname.ends_with(stripped)
                                    } else {
                                        fname == *pat
                                    }
                                });
                                if !matches_filter {
                                    return WalkState::Continue;
                                }
                            }

                            total_files.fetch_add(1, Ordering::Relaxed);

                            if let Ok(content) = std::fs::read_to_string(path) {
                                let lines: Vec<&str> = content.lines().collect();
                                let mut local_matches = Vec::new();
                                for (i, line) in lines.iter().enumerate() {
                                    if re.is_match(line) {
                                        let path_str = path.display().to_string();
                                        if context_lines > 0 {
                                            let start = i.saturating_sub(context_lines);
                                            let end = (i + context_lines + 1).min(lines.len());
                                            for (j, ctx_line) in
                                                lines[start..end].iter().enumerate()
                                            {
                                                let lineno = start + j + 1;
                                                let sep = if start + j == i { ':' } else { '-' };
                                                local_matches.push(format!(
                                                    "{path_str}{sep}{lineno}{sep}{ctx_line}"
                                                ));
                                            }
                                            local_matches.push("--".to_string());
                                        } else {
                                            local_matches.push(format!(
                                                "{}:{}:{}",
                                                path_str,
                                                i + 1,
                                                line
                                            ));
                                        }
                                    }
                                }
                                if !local_matches.is_empty() && tx.send(local_matches).is_err() {
                                    return WalkState::Quit;
                                }
                            }
                        }
                        WalkState::Continue
                    })
                });

            drop(tx);
            let mut matches: Vec<String> = rx.into_iter().flatten().collect();
            let total = total_files.load(Ordering::Relaxed);

            if matches.len() >= 500 {
                matches.truncate(500);
                matches.push(format!("... (truncated, searched {total} files)"));
            }

            (matches, total)
        })
        .await
        .map_err(|e| crate::Error::custom(format!("Task error: {e}")))?;

        let (matches, total_files) = result;

        if matches.is_empty() {
            Ok(format!(
                "No matches for '{pattern}' (searched {total_files} files)"
            ))
        } else {
            Ok(matches.join("\n"))
        }
    }

    pub fn schema() -> Value {
        json!({
            "name": "grep",
            "description": "Search file contents using a regex pattern. Returns matching lines with file:line:content format. Skips .git/ and files listed in .gitignore.",
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

    // -- GrepTool

    #[tokio::test]
    async fn grep_finds_pattern_in_files() -> Result<()> {
        let dir = tempfile::tempdir()?;
        let file = dir.path().join("test.rs");
        fs::write(&file, "fn main() {\n    println!(\"hello\");\n}\n")?;

        let args = json!({
            "pattern": "fn main",
            "path": dir.path().to_str().ok_or("Should be valid UTF-8")?
        });
        let output = GrepTool::run(&args).await?;
        assert!(output.contains("fn main"), "got: {output}");
        assert!(
            output.contains(":1:"),
            "should show line number, got: {output}"
        );

        Ok(())
    }

    #[tokio::test]
    async fn grep_no_matches() -> Result<()> {
        let dir = tempfile::tempdir()?;
        let file = dir.path().join("test.txt");
        fs::write(&file, "nothing interesting here")?;

        let args = json!({
            "pattern": "nonexistent_pattern_xyz",
            "path": dir.path().to_str().ok_or("Should be valid UTF-8")?
        });
        let output = GrepTool::run(&args).await?;
        assert!(output.contains("No matches"), "got: {output}");

        Ok(())
    }

    #[tokio::test]
    async fn grep_case_insensitive() -> Result<()> {
        let dir = tempfile::tempdir()?;
        let file = dir.path().join("test.txt");
        fs::write(&file, "Hello World")?;

        let args = json!({
            "pattern": "hello",
            "path": dir.path().to_str().ok_or("Should be valid UTF-8")?,
            "case_insensitive": true
        });
        let output = GrepTool::run(&args).await?;
        assert!(output.contains("Hello World"), "got: {output}");

        Ok(())
    }

    #[tokio::test]
    async fn grep_with_include_filter() -> Result<()> {
        let dir = tempfile::tempdir()?;
        fs::write(dir.path().join("match.rs"), "fn test()")?;
        fs::write(dir.path().join("skip.txt"), "fn test()")?;

        let args = json!({
            "pattern": "fn test",
            "path": dir.path().to_str().ok_or("Should be valid UTF-8")?,
            "include": "*.rs"
        });
        let output = GrepTool::run(&args).await?;
        assert!(output.contains("match.rs"), "got: {output}");
        assert!(
            !output.contains("skip.txt"),
            "should not include .txt files, got: {output}"
        );

        Ok(())
    }

    #[tokio::test]
    async fn grep_with_context_lines() -> Result<()> {
        let dir = tempfile::tempdir()?;
        let file = dir.path().join("test.rs");
        fs::write(&file, "line1\nline2\ntarget\nline4\nline5\n")?;

        let args = json!({
            "pattern": "target",
            "path": dir.path().to_str().ok_or("Should be valid UTF-8")?,
            "context": 1
        });
        let output = GrepTool::run(&args).await?;
        assert!(
            output.contains("line2"),
            "should have context before, got: {output}"
        );
        assert!(
            output.contains("line4"),
            "should have context after, got: {output}"
        );

        Ok(())
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
    async fn grep_skips_git_dir() -> Result<()> {
        let dir = tempfile::tempdir()?;
        let target_dir = dir.path().join(".git");
        fs::create_dir_all(&target_dir)?;
        fs::write(target_dir.join("build_output.txt"), "fn main")?;
        fs::write(dir.path().join("src.rs"), "fn main")?;

        let args = json!({
            "pattern": "fn main",
            "path": dir.path().to_str().ok_or("Should be valid UTF-8")?
        });
        let output = GrepTool::run(&args).await?;
        assert!(
            output.contains("src.rs"),
            "should find src.rs, got: {output}"
        );
        assert!(
            !output.contains(".git/"),
            "should skip .git/, got: {output}"
        );

        Ok(())
    }

    // -- GrepTool::schema

    #[test]
    fn grep_schema_valid() -> Result<()> {
        let schema = GrepTool::schema();
        assert_eq!(schema["name"], "grep");
        assert!(
            schema["description"]
                .as_str()
                .ok_or("Should be a string")?
                .len()
                > 10
        );
        assert!(schema["parameters"]["properties"]["pattern"].is_object());

        Ok(())
    }
}

// endregion: --- Tests

// -- Glob

pub struct GlobTool;

impl GlobTool {
    pub async fn run(args: &Value) -> Result<String> {
        let pattern = args["pattern"]
            .as_str()
            .ok_or_else(|| crate::Error::custom("glob: missing 'pattern'"))?;
        let search_path = args["path"].as_str().unwrap_or(".");
        let limit = args["limit"].as_u64().unwrap_or(500) as usize;

        let glob = Glob::new(pattern)
            .map_err(|e| crate::Error::custom(format!("Invalid glob '{pattern}': {e}")))?;
        let mut builder = GlobSetBuilder::new();
        builder.add(glob);
        let globset = builder.build().map_err(crate::Error::custom_from_err)?;

        let root = search_path.to_string();

        let matches = tokio::task::spawn_blocking(move || {
            let (tx, rx) = std::sync::mpsc::channel();

            WalkBuilder::new(&root)
                .hidden(false)
                .filter_entry(|e| e.file_name() != ".git")
                .build_parallel()
                .run(|| {
                    let tx = tx.clone();
                    let root = Path::new(&root).to_path_buf();
                    let globset = globset.clone();
                    Box::new(move |result| {
                        if let Ok(entry) = result
                            && entry.file_type().is_some_and(|ft| ft.is_file())
                        {
                            let path = entry.path();
                            let rel = path.strip_prefix(&root).unwrap_or(path);
                            if globset.is_match(rel)
                                || globset.is_match(path.file_name().unwrap_or_default())
                            {
                                let mtime = entry
                                    .metadata()
                                    .ok()
                                    .and_then(|m| m.modified().ok())
                                    .unwrap_or(std::time::SystemTime::UNIX_EPOCH);
                                let _ = tx.send((mtime, path.display().to_string()));
                            }
                        }
                        WalkState::Continue
                    })
                });

            drop(tx);
            let mut matches: Vec<(std::time::SystemTime, String)> = rx.into_iter().collect();
            matches.sort_by_key(|b| std::cmp::Reverse(b.0));
            matches
        })
        .await
        .map_err(|e| crate::Error::custom(format!("Task error: {e}")))?;

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
            "description": "Find files matching a glob pattern (e.g. '**/*.rs'). Returns paths sorted by modification time (newest first). Skips .git/ and files listed in .gitignore.",
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
    async fn glob_finds_files() -> Result<()> {
        let dir = tempfile::tempdir()?;
        fs::write(dir.path().join("file1.rs"), "")?;
        fs::write(dir.path().join("file2.rs"), "")?;
        fs::write(dir.path().join("file3.txt"), "")?;

        let args = json!({
            "pattern": "*.rs",
            "path": dir.path().to_str().ok_or("Should be valid UTF-8")?
        });
        let output = GlobTool::run(&args).await?;
        assert!(output.contains("file1.rs"), "got: {output}");
        assert!(output.contains("file2.rs"), "got: {output}");
        assert!(
            !output.contains("file3.txt"),
            "should not include .txt, got: {output}"
        );

        Ok(())
    }

    #[tokio::test]
    async fn glob_no_matches() -> Result<()> {
        let dir = tempfile::tempdir()?;
        fs::write(dir.path().join("file.txt"), "")?;

        let args = json!({
            "pattern": "*.xyz",
            "path": dir.path().to_str().ok_or("Should be valid UTF-8")?
        });
        let output = GlobTool::run(&args).await?;
        assert!(output.contains("No files"), "got: {output}");

        Ok(())
    }

    #[tokio::test]
    async fn glob_recursive_pattern() -> Result<()> {
        let dir = tempfile::tempdir()?;
        let sub = dir.path().join("sub");
        fs::create_dir_all(&sub)?;
        fs::write(sub.join("nested.rs"), "")?;

        let args = json!({
            "pattern": "**/*.rs",
            "path": dir.path().to_str().ok_or("Should be valid UTF-8")?
        });
        let output = GlobTool::run(&args).await?;
        assert!(output.contains("nested.rs"), "got: {output}");

        Ok(())
    }

    #[tokio::test]
    async fn glob_respects_limit() -> Result<()> {
        let dir = tempfile::tempdir()?;
        for i in 0..10 {
            fs::write(dir.path().join(format!("file{i}.txt")), "")?;
        }

        let args = json!({
            "pattern": "*.txt",
            "path": dir.path().to_str().ok_or("Should be valid UTF-8")?,
            "limit": 3
        });
        let output = GlobTool::run(&args).await?;
        let lines: Vec<&str> = output.lines().collect();
        assert_eq!(lines.len(), 3, "expected 3 results, got: {output}");

        Ok(())
    }

    #[tokio::test]
    async fn glob_skips_git_dir() -> Result<()> {
        let dir = tempfile::tempdir()?;
        let nm = dir.path().join(".git");
        fs::create_dir_all(&nm)?;
        fs::write(nm.join("dep.js"), "")?;
        fs::write(dir.path().join("app.js"), "")?;

        let args = json!({
            "pattern": "**/*.js",
            "path": dir.path().to_str().ok_or("Should be valid UTF-8")?
        });
        let output = GlobTool::run(&args).await?;
        assert!(output.contains("app.js"), "got: {output}");
        assert!(
            !output.contains(".git/"),
            "should skip .git/, got: {output}"
        );

        Ok(())
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
