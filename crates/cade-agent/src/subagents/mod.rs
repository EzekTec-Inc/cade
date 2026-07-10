// region:    --- Modules

pub mod config;

pub use config::SubagentConfig;

use crate::Result;
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
    /// Explicit list of tool names and allowed file paths
    Restricted {
        allowed_tools: Vec<String>,
        allowed_paths: Vec<String>,
    },
}

impl std::fmt::Display for SubagentTools {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::All => write!(f, "all"),
            Self::Readonly => write!(f, "readonly"),
            Self::List(v) => write!(f, "{}", v.join(", ")),
            Self::Restricted {
                allowed_tools,
                allowed_paths,
            } => {
                write!(
                    f,
                    "restricted (tools: [{}], paths: [{}])",
                    allowed_tools.join(", "),
                    allowed_paths.join(", ")
                )
            }
        }
    }
}

impl SubagentTools {
    pub fn is_readonly(&self) -> bool {
        match self {
            Self::All => false,
            Self::Readonly => true,
            Self::List(tools) => {
                !tools.iter().any(|t| {
                    matches!(t.as_str(), "bash" | "shell" | "write_file" | "edit_file" | "apply_patch" | "create_file") || t.contains("__")
                })
            }
            Self::Restricted { allowed_tools, .. } => {
                !allowed_tools.iter().any(|t| {
                    matches!(t.as_str(), "bash" | "shell" | "write_file" | "edit_file" | "apply_patch" | "create_file") || t.contains("__")
                })
            }
        }
    }

    fn from_str(s: &str) -> Self {
        match s.trim().to_lowercase().as_str() {
            "all" => Self::All,
            "readonly" | "read-only" | "read_only" => Self::Readonly,
            other => {
                if other.starts_with('{')
                    && let Ok(v) = serde_json::from_str::<serde_json::Value>(other)
                    && let (Some(tools), Some(paths)) = (
                        v.get("allowed_tools").and_then(|v| v.as_array()),
                        v.get("allowed_paths").and_then(|v| v.as_array()),
                    )
                {
                    return Self::Restricted {
                        allowed_tools: tools
                            .iter()
                            .filter_map(|t| t.as_str().map(String::from))
                            .collect(),
                        allowed_paths: paths
                            .iter()
                            .filter_map(|p| p.as_str().map(String::from))
                            .collect(),
                    };
                }
                Self::List(
                    other
                        .split(',')
                        .map(|t| t.trim().to_string())
                        .filter(|t| !t.is_empty())
                        .collect(),
                )
            }
        }
    }
}

// -- Scope

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum SubagentScope {
    Builtin = 0,
    Global = 1,
    Project = 2,
}

impl std::fmt::Display for SubagentScope {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Builtin => write!(f, "builtin"),
            Self::Global => write!(f, "global"),
            Self::Project => write!(f, "project"),
        }
    }
}

// -- Subagent definition

#[derive(Debug, Clone)]
pub struct SubagentDef {
    pub name: String,
    pub description: String,
    /// None = inherit the main agent's current model
    pub model: Option<String>,
    pub tools: SubagentTools,
    pub system_prompt: String,
    pub skills: Vec<String>,
    pub scope: SubagentScope,
    /// Path to the defining .md file (None for built-ins)
    pub path: Option<PathBuf>,
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
            name: "worker".to_string(),
            description: "Highly capable unified worker — explore, plan, implement, and review".to_string(),
            model: None,
            tools: SubagentTools::All,
            system_prompt: "\
You are a highly capable unified worker agent. Complete the assigned task autonomously. \
You have full access to tools—use them dynamically to explore code, plan changes, and implement them. \
Ensure changes are correct and idiomatic. Report back with a clear summary of what you did, \
what files were changed, and any important decisions made. \
Use `archival_memory_insert` for storing large text artifacts or logs.\n\
\n\
CRITICAL: You are running in a headless autonomous loop without human interaction. \
Do NOT ask for permission or output conversational filler without making a tool call. \
If you do not emit a tool call, your execution will terminate immediately."
                .to_string(),
            skills: vec![],
            scope: SubagentScope::Builtin,
            path: None,
        },
        SubagentDef {
            name: "reflection".to_string(),
            description:
                "Background agent — reflects on the conversation and updates memory blocks"
                    .to_string(),
            model: None,
            tools: SubagentTools::List(vec![
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
Do NOT create memory blocks for transient task details."
                .to_string(),
            skills: vec![],
            scope: SubagentScope::Builtin,
            path: None,
        },
        SubagentDef {
            name: "recall".to_string(),
            description: "Search past conversation history for relevant context".to_string(),
            model: None,
            tools: SubagentTools::Readonly,
            system_prompt: "\
You are a conversation history search agent. The user or main agent needs to recall something \
from past interactions. Search the provided conversation history or files for the requested \
information and return a precise, concise answer with source references (message index or \
file path and line). If nothing relevant is found, say so clearly."
                .to_string(),
            skills: vec![],
            scope: SubagentScope::Builtin,
            path: None,
        },
    ]
}

