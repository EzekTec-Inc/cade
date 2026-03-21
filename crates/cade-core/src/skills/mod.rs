// region:    --- Modules

use crate::{Error, Result};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

// endregion: --- Modules

// -- Skill scope

/// Where a skill was loaded from. Higher priority scopes override lower ones
/// when two skills share the same ID.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub enum SkillScope {
    /// Built-in skills shipped with CADE
    Builtin = 0,
    /// Machine-global skills in ~/.cade/skills/
    Global = 1,
    /// Agent-scoped skills in ~/.cade/agents/{id}/skills/
    Agent = 2,
    /// Project-scoped skills in <cwd>/.skills/  (highest priority)
    Project = 3,
}

impl std::fmt::Display for SkillScope {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Builtin => write!(f, "builtin"),
            Self::Global => write!(f, "global"),
            Self::Agent => write!(f, "agent"),
            Self::Project => write!(f, "project"),
        }
    }
}

// -- Skill

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Skill {
    pub id: String,
    pub name: String,
    pub description: String,
    pub category: Option<String>,
    pub tags: Vec<String>,
    /// Keyword/phrase triggers — agent auto-activates this skill when input matches
    pub triggers: Vec<String>,
    /// RPI phase this skill is active in: Research | Plan | Implement | Verification
    pub rpi_phase: Option<String>,
    /// High-level capabilities this skill provides (for display + routing)
    pub capabilities: Vec<String>,
    /// Executable scripts in `<skill_dir>/scripts/` — name → relative path
    pub scripts: Vec<SkillScript>,
    /// Reference docs in `<skill_dir>/references/` — available for lazy loading
    pub references: Vec<SkillReference>,
    pub body: String,
    pub scope: SkillScope,
    /// Absolute path to the SKILL.MD file
    pub path: PathBuf,
}

/// An executable script bundled with a skill.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SkillScript {
    /// Script name (stem of filename, e.g. "explain_error")
    pub name: String,
    /// Description from SKILL.MD `tools:` block, if present
    pub description: String,
    /// Absolute path to the script file
    pub path: PathBuf,
}

/// A reference document bundled with a skill.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SkillReference {
    /// Filename stem (e.g. "dictionary_of_pain")
    pub name: String,
    /// Absolute path to the reference file
    pub path: PathBuf,
}

impl Skill {
    /// One-line entry for the skills listing injected into the system prompt.
    pub fn listing_line(&self) -> String {
        let cat = self
            .category
            .as_deref()
            .map(|c| format!(" [{c}]"))
            .unwrap_or_default();
        let phase = self
            .rpi_phase
            .as_deref()
            .map(|p| format!(" <{p}>"))
            .unwrap_or_default();
        format!(
            "- {} [{}]{}{}: {}",
            self.id, self.scope, cat, phase, self.description
        )
    }

    /// Full formatted block returned by `load_skill` tool.
    pub fn to_context_block(&self) -> String {
        let cat = self
            .category
            .as_deref()
            .map(|c| format!("[{c}] "))
            .unwrap_or_default();
        let mut out = format!(
            "## Skill: {} {cat}\nID: {}\nScope: {}\nDescription: {}\n",
            self.name, self.id, self.scope, self.description
        );
        if !self.capabilities.is_empty() {
            out.push_str(&format!("Capabilities: {}\n", self.capabilities.join(", ")));
        }
        if !self.triggers.is_empty() {
            out.push_str(&format!("Triggers: {}\n", self.triggers.join(", ")));
        }
        if let Some(phase) = &self.rpi_phase {
            out.push_str(&format!("RPI Phase: {phase}\n"));
        }
        if !self.scripts.is_empty() {
            out.push_str("\nAvailable scripts (call with run_skill_script):\n");
            for s in &self.scripts {
                out.push_str(&format!("  - {} : {}\n", s.name, s.description));
            }
        }
        if !self.references.is_empty() {
            out.push_str("\nReference docs (load with load_skill_ref):\n");
            for r in &self.references {
                out.push_str(&format!("  - {}\n", r.name));
            }
        }
        out.push('\n');
        out.push_str(&self.body);
        out.push('\n');
        out
    }

