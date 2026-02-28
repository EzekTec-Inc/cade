use anyhow::Result;
use serde::{Deserialize, Serialize};
use std::path::Path;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Skill {
    pub id: String,
    pub name: String,
    pub description: String,
    pub category: Option<String>,
    pub tags: Vec<String>,
    pub body: String,
}

/// Parsed frontmatter fields from SKILL.MD
#[derive(Default)]
struct Frontmatter {
    name: Option<String>,
    description: Option<String>,
    category: Option<String>,
    tags: Vec<String>,
}

/// Scan a directory recursively for SKILL.MD files and parse them.
pub fn discover_skills(skills_dir: &Path) -> Result<Vec<Skill>> {
    if !skills_dir.exists() {
        return Ok(Vec::new());
    }

    let mut skills = Vec::new();

    for entry in walkdir::WalkDir::new(skills_dir)
        .into_iter()
        .filter_map(|e| e.ok())
        .filter(|e| {
            e.file_type().is_file()
                && e.file_name().to_str()
                    .map(|n| n.to_uppercase() == "SKILL.MD")
                    .unwrap_or(false)
        })
    {
        let path = entry.path();
        let content = std::fs::read_to_string(path)?;

        // Derive skill ID from directory path relative to skills_dir
        let rel = path.strip_prefix(skills_dir).unwrap();
        let id = rel
            .parent()
            .map(|p| p.to_string_lossy().replace(std::path::MAIN_SEPARATOR, "/"))
            .unwrap_or_default();

        match parse_skill(&id, &content) {
            Ok(skill) => skills.push(skill),
            Err(e) => tracing::warn!("Failed to parse skill at {}: {e}", path.display()),
        }
    }

    Ok(skills)
}

fn parse_skill(id: &str, content: &str) -> Result<Skill> {
    let content = content.trim();
    let (frontmatter_str, body) = if content.starts_with("---") {
        let end = content[3..].find("---").map(|i| i + 3).unwrap_or(0);
        if end > 0 {
            (&content[3..end], &content[end + 3..])
        } else {
            ("", content)
        }
    } else {
        ("", content)
    };

    let fm = parse_frontmatter(frontmatter_str);

    Ok(Skill {
        id: id.to_string(),
        name: fm.name.unwrap_or_else(|| id.to_string()),
        description: fm.description.unwrap_or_default(),
        category: fm.category,
        tags: fm.tags,
        body: body.trim().to_string(),
    })
}

fn parse_frontmatter(fm: &str) -> Frontmatter {
    let mut out = Frontmatter::default();
    for line in fm.lines() {
        let line = line.trim();
        if let Some((key, val)) = line.split_once(':') {
            let key = key.trim();
            let val = val.trim().trim_matches('"').trim_matches('\'');
            match key {
                "name" => out.name = Some(val.to_string()),
                "description" => out.description = Some(val.to_string()),
                "category" => out.category = Some(val.to_string()),
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