// -- Discovery

/// Scan a directory for `*.md` and `*.json` files defining custom subagents.
fn discover_in_dir(dir: &Path, scope: SubagentScope) -> Vec<SubagentDef> {
    if !dir.exists() {
        return vec![];
    }
    let mut defs = vec![];
    let Ok(entries) = std::fs::read_dir(dir) else {
        return vec![];
    };
    for entry in entries.flatten() {
        let path = entry.path();
        let ext = path.extension().and_then(|e| e.to_str());
        match ext {
            Some("md") => {
                let Ok(content) = std::fs::read_to_string(&path) else {
                    continue;
                };
                let id = path
                    .file_stem()
                    .unwrap_or_default()
                    .to_string_lossy()
                    .to_string();
                match parse_subagent_md(&id, &content, scope, path.clone()) {
                    Ok(def) => defs.push(def),
                    Err(e) => tracing::warn!("Bad subagent at {}: {e}", path.display()),
                }
            }
            Some("json") => {
                let Ok(content) = std::fs::read_to_string(&path) else {
                    continue;
                };
                let id = path
                    .file_stem()
                    .unwrap_or_default()
                    .to_string_lossy()
                    .to_string();
                match parse_subagent_json(&id, &content, scope, path.clone()) {
                    Ok(def) => defs.push(def),
                    Err(e) => tracing::warn!("Bad subagent profile at {}: {e}", path.display()),
                }
            }
            _ => {}
        }
    }
    defs
}

/// Discover all subagents: built-ins < global < project (same name = higher scope wins).
pub fn discover_all_subagents(cwd: &Path) -> Vec<SubagentDef> {
    let mut all: Vec<SubagentDef> = builtin_subagents();

    // Global: ~/.cade/subagents/
    if let Some(home) = dirs::home_dir() {
        all.extend(discover_in_dir(
            &home.join(".cade").join("subagents"),
            SubagentScope::Global,
        ));
    }

    // Project: <cwd>/.cade/subagents/
    all.extend(discover_in_dir(
        &cwd.join(".cade").join("subagents"),
        SubagentScope::Project,
    ));

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
    merged.sort_by(|a, b| {
        (a.scope as u8)
            .cmp(&(b.scope as u8))
            .then(a.name.cmp(&b.name))
    });
    merged
}

/// Find a subagent definition by name from a list.
pub fn find_subagent<'a>(name: &str, all: &'a [SubagentDef]) -> Option<&'a SubagentDef> {
    all.iter().find(|d| d.name == name)
}

/// Resolve which subagent definition should run for a given `mode` argument.
///
/// Selection order:
/// 1. Exact name match against `all` (lets users put a custom `bug-hunter.md`
///    into `~/.cade/subagents/` and call `run_subagent(mode="bug-hunter")`).
/// 2. Fallback to the built-in `worker` definition, so existing prompts that
///    pass `mode="build"`, `mode="plan"`, etc. keep working unchanged.
/// 3. `None` only if neither the named def nor `worker` are present —
///    callers must handle this with a default system prompt.
///
/// Pure: no I/O, no clones except the trivial `Option<&_>` slot — the caller
/// decides whether to clone the returned definition.
#[must_use]
pub fn resolve_subagent_def<'a>(mode: &str, all: &'a [SubagentDef]) -> Option<&'a SubagentDef> {
    find_subagent(mode, all).or_else(|| find_subagent("worker", all))
}