    /// Returns true if the given text matches any of this skill's triggers.
    /// Single-word triggers require a whole-token match (word boundary).
    /// Multi-word triggers (containing a space) fall back to substring match.
    pub fn matches_trigger(&self, text: &str) -> bool {
        if self.triggers.is_empty() {
            return false;
        }
        let lower = text.to_lowercase();
        // Tokenise input: split on anything that is not alphanumeric / _ / -
        let words: Vec<&str> = lower
            .split(|c: char| !c.is_alphanumeric() && c != '_' && c != '-')
            .filter(|w| !w.is_empty())
            .collect();
        self.triggers.iter().any(|t| {
            let tl = t.to_lowercase();
            if tl.contains(' ') {
                // Multi-word phrase: substring match is intentional
                lower.contains(&tl)
            } else {
                // Single-word trigger: must match a whole token
                words.iter().any(|w| *w == tl)
            }
        })
    }
}

// -- Discovery

/// Scan `dir` for SKILL.MD files, tagging each with `scope`.
pub fn discover_skills_in(dir: &Path, scope: SkillScope) -> Vec<Skill> {
    if !dir.exists() {
        return vec![];
    }
    let mut skills = Vec::new();
    let walker = walkdir::WalkDir::new(dir)
        .into_iter()
        .filter_map(|e| e.ok())
        .filter(|e| {
            e.file_type().is_file()
                && e.file_name()
                    .to_str()
                    .map(|n| n.to_uppercase() == "SKILL.MD")
                    .unwrap_or(false)
        });

    for entry in walker {
        let path = entry.path();
        let content = match std::fs::read_to_string(path) {
            Ok(c) => c,
            Err(e) => {
                tracing::warn!("Cannot read {}: {e}", path.display());
                continue;
            }
        };
        let rel = path.strip_prefix(dir).unwrap_or(path);
        let id = rel
            .parent()
            .map(|p| p.to_string_lossy().replace(std::path::MAIN_SEPARATOR, "/"))
            .unwrap_or_default();
        if id.is_empty() {
            continue;
        }

        match parse_skill(&id, &content, scope, path.to_path_buf()) {
            Ok(s) => {
                tracing::debug!("Loaded skill: {} [{}]", s.id, s.scope);
                skills.push(s);
            }
            Err(e) => tracing::warn!("Bad skill at {}: {e}", path.display()),
        }
    }
    skills
}

/// Discover skills from all scopes and merge by priority.
/// Project > Agent > Global > Builtin (same ID → higher scope wins).
pub fn discover_all_skills(
    cwd: &Path,
    agent_id: Option<&str>,
    cade_home: Option<&Path>,
) -> Vec<Skill> {
    let home = dirs::home_dir();
    let cade_home = cade_home
        .map(|p| p.to_path_buf())
        .or_else(|| home.as_ref().map(|h| h.join(".cade")));

    let mut all: Vec<Skill> = Vec::new();

    // Lowest priority first
    if let Some(ch) = &cade_home {
        all.extend(discover_skills_in(&ch.join("skills"), SkillScope::Global));
        if let Some(id) = agent_id {
            all.extend(discover_skills_in(
                &ch.join("agents").join(id).join("skills"),
                SkillScope::Agent,
            ));
        }
    }
    all.extend(discover_skills_in(
        &cwd.join(".skills"),
        SkillScope::Project,
    ));

    // Merge: for each ID keep only the highest-scope version
    let mut seen: std::collections::HashMap<String, usize> = std::collections::HashMap::new();
    let mut merged: Vec<Skill> = Vec::new();
    for skill in all {
        if let Some(&idx) = seen.get(&skill.id) {
            if skill.scope > merged[idx].scope {
                merged[idx] = skill;
            }
        } else {
            seen.insert(skill.id.clone(), merged.len());
            merged.push(skill);
        }
    }
    merged.sort_by(|a, b| a.id.cmp(&b.id));
    merged
}

/// Compact listing for injection into the system prompt.
/// Only names + descriptions — full content loaded on-demand via load_skill.
pub fn skills_listing(skills: &[Skill]) -> Option<String> {
    if skills.is_empty() {
        return None;
    }
    let mut out = String::from(
        "# Available Skills\n\
         Use the `load_skill` tool to load a skill's full content when working on a relevant task.\n\n",
    );
    for s in skills {
        out.push_str(&s.listing_line());
        out.push('\n');
    }
    Some(out)
}

