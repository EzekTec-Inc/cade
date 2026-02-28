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

impl Skill {
    /// Format as a concise context block for inclusion in the agent's system prompt
    pub fn to_context_block(&self) -> String {
        let cat = self
            .category
            .as_deref()
            .map(|c| format!("[{c}] "))
            .unwrap_or_default();
        format!(
            "## Skill: {} {cat}\nID: {}\nDescription: {}\n\n{}\n",
            self.name, self.id, self.description, self.body
        )
    }
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
                && e.file_name()
                    .to_str()
                    .map(|n| n.to_uppercase() == "SKILL.MD")
                    .unwrap_or(false)
        })
    {
        let path = entry.path();
        let content = std::fs::read_to_string(path)?;

        let rel = path.strip_prefix(skills_dir).unwrap_or(path);
        let id = rel
            .parent()
            .map(|p| p.to_string_lossy().replace(std::path::MAIN_SEPARATOR, "/"))
            .unwrap_or_default();

        match parse_skill(&id, &content) {
            Ok(skill) => {
                tracing::debug!("Loaded skill: {}", skill.id);
                skills.push(skill);
            }
            Err(e) => tracing::warn!("Failed to parse skill at {}: {e}", path.display()),
        }
    }

    Ok(skills)
}

/// Build a system-prompt block from a list of skills
pub fn skills_context(skills: &[Skill]) -> Option<String> {
    if skills.is_empty() {
        return None;
    }
    let mut out = "# Available Skills\n\n".to_string();
    for skill in skills {
        out.push_str(&skill.to_context_block());
        out.push('\n');
    }
    Some(out)
}

fn parse_skill(id: &str, content: &str) -> Result<Skill> {
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