// -- Parsing

/// Parse a JSON profile file into a [`SubagentDef`].
///
/// Expected schema:
/// ```json
/// {
///   "name":          "tester",
///   "description":   "Runs the test suite",
///   "model":         "anthropic/claude-haiku-4-5",
///   "tools":         ["bash", "read_file", "glob"],
///   "system_prompt": "You are a test runner.",
///   "skills":        []
/// }
/// ```
///
/// `tools` can be:
/// - `"all"` or `"readonly"` (string shortcuts)
/// - `["bash", "read_file"]` (explicit list)
/// - absent / `null`  → `SubagentTools::All`
fn parse_subagent_json(
    id: &str,
    content: &str,
    scope: SubagentScope,
    path: PathBuf,
) -> std::result::Result<SubagentDef, String> {
    let v: serde_json::Value =
        serde_json::from_str(content).map_err(|e| format!("JSON parse error: {e}"))?;

    let name = v["name"]
        .as_str()
        .filter(|s| !s.is_empty())
        .unwrap_or(id)
        .to_string();

    let description = v["description"].as_str().unwrap_or("").to_string();

    let model = v["model"]
        .as_str()
        .filter(|s| !s.is_empty())
        .map(String::from);

    let tools = match &v["tools"] {
        serde_json::Value::String(s) => SubagentTools::from_str(s),
        serde_json::Value::Array(arr) => {
            let list: Vec<String> = arr
                .iter()
                .filter_map(|t| t.as_str().map(String::from))
                .collect();
            SubagentTools::List(list)
        }
        _ => SubagentTools::All,
    };

    let system_prompt = v["system_prompt"].as_str().unwrap_or("").to_string();

    let skills: Vec<String> = v["skills"]
        .as_array()
        .map(|arr| {
            arr.iter()
                .filter_map(|s| s.as_str().map(String::from))
                .collect()
        })
        .unwrap_or_default();

    Ok(SubagentDef {
        name,
        description,
        model,
        tools,
        system_prompt,
        skills,
        scope,
        path: Some(path),
    })
}

fn parse_subagent_md(
    id: &str,
    content: &str,
    scope: SubagentScope,
    path: PathBuf,
) -> Result<SubagentDef> {
    let content = content.trim();
    let (fm_str, body) = if let Some(stripped) = content.strip_prefix("---") {
        match stripped.find("---") {
            Some(end) => (&content[3..end + 3], &content[end + 6..]),
            None => ("", content),
        }
    } else {
        ("", content)
    };

    let mut name = id.to_string();
    let mut description = String::new();
    let mut model = None::<String>;
    let mut tools = SubagentTools::All;
    let mut skills = vec![];

    for line in fm_str.lines() {
        let line = line.trim();
        if let Some((k, v)) = line.split_once(':') {
            let k = k.trim();
            let v = v.trim().trim_matches('"').trim_matches('\'');
            match k {
                "name" => name = v.to_string(),
                "description" => description = v.to_string(),
                "model" => model = Some(v.to_string()),
                "tools" => tools = SubagentTools::from_str(v),
                "skills" => skills = v.split(',').map(|s| s.trim().to_string()).collect(),
                _ => {}
            }
        }
    }

    Ok(SubagentDef {
        name,
        description,
        model,
        tools,
        skills,
        scope,
        path: Some(path),
        system_prompt: body.trim().to_string(),
    })
}

// -- Scaffold built-in profiles