/// Full context block for ALL skills — kept for backwards compat / debug.
#[deprecated(note = "Injects too many tokens. Use skills_listing instead.")]
#[allow(deprecated)] // Keep here in case it's used internally for debug temporarily
pub fn skills_context(skills: &[Skill]) -> Option<String> {
    if skills.is_empty() {
        return None;
    }
    let mut out = "# Available Skills\n\n".to_string();
    for s in skills {
        out.push_str(&s.to_context_block());
        out.push('\n');
    }
    Some(out)
}

// -- Parsing

fn parse_skill(id: &str, content: &str, scope: SkillScope, path: PathBuf) -> Result<Skill> {
    let content = content.trim();
    let (fm_str, body) = if let Some(stripped) = content.strip_prefix("---") {
        match stripped.find("---") {
            Some(end) => (&content[3..end + 3], &content[end + 6..]),
            None => ("", content),
        }
    } else {
        ("", content)
    };

    let fm = parse_frontmatter(fm_str);

    // Discover scripts/ and references/ relative to the SKILL.MD file
    let skill_dir = path.parent().unwrap_or(path.as_path());
    let scripts = discover_scripts(skill_dir, &fm.tools);
    let references = discover_references(skill_dir);

    Ok(Skill {
        id: id.to_string(),
        name: fm.name.unwrap_or_else(|| id.to_string()),
        description: fm.description.unwrap_or_default(),
        category: fm.category,
        tags: fm.tags,
        triggers: fm.triggers,
        rpi_phase: fm.rpi_phase,
        capabilities: fm.capabilities,
        scripts,
        references,
        body: body.trim().to_string(),
        scope,
        path,
    })
}

/// Scan `<skill_dir>/scripts/` for executable files.
fn discover_scripts(skill_dir: &Path, tool_hints: &[FrontmatterTool]) -> Vec<SkillScript> {
    let scripts_dir = skill_dir.join("scripts");
    if !scripts_dir.exists() {
        return vec![];
    }
    let mut scripts = Vec::new();
    if let Ok(entries) = std::fs::read_dir(&scripts_dir) {
        for entry in entries.flatten() {
            let p = entry.path();
            if p.is_file() {
                let name = p
                    .file_stem()
                    .and_then(|n| n.to_str())
                    .unwrap_or("")
                    .to_string();
                if name.is_empty() {
                    continue;
                }
                // Match description from frontmatter tools: block if present
                let description = tool_hints
                    .iter()
                    .find(|t| {
                        t.entrypoint
                            .as_deref()
                            .map(|e| e.contains(&name))
                            .unwrap_or(false)
                            || t.name == name
                    })
                    .map(|t| t.description.clone())
                    .unwrap_or_default();
                scripts.push(SkillScript {
                    name,
                    description,
                    path: p,
                });
            }
        }
    }
    scripts.sort_by(|a, b| a.name.cmp(&b.name));
    scripts
}

/// Scan `<skill_dir>/references/` for documentation files.
fn discover_references(skill_dir: &Path) -> Vec<SkillReference> {
    let refs_dir = skill_dir.join("references");
    if !refs_dir.exists() {
        return vec![];
    }
    let mut refs = Vec::new();
    if let Ok(entries) = std::fs::read_dir(&refs_dir) {
        for entry in entries.flatten() {
            let p = entry.path();
            if p.is_file() {
                let name = p
                    .file_stem()
                    .and_then(|n| n.to_str())
                    .unwrap_or("")
                    .to_string();
                if name.is_empty() {
                    continue;
                }
                refs.push(SkillReference { name, path: p });
            }
        }
    }
    refs.sort_by(|a, b| a.name.cmp(&b.name));
    refs
}

#[derive(Default)]
struct FrontmatterTool {
    name: String,
    description: String,
    entrypoint: Option<String>,
}

#[derive(Default)]
struct Frontmatter {
    name: Option<String>,
    description: Option<String>,
    category: Option<String>,
    tags: Vec<String>,
    triggers: Vec<String>,
    rpi_phase: Option<String>,
    capabilities: Vec<String>,
    tools: Vec<FrontmatterTool>,
}

