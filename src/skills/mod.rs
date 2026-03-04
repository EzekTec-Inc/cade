use anyhow::Result;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

// ── Skill scope ───────────────────────────────────────────────────────────────

/// Where a skill was loaded from. Higher priority scopes override lower ones
/// when two skills share the same ID.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub enum SkillScope {
    /// Built-in skills shipped with CADE
    Builtin  = 0,
    /// Machine-global skills in ~/.cade/skills/
    Global   = 1,
    /// Agent-scoped skills in ~/.cade/agents/{id}/skills/
    Agent    = 2,
    /// Project-scoped skills in <cwd>/.skills/  (highest priority)
    Project  = 3,
}

impl std::fmt::Display for SkillScope {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Builtin => write!(f, "builtin"),
            Self::Global  => write!(f, "global"),
            Self::Agent   => write!(f, "agent"),
            Self::Project => write!(f, "project"),
        }
    }
}

// ── Skill ─────────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Skill {
    pub id: String,
    pub name: String,
    pub description: String,
    pub category: Option<String>,
    pub tags: Vec<String>,
    pub body: String,
    pub scope: SkillScope,
    /// Absolute path to the SKILL.MD file
    pub path: PathBuf,
}

impl Skill {
    /// One-line entry for the skills listing injected into the system prompt.
    pub fn listing_line(&self) -> String {
        let cat = self.category.as_deref()
            .map(|c| format!(" [{c}]"))
            .unwrap_or_default();
        format!("- {} [{}]{}: {}", self.id, self.scope, cat, self.description)
    }

    /// Full formatted block returned by `load_skill` tool.
    pub fn to_context_block(&self) -> String {
        let cat = self.category.as_deref()
            .map(|c| format!("[{c}] "))
            .unwrap_or_default();
        format!(
            "## Skill: {} {cat}\nID: {}\nScope: {}\nDescription: {}\n\n{}\n",
            self.name, self.id, self.scope, self.description, self.body
        )
    }
}

// ── Discovery ─────────────────────────────────────────────────────────────────

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
            Err(e) => { tracing::warn!("Cannot read {}: {e}", path.display()); continue; }
        };
        let rel = path.strip_prefix(dir).unwrap_or(path);
        let id = rel.parent()
            .map(|p| p.to_string_lossy().replace(std::path::MAIN_SEPARATOR, "/"))
            .unwrap_or_default();
        if id.is_empty() { continue; }

        match parse_skill(&id, &content, scope, path.to_path_buf()) {
            Ok(s) => { tracing::debug!("Loaded skill: {} [{}]", s.id, s.scope); skills.push(s); }
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
    if let Some(ref ch) = cade_home {
        all.extend(discover_skills_in(&ch.join("skills"), SkillScope::Global));
        if let Some(id) = agent_id {
            all.extend(discover_skills_in(
                &ch.join("agents").join(id).join("skills"),
                SkillScope::Agent,
            ));
        }
    }
    all.extend(discover_skills_in(&cwd.join(".skills"), SkillScope::Project));

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
         Use the `load_skill` tool to load a skill's full content when working on a relevant task.\n\n"
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
    if skills.is_empty() { return None; }
    let mut out = "# Available Skills\n\n".to_string();
    for s in skills {
        out.push_str(&s.to_context_block());
        out.push('\n');
    }
    Some(out)
}

// ── Parsing ───────────────────────────────────────────────────────────────────

fn parse_skill(id: &str, content: &str, scope: SkillScope, path: PathBuf) -> Result<Skill> {
    let content = content.trim();
    let (fm_str, body) = if content.starts_with("---") {
        match content[3..].find("---") {
            Some(end) => (&content[3..end + 3], &content[end + 6..]),
            None => ("", content),
        }
    } else {
        ("", content)
    };

    let fm = parse_frontmatter(fm_str);
    Ok(Skill {
        id: id.to_string(),
        name: fm.name.unwrap_or_else(|| id.to_string()),
        description: fm.description.unwrap_or_default(),
        category: fm.category,
        tags: fm.tags,
        body: body.trim().to_string(),
        scope,
        path,
    })
}

#[derive(Default)]
struct Frontmatter {
    name: Option<String>,
    description: Option<String>,
    category: Option<String>,
    tags: Vec<String>,
}

fn parse_frontmatter(fm: &str) -> Frontmatter {
    let mut out = Frontmatter::default();
    for line in fm.lines() {
        let line = line.trim();
        if let Some((key, val)) = line.split_once(':') {
            let key = key.trim();
            let val = val.trim().trim_matches('"').trim_matches('\'');
            match key {
                "name"        => out.name = Some(val.to_string()),
                "description" => out.description = Some(val.to_string()),
                "category"    => out.category = Some(val.to_string()),
                "tags" => {
                    out.tags = val
                        .trim_matches(|c| c == '[' || c == ']')
                        .split(',')
                        .map(|s| s.trim().trim_matches('"').to_string())
                        .filter(|s| !s.is_empty())
                        .collect()
                }
                _ => {}
            }
        }
    }
    out
}

// ── Installation ──────────────────────────────────────────────────────────────

/// Convert a GitHub tree URL to a raw SKILL.MD URL.
/// https://github.com/USER/REPO/tree/BRANCH/path → https://raw.../path/SKILL.MD
pub fn github_url_to_raw_skill(url: &str) -> Option<String> {
    // Match: https://github.com/<user>/<repo>/tree/<branch>/<path...>
    let stripped = url
        .trim_start_matches("https://github.com/")
        .trim_start_matches("http://github.com/");
    let parts: Vec<&str> = stripped.splitn(5, '/').collect();
    if parts.len() >= 5 && parts[2] == "tree" {
        let user   = parts[0];
        let repo   = parts[1];
        let branch = parts[3];
        let path   = parts[4];
        Some(format!(
            "https://raw.githubusercontent.com/{user}/{repo}/{branch}/{path}/SKILL.MD"
        ))
    } else if parts.len() >= 4 && parts[2] == "blob" {
        // Direct file URL — return as-is converted to raw
        let branch = parts[3];
        let path   = parts[4..].join("/");
        Some(format!(
            "https://raw.githubusercontent.com/{}/{}/{}/{}",
            parts[0], parts[1], branch, path
        ))
    } else {
        None
    }
}

/// Download and install a skill from a URL into `target_dir/<skill-name>/SKILL.MD`.
/// Returns the installed skill on success.
pub async fn install_skill_from_url(
    url: &str,
    target_dir: &Path,
) -> Result<Skill> {
    // Resolve to raw content URL if needed
    let raw_url = if url.contains("github.com") && url.contains("/tree/") {
        github_url_to_raw_skill(url)
            .ok_or_else(|| anyhow::anyhow!("Cannot parse GitHub URL: {url}"))?
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

    let skill_dir = target_dir.join(&skill_id);
    let skill_file = skill_dir.join("SKILL.MD");

    if skill_file.exists() {
        anyhow::bail!("Skill '{}' already installed at {}", skill_id, skill_file.display());
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