/// Write example JSON profile files to `~/.cade/subagents/` the first time
/// a user runs CADE, so they have working templates to customise.
///
/// This function is intentionally **not** called automatically — the caller
/// decides when to invoke it (e.g. on first-run detection).
///
/// Files are only written if they do **not** already exist, so re-invoking
/// this is safe and idempotent.
pub fn scaffold_builtin_profiles(cwd: &Path) {
    let _ = cwd; // reserved for future project-local scaffolding

    let Some(home) = dirs::home_dir() else {
        tracing::warn!("scaffold_builtin_profiles: cannot determine home directory");
        return;
    };
    let dir = home.join(".cade").join("subagents");
    if let Err(e) = std::fs::create_dir_all(&dir) {
        tracing::warn!(
            "scaffold_builtin_profiles: cannot create {}: {e}",
            dir.display()
        );
        return;
    }

    let profiles: &[(&str, &str)] = &[
        (
            "tester.json",
            r#"{
  "name": "tester",
  "description": "Runs the test suite and reports failures",
  "model": "anthropic/claude-haiku-4-5",
  "tools": ["bash", "read_file", "glob"],
  "system_prompt": "You are a test runner. Your only job is to run the project's test suite and report any failures concisely.\n\n1. Find the test command (look for Makefile, package.json, Cargo.toml, etc.).\n2. Run it.\n3. Collect failures and report them with file paths and line numbers.\nDo NOT fix anything — only report.",
  "skills": []
}
"#,
        ),
        (
            "refactorer.json",
            r#"{
  "name": "refactorer",
  "description": "Reads code and applies targeted refactoring edits",
  "model": null,
  "tools": ["read_file", "edit_file", "glob", "grep"],
  "system_prompt": "You are a refactoring specialist. You improve code structure, readability, and maintainability without changing observable behaviour.\n\nRules:\n- Read the relevant files first.\n- Make minimal, targeted edits.\n- Do NOT add new features or fix bugs unless explicitly asked.\n- Preserve all public APIs and existing tests.",
  "skills": []
}
"#,
        ),
        (
            "researcher.json",
            r#"{
  "name": "researcher",
  "description": "Read-only research agent — explores code and web, never writes files",
  "model": null,
  "tools": "readonly",
  "system_prompt": "You are a read-only research assistant. Your job is to gather information and produce a clear, concise report.\n\nYou MAY:\n- Read files (read_file, glob, grep)\n- Search the web (web_search)\n- Search memory\n\nYou MUST NOT modify any file or run mutating shell commands. Return a structured report with your findings.",
  "skills": []
}
"#,
        ),
    ];

    for (filename, content) in profiles {
        let dest = dir.join(filename);
        if dest.exists() {
            continue; // never overwrite user edits
        }
        match std::fs::write(&dest, content) {
            Ok(()) => tracing::info!("scaffolded {}", dest.display()),
            Err(e) => tracing::warn!(
                "scaffold_builtin_profiles: cannot write {}: {e}",
                dest.display()
            ),
        }
    }
}

// -- Background result

#[derive(Debug, Clone)]
pub struct BackgroundResult {
    pub task_id: String,
    pub subagent: String,
    pub prompt_preview: String,
    pub result: String,
    pub is_error: bool,
}

// -- Background completion notification (Option 1: terminal BEL)

/// Decide whether a background subagent completion should emit a terminal
/// BEL byte (`0x07`) to alert the user.
///
/// Pure decision function — kept separate from `std::io::stdout()` so it
/// can be unit-tested without touching the real terminal.  The CLI calls
/// this from the spawned background task and, if it returns `true`,
/// writes a single BEL byte to stdout.
///
/// Rules:
/// - `silent`: user opted out (e.g. `silent_subagents` setting).  Never bell.
/// - `is_tty`: only bell when stdout is an interactive terminal.  CI logs,
///   piped output, and redirected files must not receive control bytes.
/// - Errors and successes both bell — the user wants to know either way.
#[must_use]
pub fn should_emit_completion_bell(silent: bool, is_tty: bool) -> bool {
    !silent && is_tty
}