fn parse_frontmatter(fm: &str) -> Frontmatter {
    let mut out = Frontmatter::default();
    let mut in_tools = false;
    let mut current_tool: Option<FrontmatterTool> = None;
    // Tracks which top-level list field a YAML multiline block belongs to.
    let mut current_list_field: Option<&str> = None;

    for line in fm.lines() {
        let trimmed = line.trim();

        // Detect `tools:` block start
        if trimmed == "tools:" {
            in_tools = true;
            current_list_field = None;
            continue;
        }

        // If we hit another top-level key (no leading spaces/dash), exit tools block
        if in_tools && !line.starts_with(' ') && !line.starts_with('-') && !trimmed.is_empty() {
            if let Some(t) = current_tool.take() {
                out.tools.push(t);
            }
            in_tools = false;
        }

        if in_tools {
            // New tool entry
            if trimmed.starts_with("- name:") {
                if let Some(t) = current_tool.take() {
                    out.tools.push(t);
                }
                let val = trimmed
                    .trim_start_matches("- name:")
                    .trim()
                    .trim_matches('"')
                    .to_string();
                current_tool = Some(FrontmatterTool {
                    name: val,
                    ..Default::default()
                });
            } else if let Some(t) = &mut current_tool
                && let Some((k, v)) = trimmed.split_once(':')
            {
                let v = v.trim().trim_matches('"').trim_matches('\'');
                match k.trim() {
                    "description" => t.description = v.to_string(),
                    "entrypoint" => t.entrypoint = Some(v.to_string()),
                    _ => {}
                }
            }
            continue;
        }

        // YAML multiline list item (e.g. `  - item`)
        if trimmed.starts_with("- ") && (line.starts_with(' ') || line.starts_with('\t')) {
            if let Some(field) = current_list_field {
                let item = trimmed[2..]
                    .trim()
                    .trim_matches('"')
                    .trim_matches('\'')
                    .to_string();
                if !item.is_empty() {
                    match field {
                        "tags" => out.tags.push(item),
                        "trigger" | "triggers" => out.triggers.push(item),
                        "capabilities" => out.capabilities.push(item),
                        _ => {}
                    }
                }
            }
            continue;
        }

        // Any non-list, non-empty line resets the list-field context
        if !trimmed.is_empty() {
            current_list_field = None;
        }

        // Top-level keys
        if let Some((key, val)) = trimmed.split_once(':') {
            let key = key.trim();
            let val = val.trim().trim_matches('"').trim_matches('\'');
            match key {
                "name" => out.name = Some(val.to_string()),
                "description" => out.description = Some(val.to_string()),
                "category" => out.category = Some(val.to_string()),
                "rpi_phase" => out.rpi_phase = Some(val.to_string()),
                "tags" | "trigger" | "triggers" | "capabilities" => {
                    if val.is_empty() {
                        // No inline value — items will follow as `  - item` lines
                        current_list_field = Some(key);
                    } else {
                        let parsed: Vec<String> = val
                            .trim_matches(|c| c == '[' || c == ']')
                            .split(',')
                            .map(|s| s.trim().trim_matches('"').trim_matches('\'').to_string())
                            .filter(|s| !s.is_empty())
                            .collect();
                        match key {
                            "tags" => out.tags = parsed,
                            "trigger" | "triggers" => out.triggers = parsed,
                            "capabilities" => out.capabilities = parsed,
                            _ => {}
                        }
                    }
                }
                _ => {}
            }
        }
    }

    // Flush final tool
    if let Some(t) = current_tool.take() {
        out.tools.push(t);
    }

    out
}

// -- Live file watcher

