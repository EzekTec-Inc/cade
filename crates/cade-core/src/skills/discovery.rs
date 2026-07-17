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
        .filter_map(|e| e.ok());

    for entry in walker {
        let path = entry.path();
        if !path.is_file() {
            continue;
        }

        let file_name = path.file_name().and_then(|n| n.to_str()).unwrap_or("");
        let file_name_upper = file_name.to_uppercase();

        let is_skill_md = file_name_upper == "SKILL.MD";
        let is_direct_md = !is_skill_md
            && path.extension().map(|ext| ext == "md").unwrap_or(false)
            && path.parent() == Some(dir);

        if is_skill_md {
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
        } else if is_direct_md {
            let content = match std::fs::read_to_string(path) {
                Ok(c) => c,
                Err(e) => {
                    tracing::warn!("Cannot read {}: {e}", path.display());
                    continue;
                }
            };
            let id = path
                .file_stem()
                .and_then(|s| s.to_str())
                .unwrap_or("")
                .to_string();
            if id.is_empty() {
                continue;
            }

            match parse_skill(&id, &content, scope, path.to_path_buf()) {
                Ok(s) => {
                    tracing::debug!("Loaded direct root skill: {} [{}]", s.id, s.scope);
                    skills.push(s);
                }
                Err(e) => tracing::warn!("Bad skill at {}: {e}", path.display()),
            }
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
        "<available_skills>\n\
         Use the standard file `read` tool to load a skill's full content (SKILL.md) or its references directly from their listed paths. Use the standard shell/bash execution tool to run any scripts under scripts/ directly.\n\n"
    );
    for s in skills {
        out.push_str("  <skill>\n");
        out.push_str(&format!("    <id>{}</id>\n", s.id));
        out.push_str(&format!("    <name>{}</name>\n", s.name));
        out.push_str(&format!("    <scope>{}</scope>\n", s.scope));
        out.push_str(&format!("    <path>{}</path>\n", s.path.display()));
        out.push_str(&format!(
            "    <description>{}</description>\n",
            s.description
        ));
        if !s.scripts.is_empty() {
            out.push_str("    <scripts>\n");
            for script in &s.scripts {
                out.push_str(&format!(
                    "      <script path=\"{}\">{}</script>\n",
                    script.path.display(),
                    script.name
                ));
            }
            out.push_str("    </scripts>\n");
        }
        if !s.references.is_empty() {
            out.push_str("    <references>\n");
            for r in &s.references {
                out.push_str(&format!(
                    "      <reference path=\"{}\">{}</reference>\n",
                    r.path.display(),
                    r.name
                ));
            }
            out.push_str("    </references>\n");
        }
        out.push_str("  </skill>\n");
    }
    out.push_str("</available_skills>\n");
    Some(out)
}