/// Build the toast message shown by the TUI when one or more background
/// subagents have completed and are waiting in the pending-results queue.
///
/// Pure formatter — returns `None` when there is nothing to surface, so
/// the caller can early-return without touching `self.toast`.  Singular vs
/// plural is handled here so the TUI tick loop stays a one-liner.
#[must_use]
pub fn pending_bg_toast(pending: usize) -> Option<String> {
    match pending {
        0 => None,
        1 => Some("✓ Subagent finished — press Enter to receive".to_string()),
        n => Some(format!("✓ {n} subagents finished — press Enter to receive")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bell_fires_on_normal_completion_in_tty() {
        assert!(should_emit_completion_bell(false, true));
    }

    #[test]
    fn bell_suppressed_when_silent_subagents_set() {
        assert!(!should_emit_completion_bell(true, true));
    }

    #[test]
    fn bell_suppressed_when_stdout_not_tty() {
        // Avoid corrupting CI logs / piped output with control bytes.
        assert!(!should_emit_completion_bell(false, false));
    }

    #[test]
    fn silent_dominates_tty() {
        assert!(!should_emit_completion_bell(true, false));
    }

    #[test]
    fn pending_toast_none_when_empty() {
        assert_eq!(pending_bg_toast(0), None);
    }

    #[test]
    fn pending_toast_singular_for_one() {
        assert_eq!(
            pending_bg_toast(1).as_deref(),
            Some("✓ Subagent finished — press Enter to receive"),
        );
    }

    #[test]
    fn pending_toast_plural_for_many() {
        assert_eq!(
            pending_bg_toast(3).as_deref(),
            Some("✓ 3 subagents finished — press Enter to receive"),
        );
    }

    // -- resolve_subagent_def

    fn def(name: &str) -> SubagentDef {
        SubagentDef {
            name: name.to_string(),
            description: format!("test-{name}"),
            model: None,
            tools: SubagentTools::All,
            system_prompt: format!("prompt-{name}"),
            skills: vec![],
            scope: SubagentScope::Builtin,
            path: None,
        }
    }

    #[test]
    fn resolve_exact_name_match_wins() {
        let defs = vec![def("worker"), def("rust-dev-worker")];
        let got = resolve_subagent_def("rust-dev-worker", &defs);
        assert_eq!(got.map(|d| d.name.as_str()), Some("rust-dev-worker"));
    }

    #[test]
    fn resolve_falls_back_to_worker_when_mode_unknown() {
        let defs = vec![def("worker"), def("recall")];
        let got = resolve_subagent_def("build", &defs);
        // "build" is not a defined name; must fall back to worker.
        assert_eq!(got.map(|d| d.name.as_str()), Some("worker"));
    }

    #[test]
    fn resolve_returns_none_when_neither_mode_nor_worker_present() {
        let defs = vec![def("recall")];
        let got = resolve_subagent_def("bug-hunter", &defs);
        assert!(got.is_none());
    }

    #[test]
    fn resolve_empty_mode_string_falls_back_to_worker() {
        let defs = vec![def("worker")];
        let got = resolve_subagent_def("", &defs);
        // An empty mode never matches any name, so fallback applies.
        assert_eq!(got.map(|d| d.name.as_str()), Some("worker"));
    }

    #[test]
    fn resolve_does_not_match_worker_when_mode_says_worker_explicitly() {
        // Sanity: if mode == "worker" the exact match is just worker; same
        // result either way.  Locks in the no-double-match behaviour.
        let defs = vec![def("worker")];
        let got = resolve_subagent_def("worker", &defs);
        assert_eq!(got.map(|d| d.name.as_str()), Some("worker"));
    }

    // -- Bug 2+3: system prompt inherited from resolved definition

    #[test]
    fn resolved_def_carries_system_prompt() {
        let defs = vec![def("worker"), def("bug-hunter")];
        let got = resolve_subagent_def("bug-hunter", &defs).unwrap();
        assert_eq!(got.system_prompt, "prompt-bug-hunter");
    }

    #[test]
    fn worker_fallback_carries_worker_system_prompt() {
        let defs = vec![def("worker")];
        let got = resolve_subagent_def("build", &defs).unwrap();
        assert_eq!(got.system_prompt, "prompt-worker");
    }

    #[test]
    fn custom_def_model_available_for_override() {
        let mut custom = def("custom-agent");
        custom.model = Some("anthropic/claude-haiku-4-5".to_string());
        let defs = vec![def("worker"), custom];
        let got = resolve_subagent_def("custom-agent", &defs).unwrap();
        assert_eq!(got.model.as_deref(), Some("anthropic/claude-haiku-4-5"));
    }
}