/// Spawn a background thread that watches all skills directories for changes.
/// Any Create / Modify / Remove event sends a `()` on the returned channel.
/// The REPL polls the receiver each loop iteration to trigger a reload.
///
/// Uses `notify` 6.x `RecommendedWatcher` (inotify on Linux, FSEvents on macOS,
/// ReadDirectoryChangesW on Windows). The watcher runs on a dedicated std thread
/// (notify is not async-native) and forwards events to a `tokio::sync::mpsc` channel.
pub fn spawn_skill_watcher(cwd: &Path) -> tokio::sync::mpsc::Receiver<()> {
    use notify::event::{CreateKind, ModifyKind, RemoveKind};
    use notify::{Config, Event, EventKind, RecommendedWatcher, RecursiveMode, Watcher};

    let (tx, rx) = tokio::sync::mpsc::channel::<()>(8);

    // Collect directories to watch
    let home = dirs::home_dir();
    let cade_home = home.as_ref().map(|h| h.join(".cade"));

    let mut watch_dirs: Vec<PathBuf> = Vec::new();

    if let Some(ch) = &cade_home {
        let global_skills = ch.join("skills");
        if global_skills.exists() {
            watch_dirs.push(global_skills);
        }
    }

    let project_skills = cwd.join(".skills");
    if project_skills.exists() {
        watch_dirs.push(project_skills.clone());
    }

    if watch_dirs.is_empty() {
        // No dirs to watch yet — still return the receiver; caller can start
        // without a watcher. The REPL will never receive on this channel.
        return rx;
    }

    std::thread::spawn(move || {
        // notify 6.x: create watcher with a sync std::sync::mpsc channel internally,
        // then forward to tokio channel via try_send.
        let (sync_tx, sync_rx) = std::sync::mpsc::channel::<notify::Result<Event>>();

        let mut watcher = match RecommendedWatcher::new(sync_tx, Config::default()) {
            Ok(w) => w,
            Err(e) => {
                tracing::warn!("skill watcher: failed to create watcher: {e}");
                return;
            }
        };

        for dir in &watch_dirs {
            if let Err(e) = watcher.watch(dir, RecursiveMode::Recursive) {
                tracing::warn!("skill watcher: cannot watch {}: {e}", dir.display());
            } else {
                tracing::info!("skill watcher: watching {}", dir.display());
            }
        }

        // Forward relevant events to the tokio channel
        for res in sync_rx {
            match res {
                Ok(event) => {
                    let relevant = matches!(
                        event.kind,
                        EventKind::Create(CreateKind::File)
                            | EventKind::Create(CreateKind::Any)
                            | EventKind::Modify(ModifyKind::Data(_))
                            | EventKind::Modify(ModifyKind::Any)
                            | EventKind::Remove(RemoveKind::File)
                            | EventKind::Remove(RemoveKind::Any)
                    );
                    // Only care about SKILL.MD files
                    let is_skill_file = event.paths.iter().any(|p| {
                        p.file_name()
                            .and_then(|n| n.to_str())
                            .map(|n| n.to_uppercase() == "SKILL.MD")
                            .unwrap_or(false)
                    });
                    if relevant && is_skill_file {
                        // Non-blocking send — drop if receiver is behind
                        let _ = tx.try_send(());
                    }
                }
                Err(e) => tracing::warn!("skill watcher error: {e}"),
            }
        }
    });

    rx
}

/// Convert a GitHub tree URL to a raw SKILL.MD URL.
/// https://github.com/USER/REPO/tree/BRANCH/path → https://raw.../path/SKILL.MD
pub fn github_url_to_raw_skill(url: &str) -> Option<String> {
    // Match: https://github.com/<user>/<repo>/tree/<branch>/<path...>
    let stripped = url
        .trim_start_matches("https://github.com/")
        .trim_start_matches("http://github.com/");
    let parts: Vec<&str> = stripped.splitn(5, '/').collect();
    if parts.len() >= 5 && parts[2] == "tree" {
        let user = parts[0];
        let repo = parts[1];
        let branch = parts[3];
        let path = parts[4];
        Some(format!(
            "https://raw.githubusercontent.com/{user}/{repo}/{branch}/{path}/SKILL.MD"
        ))
    } else if parts.len() >= 5 && parts[2] == "blob" {
        // Direct file URL — return as-is converted to raw
        let branch = parts[3];
        let path = parts[4..].join("/");
        Some(format!(
            "https://raw.githubusercontent.com/{}/{}/{}/{}",
            parts[0], parts[1], branch, path
        ))
    } else {
        None
    }
}

// endregion: --- Tests

