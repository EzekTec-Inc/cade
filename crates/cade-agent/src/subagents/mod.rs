// region:    --- Modules

use anyhow::Result;
use std::path::{Path, PathBuf};

// endregion: --- Modules

// -- Tool access level

#[derive(Debug, Clone)]
pub enum SubagentTools {
    /// All registered CADE tools
    All,
    /// Read-only: bash (read-only commands only), read, glob, grep
    Readonly,
    /// Explicit list of tool names
    List(Vec<String>),
}

impl std::fmt::Display for SubagentTools {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::All      => write!(f, "all"),
            Self::Readonly => write!(f, "readonly"),
            Self::List(v)  => write!(f, "{}", v.join(", ")),
        }
    }
}

impl SubagentTools {
    fn from_str(s: &str) -> Self {
        match s.trim().to_lowercase().as_str() {
            "all"      => Self::All,
            "readonly" | "read-only" | "read_only" => Self::Readonly,
            other      => Self::List(
                other.split(',').map(|t| t.trim().to_string()).filter(|t| !t.is_empty()).collect()
            ),
        }
    }
}

// -- Scope

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum SubagentScope {
    Builtin  = 0,
    Global   = 1,
    Project  = 2,
}

impl std::fmt::Display for SubagentScope {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Builtin => write!(f, "builtin"),
            Self::Global  => write!(f, "global"),
            Self::Project => write!(f, "project"),
        }
    }
}

// -- Subagent definition

#[derive(Debug, Clone)]
pub struct SubagentDef {
    pub name:          String,
    pub description:   String,
    /// None = inherit the main agent's current model
    pub model:         Option<String>,
    pub tools:         SubagentTools,
    pub system_prompt: String,
    pub skills:        Vec<String>,
    pub scope:         SubagentScope,
    /// Path to the defining .md file (None for built-ins)
    pub path:          Option<PathBuf>,
}

impl SubagentDef {
    /// One-line summary for /subagents list
    pub fn summary(&self) -> String {
        format!(
            "  [{:<8}] {:<22} — {} ({})",
            self.scope.to_string(),
            self.name,
            self.description,
            self.tools,
        )
    }
}

// -- Built-ins

pub fn builtin_subagents() -> Vec<SubagentDef> {
    vec![
        SubagentDef {
            name:        "explore".to_string(),
            description: "Fast read-only codebase search — find files, search patterns, understand structure".to_string(),
            model:       None, // inherit (works well with any model)
            tools:       SubagentTools::Readonly,
            system_prompt: "\
You are an expert code explorer. Your job is to search the codebase efficiently and return \
precise, concise answers. Do NOT make any modifications to files. \
Focus on finding exactly what was asked and report back clearly with file paths, line numbers, \
and relevant code snippets. Be thorough but succinct.".to_string(),
            skills:      vec![],
            scope:       SubagentScope::Builtin,
            path:        None,
        },
        SubagentDef {
            name:        "general-purpose".to_string(),
            description: "Full-capability agent — research, plan, and implement changes".to_string(),
            model:       None,
            tools:       SubagentTools::List(vec!["bash".to_string(), "read_file".to_string(), "write_file".to_string(), "edit_file".to_string(), "glob".to_string(), "grep_search".to_string()]),
            system_prompt: "\
You are a general-purpose coding assistant. Complete the assigned task thoroughly, \
making all necessary file changes. Report back with a clear summary of what you did, \
what files were changed, and any important decisions made.".to_string(),
            skills:      vec![],
            scope:       SubagentScope::Builtin,
            path:        None,
        },
        SubagentDef {
            name:        "coder".to_string(),
            description: "Focused code implementation — edits files, runs tests".to_string(),
            model:       None,
            tools:       SubagentTools::List(vec!["bash".to_string(), "read_file".to_string(), "write_file".to_string(), "edit_file".to_string(), "glob".to_string(), "grep_search".to_string()]),
            system_prompt: "\
You are a focused coding agent. Implement the requested changes cleanly and correctly. \
Follow the existing code style and patterns. Run tests if available. \
Report what you changed and whether tests pass.".to_string(),
            skills:      vec![],
            scope:       SubagentScope::Builtin,
            path:        None,
        },
        SubagentDef {
            name:        "reviewer".to_string(),
            description: "Code review — read-only analysis of quality, bugs, security".to_string(),
            model:       None,
            tools:       SubagentTools::Readonly,
            system_prompt: "\
You are an expert code reviewer. Analyse the specified code for: bugs, security issues, \
performance problems, style inconsistencies, and missing error handling. \
Be specific — include file paths and line numbers. Prioritise findings by severity.".to_string(),
            skills:      vec![],
            scope:       SubagentScope::Builtin,
            path:        None,
        },
        SubagentDef {
            name:        "reflection".to_string(),
            description: "Background agent — reflects on the conversation and updates memory blocks".to_string(),
            model:       None,
            tools:       SubagentTools::List(vec![
                "update_memory".to_string(),
                "read_file".to_string(),
                "glob".to_string(),
            ]),
            system_prompt: "\
You are a background memory-maintenance agent. Your sole job is to reflect on the recent \
conversation summary provided and update the agent's memory blocks to capture:\n\
1. New facts learned about the project, user preferences, or codebase structure.\n\
2. Corrections to outdated information.\n\
3. Important decisions made during this session.\n\
\n\
Use the update_memory tool to upsert memory blocks. Keep each block concise and factual. \
Do NOT summarise the conversation itself — only distil persistent knowledge. \
Do NOT create memory blocks for transient task details.".to_string(),
            skills:      vec![],
            scope:       SubagentScope::Builtin,
            path:        None,
        },
        SubagentDef {
            name:        "recall".to_string(),
            description: "Search past conversation history for relevant context".to_string(),
            model:       None,
            tools:       SubagentTools::Readonly,
            system_prompt: "\
You are a conversation history search agent. The user or main agent needs to recall something \
from past interactions. Search the provided conversation history or files for the requested \
information and return a precise, concise answer with source references (message index or \
file path and line). If nothing relevant is found, say so clearly.".to_string(),
            skills:      vec![],
            scope:       SubagentScope::Builtin,
            path:        None,
        },
    ]
}

