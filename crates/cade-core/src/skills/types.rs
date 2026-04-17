// region:    --- Modules

use serde::{Deserialize, Serialize};
use std::path::PathBuf;

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
    /// Agent-scoped skills in ~/.cade/subagents/{id}/skills/
    Agent = 2,
    /// Project-scoped skills in <cwd>/.cade/skills/  (highest priority)
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

impl SkillScope {
    /// Stable ordering used by UI listings (project first, then agent, global, builtin).
    /// This is the inverse of priority resolution — the highest-priority scope shows
    /// at the top of the list. Keep UI and command layers in sync via this one method.
    pub fn display_order(&self) -> u8 {
        match self {
            Self::Project => 0,
            Self::Agent => 1,
            Self::Global => 2,
            Self::Builtin => 3,
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
        // Truncate description to save context tokens — full text available via load_skill.
        let desc = if self.description.len() > 80 {
            let end = self
                .description
                .char_indices()
                .nth(80)
                .map(|(i, _)| i)
                .unwrap_or(self.description.len());
            format!("{}…", &self.description[..end])
        } else {
            self.description.clone()
        };
        format!(
            "- {} [{}]{}{}: {}",
            self.id, self.scope, cat, phase, desc
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