/// Write edited skill fields back to the SKILL.MD file on disk.
/// fields: [name, description, category, tags_csv, triggers_csv, body]
pub fn write_skill_to_disk(skill: &Skill, fields: &[String]) -> std::io::Result<()> {
    let name = &fields[0];
    let desc = &fields[1];
    let cat = &fields[2];
    let tags_str = &fields[3];
    let trig_str = &fields[4];
    let body = &fields[5];

    let fmt_list = |s: &str| -> String {
        let items: Vec<String> = s
            .split(',')
            .map(|t| t.trim().to_string())
            .filter(|t| !t.is_empty())
            .collect();
        if items.is_empty() {
            "[]".to_string()
        } else {
            format!(
                "[{}]",
                items
                    .iter()
                    .map(|t| format!("\"{}\"", t))
                    .collect::<Vec<_>>()
                    .join(", ")
            )
        }
    };

    let tags_yaml = fmt_list(tags_str);
    let trigs_yaml = fmt_list(trig_str);

    let content = format!(
        "---\nname: {name}\ndescription: {desc}\ncategory: {cat}\ntags: {tags_yaml}\ntriggers: {trigs_yaml}\n---\n\n{body}"
    );
    std::fs::write(&skill.path, content)
}

/// Download and install a skill from a URL into `target_dir/<skill-name>/SKILL.MD`.
/// Returns the installed skill on success.
pub async fn install_skill_from_url(url: &str, target_dir: &Path) -> Result<Skill> {
    // Resolve to raw content URL if needed
    let raw_url = if url.contains("github.com") && url.contains("/tree/") {
        github_url_to_raw_skill(url)
            .ok_or_else(|| Error::custom(format!("Cannot parse GitHub URL: {url}")))?
    } else {
        url.to_string()
    };

    // Derive skill ID from URL path
    let skill_id = raw_url
        .trim_end_matches("/SKILL.MD")
        .trim_end_matches("/SKILL.md")
        .rsplit('/')
        .next()
        .unwrap_or("downloaded-skill")
        .to_lowercase()
        .replace(' ', "-");

    // SEC-B4: Validate derived skill ID to prevent path traversal
    if !skill_id
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || c == '-')
    {
        return Err(Error::custom(format!("Invalid skill ID derived from URL: {skill_id}")));
    }

    let skill_dir = target_dir.join(&skill_id);
    let skill_file = skill_dir.join("SKILL.MD");

    if skill_file.exists() {
        return Err(Error::custom(format!(
            "Skill '{}' already installed at {}",
            skill_id,
            skill_file.display()
        )));
    }

    // Fetch content
    let client = reqwest::Client::new();
    let content = client
        .get(&raw_url)
        .header("User-Agent", "CADE-agent")
        .send()
        .await?
        .error_for_status()?
        .text()
        .await?;

    // Write
    std::fs::create_dir_all(&skill_dir)?;
    std::fs::write(&skill_file, &content)?;

    // Parse and return
    let scope = SkillScope::Project; // installed to project scope by default
    parse_skill(&skill_id, &content, scope, skill_file)
}

// region:    --- Tests

#[cfg(test)]
mod tests {
    #[allow(unused)]
    type Result<T> = core::result::Result<T, Box<dyn std::error::Error>>; // For tests.

    use super::*;
    use std::fs;

    // -- SkillScope ordering

    #[test]
    fn scope_ordering() {
        assert!(SkillScope::Project > SkillScope::Agent);
        assert!(SkillScope::Agent > SkillScope::Global);
        assert!(SkillScope::Global > SkillScope::Builtin);
    }

    #[test]
    fn scope_display() {
        assert_eq!(SkillScope::Builtin.to_string(), "builtin");
        assert_eq!(SkillScope::Global.to_string(), "global");
        assert_eq!(SkillScope::Agent.to_string(), "agent");
        assert_eq!(SkillScope::Project.to_string(), "project");
    }

    // -- Frontmatter parsing

    #[test]
    fn parse_skill_minimal() -> Result<()> {
        let content = "---\nname: Test Skill\ndescription: A test\n---\nBody here.";
        let skill = parse_skill(
            "test-skill",
            content,
            SkillScope::Project,
            PathBuf::from("/fake/SKILL.MD"),
        )
        ?;
        assert_eq!(skill.id, "test-skill");
        assert_eq!(skill.name, "Test Skill");
        assert_eq!(skill.description, "A test");
        assert_eq!(skill.body, "Body here.");
        assert_eq!(skill.scope, SkillScope::Project);

        Ok(())
    }

