// region:    --- Modules

use crate::Result;
use std::path::{Path, PathBuf};

// endregion: --- Modules

use super::types::*;
// -- Parsing

pub fn parse_skill(id: &str, content: &str, scope: SkillScope, path: PathBuf) -> Result<Skill> {
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
