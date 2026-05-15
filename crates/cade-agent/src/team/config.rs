use super::discovery::TeamDef;
use super::mode::TeamMode;
use crate::agent::client::MemoryBlock;
use serde_json::Value;
const SEED_BLOCK_MAX_CHARS: usize = 1500;
const SKIP_LABELS: &[&str] = &["active_goal", "session_summary"];
#[derive(Debug, Clone)]
pub struct TeamConfig {
    pub task: String,
    pub team_id: String,
    pub mode_override: Option<TeamMode>,
    pub background: bool,
    pub model_override: Option<String>,
    pub custom_system_prompt: Option<String>,
    pub description: Option<String>,
    pub test_command: Option<String>,
    pub human_review: bool,
    pub silent_stream: bool,
    pub max_iterations: Option<usize>,
    pub depth: usize,
    pub max_tokens_budget: Option<u64>,
}
impl TeamConfig {
    pub fn from_args(args: &Value) -> Self {
        Self {
            task: args["task"]
                .as_str()
                .or_else(|| args["prompt"].as_str())
                .unwrap_or("")
                .trim()
                .to_string(),
            team_id: args["team"]
                .as_str()
                .or_else(|| args["team_id"].as_str())
                .unwrap_or("default")
                .trim()
                .to_string(),
            mode_override: args["mode"].as_str().and_then(TeamMode::from_str),
            background: args["background"].as_bool().unwrap_or(false),
            model_override: args["model"]
                .as_str()
                .map(|s| s.trim().to_string())
                .filter(|s| !s.is_empty()),
            custom_system_prompt: args["system_prompt"]
                .as_str()
                .map(|s| s.trim().to_string())
                .filter(|s| !s.is_empty()),
            description: args["description"]
                .as_str()
                .map(|s| s.trim().to_string())
                .filter(|s| !s.is_empty()),
            test_command: args["test_command"]
                .as_str()
                .map(|s| s.trim().to_string())
                .filter(|s| !s.is_empty()),
            human_review: args["human_review"].as_bool().unwrap_or(false),
            silent_stream: args["silent_stream"].as_bool().unwrap_or(false),
            max_iterations: args["max_iterations"].as_u64().map(|n| n as usize),
            depth: args["_team_depth"]
                .as_u64()
                .map(|n| n as usize)
                .unwrap_or(0),
            max_tokens_budget: args["max_tokens_budget"].as_u64(),
        }
    }
    pub fn validate(&self) -> Result<(), String> {
        if self.task.trim().is_empty() {
            Err("error: 'task' is required".into())
        } else {
            Ok(())
        }
    }
    pub fn resolve_mode(&self, def: Option<&TeamDef>) -> TeamMode {
        self.mode_override
            .or_else(|| def.map(|d| d.mode))
            .unwrap_or(TeamMode::Coordinate)
    }
    pub fn resolve_model<'a>(&'a self, def: Option<&'a TeamDef>) -> Option<&'a str> {
        self.model_override
            .as_deref()
            .or_else(|| def.and_then(|d| d.leader_model.as_deref()))
    }
    pub fn resolve_max_iterations(&self, def: Option<&TeamDef>) -> usize {
        self.max_iterations
            .or_else(|| def.map(|d| d.max_iterations))
            .unwrap_or(10)
    }
    pub fn task_with_test_command(&self) -> String {
        match &self.test_command {
            Some(cmd) => format!(
                "{}\n\nCRITICAL PROOF OF WORK: Run `{cmd}` to verify.",
                self.task
            ),
            None => self.task.clone(),
        }
    }
    pub fn ephemeral_agent_name(&self, id: &str) -> String {
        format!("team-{}-{}", self.team_id, id)
    }
    pub fn ephemeral_description(&self) -> String {
        self.description
            .clone()
            .unwrap_or_else(|| format!("Team run: {}", self.team_id))
    }
    pub fn build_seed_memory(blocks: Vec<MemoryBlock>) -> Vec<MemoryBlock> {
        blocks
            .into_iter()
            .filter(|b| {
                !b.label.starts_with("__")
                    && !SKIP_LABELS.contains(&b.label.as_str())
                    && b.tier
                        .as_deref()
                        .is_none_or(|t| t == "pinned" || t == "short")
                    && !b.value.trim().is_empty()
            })
            .map(|b| MemoryBlock {
                label: b.label,
                value: cap(&b.value, SEED_BLOCK_MAX_CHARS),
                description: b.description,
                tier: None,
            })
            .collect()
    }
    pub fn format_seed_section(blocks: &[MemoryBlock]) -> String {
        if blocks.is_empty() {
            return String::new();
        }
        let mut o = String::from("\n\n# Inherited memory\n");
        for b in blocks {
            if !b.value.trim().is_empty() {
                o.push_str(&format!("\n## {}\n{}\n", b.label, b.value));
            }
        }
        o
    }
}
fn cap(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        s.to_string()
    } else {
        let e = s.char_indices().nth(max).map(|(i, _)| i).unwrap_or(s.len());
        format!("{}…", &s[..e])
    }
}