    #[test]
    fn parse_skill_with_tags_inline() -> Result<()> {
        let content = "---\nname: S\ndescription: D\ntags: [\"rust\", \"testing\"]\n---\nBody";
        let skill = parse_skill("s", content, SkillScope::Global, PathBuf::from("/f"))?;
        assert_eq!(skill.tags, vec!["rust", "testing"]);

        Ok(())
    }

    #[test]
    fn parse_skill_with_tags_multiline() -> Result<()> {
        let content = "---\nname: S\ndescription: D\ntags:\n  - rust\n  - testing\n---\nBody";
        let skill = parse_skill("s", content, SkillScope::Global, PathBuf::from("/f"))?;
        assert_eq!(skill.tags, vec!["rust", "testing"]);

        Ok(())
    }

    #[test]
    fn parse_skill_with_triggers() -> Result<()> {
        let content = "---\nname: S\ndescription: D\ntriggers: [debug, \"fix error\"]\n---\nBody";
        let skill = parse_skill("s", content, SkillScope::Global, PathBuf::from("/f"))?;
        assert_eq!(skill.triggers, vec!["debug", "fix error"]);

        Ok(())
    }

    #[test]
    fn parse_skill_with_tools_block() -> Result<()> {
        let content = "---\nname: S\ndescription: D\ntools:\n  - name: my_tool\n    description: does stuff\n    entrypoint: scripts/my_tool.sh\n---\nBody";
        let skill = parse_skill("s", content, SkillScope::Global, PathBuf::from("/f"))?;
        // tools are parsed but scripts require actual disk files — verify frontmatter parsed
        assert_eq!(skill.body, "Body");

        Ok(())
    }

    #[test]
    fn parse_skill_no_frontmatter() -> Result<()> {
        let content = "Just a body with no frontmatter.";
        let skill = parse_skill("bare", content, SkillScope::Builtin, PathBuf::from("/f"))?;
        assert_eq!(skill.name, "bare"); // falls back to id
        assert_eq!(skill.description, "");
        assert_eq!(skill.body, "Just a body with no frontmatter.");

        Ok(())
    }

    #[test]
    fn parse_skill_rpi_phase() -> Result<()> {
        let content = "---\nname: S\ndescription: D\nrpi_phase: Implement\n---\nBody";
        let skill = parse_skill("s", content, SkillScope::Global, PathBuf::from("/f"))?;
        assert_eq!(skill.rpi_phase.as_deref(), Some("Implement"));

        Ok(())
    }

    // -- Skill::matches_trigger

    fn make_skill(triggers: Vec<&str>) -> Skill {
        Skill {
            id: "test".into(),
            name: "Test".into(),
            description: "".into(),
            category: None,
            tags: vec![],
            triggers: triggers.into_iter().map(String::from).collect(),
            rpi_phase: None,
            capabilities: vec![],
            scripts: vec![],
            references: vec![],
            body: "".into(),
            scope: SkillScope::Project,
            path: PathBuf::from("/f"),
        }
    }

    #[test]
    fn trigger_single_word_exact() {
        let s = make_skill(vec!["debug"]);
        assert!(s.matches_trigger("Help me debug this error"));
        assert!(s.matches_trigger("debug"));
        assert!(!s.matches_trigger("debugging")); // not a word boundary match
    }

    #[test]
    fn trigger_multi_word_substring() -> Result<()> {
        let s = make_skill(vec!["fix error"]);
        assert!(s.matches_trigger("Can you fix error in module?"));
        assert!(!s.matches_trigger("fix the error")); // not exact substring

        Ok(())
    }

    #[test]
    fn trigger_case_insensitive() {
        let s = make_skill(vec!["Debug"]);
        assert!(s.matches_trigger("DEBUG this please"));
        assert!(s.matches_trigger("debug this please"));
    }

    #[test]
    fn trigger_empty_triggers() {
        let s = make_skill(vec![]);
        assert!(!s.matches_trigger("anything"));
    }

