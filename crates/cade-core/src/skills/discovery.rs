// region:    --- Modules

use std::path::Path;

// endregion: --- Modules

use super::parsing::*;
use super::types::*;
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
                &ch.join("subagents").join(id).join("skills"),
                SkillScope::Agent,
            ));
        }
    }
    all.extend(discover_skills_in(
        &cwd.join(".cade/skills"),
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