// -- Discovery

/// Scan a directory for *.md files defining custom subagents.
fn discover_in_dir(dir: &Path, scope: SubagentScope) -> Vec<SubagentDef> {
    if !dir.exists() { return vec![]; }
    let mut defs = vec![];
    let Ok(entries) = std::fs::read_dir(dir) else { return vec![]; };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.extension().and_then(|e| e.to_str()) != Some("md") { continue; }
        let Ok(content) = std::fs::read_to_string(&path) else { continue; };
        let id = path.file_stem().unwrap_or_default().to_string_lossy().to_string();
        match parse_subagent_md(&id, &content, scope, path.clone()) {
            Ok(def) => defs.push(def),
            Err(e)  => tracing::warn!("Bad subagent at {}: {e}", path.display()),
        }
    }
    defs
}

/// Discover all subagents: built-ins < global < project (same name = higher scope wins).
pub fn discover_all_subagents(cwd: &Path) -> Vec<SubagentDef> {
    let mut all: Vec<SubagentDef> = builtin_subagents();

    // Global: ~/.cade/agents/
    if let Some(home) = dirs::home_dir() {
        all.extend(discover_in_dir(&home.join(".cade").join("agents"), SubagentScope::Global));
    }

    // Project: <cwd>/.cade/agents/
    all.extend(discover_in_dir(&cwd.join(".cade").join("agents"), SubagentScope::Project));

    // Merge: for each name keep highest-scope version
    let mut seen: std::collections::HashMap<String, usize> = std::collections::HashMap::new();
    let mut merged: Vec<SubagentDef> = vec![];
    for def in all {
        if let Some(&idx) = seen.get(&def.name) {
            if def.scope > merged[idx].scope {
                merged[idx] = def;
            }
        } else {
            seen.insert(def.name.clone(), merged.len());
            merged.push(def);
        }
    }
    merged.sort_by(|a, b| (a.scope as u8).cmp(&(b.scope as u8)).then(a.name.cmp(&b.name)));
    merged
}

/// Find a subagent definition by name from a list.
pub fn find_subagent<'a>(name: &str, all: &'a [SubagentDef]) -> Option<&'a SubagentDef> {
    all.iter().find(|d| d.name == name)
}

// -- Parsing

fn parse_subagent_md(id: &str, content: &str, scope: SubagentScope, path: PathBuf) -> Result<SubagentDef> {
    let content = content.trim();
    let (fm_str, body) = if content.starts_with("---") {
        match content[3..].find("---") {
            Some(end) => (&content[3..end + 3], &content[end + 6..]),
            None      => ("", content),
        }
    } else {
        ("", content)
    };

    let mut name        = id.to_string();
    let mut description = String::new();
    let mut model       = None::<String>;
    let mut tools       = SubagentTools::All;
    let mut skills      = vec![];

    for line in fm_str.lines() {
        let line = line.trim();
        if let Some((k, v)) = line.split_once(':') {
            let k = k.trim();
            let v = v.trim().trim_matches('"').trim_matches('\'');
            match k {
                "name"        => name        = v.to_string(),
                "description" => description = v.to_string(),
                "model"       => model       = Some(v.to_string()),
                "tools"       => tools       = SubagentTools::from_str(v),
                "skills"      => skills      = v.split(',').map(|s| s.trim().to_string()).collect(),
                _             => {}
            }
        }
    }

    Ok(SubagentDef {
        name, description, model, tools, skills, scope, path: Some(path),
        system_prompt: body.trim().to_string(),
    })
}

// -- Background result

#[derive(Debug, Clone)]
pub struct BackgroundResult {
    pub task_id:    String,
    pub subagent:   String,
    pub prompt_preview: String,
    pub result:     String,
    pub is_error:   bool,
}
