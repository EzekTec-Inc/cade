use anyhow::Result;
use globset::{Glob, GlobSetBuilder};
use regex::Regex;
use serde_json::Value;
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
        serde_json::json!({
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
        serde_json::json!({
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