    // -- Skill::listing_line

    #[test]
    fn listing_line_format() {
        let mut s = make_skill(vec![]);
        s.category = Some("code".into());
        s.rpi_phase = Some("Implement".into());
        s.description = "A useful skill".into();
        let line = s.listing_line();
        assert!(line.contains("test"));
        assert!(line.contains("[code]"));
        assert!(line.contains("<Implement>"));
        assert!(line.contains("A useful skill"));
    }

    // -- skills_listing

    #[test]
    fn skills_listing_empty() {
        assert!(skills_listing(&[]).is_none());
    }

    #[test]
    fn skills_listing_nonempty() -> Result<()> {
        let s = make_skill(vec![]);
        let listing = skills_listing(&[s]).ok_or("Should produce listing")?;
        assert!(listing.contains("Available Skills"));

        Ok(())
    }

    // -- github_url_to_raw_skill

    #[test]
    fn github_tree_url_conversion() -> Result<()> {
        let url = "https://github.com/user/repo/tree/main/skills/my-skill";
        let raw = github_url_to_raw_skill(url).ok_or("Should convert URL")?;
        assert_eq!(
            raw,
            "https://raw.githubusercontent.com/user/repo/main/skills/my-skill/SKILL.MD"
        );

        Ok(())
    }

    #[test]
    fn github_blob_url_conversion() -> Result<()> {
        let url = "https://github.com/user/repo/blob/main/skills/SKILL.MD";
        let raw = github_url_to_raw_skill(url).ok_or("Should convert URL")?;
        assert!(raw.starts_with("https://raw.githubusercontent.com/"));

        Ok(())
    }

    #[test]
    fn non_github_url_returns_none() {
        assert!(github_url_to_raw_skill("https://example.com/skills").is_none());
    }

    // -- discover_skills_in (filesystem)

    #[test]
    fn discover_skills_empty_dir() -> Result<()> {
        let dir = tempfile::tempdir()?;
        let skills = discover_skills_in(dir.path(), SkillScope::Project);
        assert!(skills.is_empty());

        Ok(())
    }

    #[test]
    fn discover_skills_finds_skill_md() -> Result<()> {
        let dir = tempfile::tempdir()?;
        let skill_dir = dir.path().join("my-skill");
        fs::create_dir_all(&skill_dir)?;
        fs::write(
            skill_dir.join("SKILL.MD"),
            "---\nname: My Skill\ndescription: Test\n---\nBody",
        )
        ?;

        let skills = discover_skills_in(dir.path(), SkillScope::Project);
        assert_eq!(skills.len(), 1);
        assert_eq!(skills[0].id, "my-skill");
        assert_eq!(skills[0].name, "My Skill");
        assert_eq!(skills[0].scope, SkillScope::Project);

        Ok(())
    }

    #[test]
    fn discover_skills_nonexistent_dir() {
        let skills = discover_skills_in(Path::new("/nonexistent/path"), SkillScope::Global);
        assert!(skills.is_empty());
    }

    // -- discover_all_skills merging

    #[test]
    fn discover_all_skills_higher_scope_wins() -> Result<()> {
        let dir = tempfile::tempdir()?;
        let cade_home = tempfile::tempdir()?;

        // Global skill
        let global_dir = cade_home.path().join("skills").join("shared");
        fs::create_dir_all(&global_dir)?;
        fs::write(
            global_dir.join("SKILL.MD"),
            "---\nname: Global Version\ndescription: global\n---\nGlobal body",
        )
        ?;

        // Project skill with same ID
        let proj_dir = dir.path().join(".skills").join("shared");
        fs::create_dir_all(&proj_dir)?;
        fs::write(
            proj_dir.join("SKILL.MD"),
            "---\nname: Project Version\ndescription: project\n---\nProject body",
        )
        ?;

        let skills = discover_all_skills(dir.path(), None, Some(cade_home.path()));
        let shared = skills.iter().find(|s| s.id == "shared").ok_or("Should find skill")?;
        assert_eq!(shared.name, "Project Version"); // project scope wins
        assert_eq!(shared.scope, SkillScope::Project);

        Ok(())
    }
}
