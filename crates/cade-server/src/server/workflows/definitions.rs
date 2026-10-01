use super::WorkflowEngine;
use cade_api_types::WorkflowStepDef;
use cade_store::sqlite::{self, AgentRow};
use std::collections::HashSet;

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct WorkflowDef {
    pub name: String,
    #[serde(default)]
    pub description: String,
    pub steps: Vec<WorkflowStepDef>,
}

/// Legacy single-agent webhook configuration remains supported.
#[derive(Debug, serde::Deserialize, serde::Serialize)]
pub struct WorkflowConfig {
    pub name: String,
    pub agent: String,
    #[serde(default)]
    pub model: Option<String>,
    pub prompt: String,
}

pub(super) fn valid_name(name: &str) -> bool {
    !name.is_empty()
        && name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
}

impl WorkflowDef {
    /// Stable topological ordering, preserving original indices for API events.
    pub(super) fn order(&self) -> Result<Vec<usize>, String> {
        if !valid_name(&self.name) || self.steps.is_empty() {
            return Err("Workflow requires a valid name and at least one step".into());
        }
        let mut names = HashSet::new();
        for step in &self.steps {
            if step.name.trim().is_empty() || !names.insert(step.name.clone()) {
                return Err(format!("Empty or duplicate workflow step: {}", step.name));
            }
            if step.prompt.trim().is_empty()
                || step
                    .agent
                    .as_deref()
                    .is_none_or(|agent| agent.trim().is_empty())
            {
                return Err(format!("Step '{}' requires an agent and prompt", step.name));
            }
        }
        for step in &self.steps {
            for dependency in &step.depends_on {
                if !names.contains(dependency) {
                    return Err(format!(
                        "Step '{}' has unknown dependency '{dependency}'",
                        step.name
                    ));
                }
            }
        }
        let mut done = HashSet::new();
        let mut order = Vec::new();
        while order.len() < self.steps.len() {
            let next = self.steps.iter().enumerate().find(|(_, step)| {
                !done.contains(&step.name)
                    && step
                        .depends_on
                        .iter()
                        .all(|dependency| done.contains(dependency))
            });
            let Some((index, step)) = next else {
                return Err("Workflow dependency cycle".into());
            };
            done.insert(step.name.clone());
            order.push(index);
        }
        Ok(order)
    }
}

impl WorkflowEngine {
    pub fn definition(&self, name: &str) -> Result<Option<WorkflowDef>, String> {
        if !valid_name(name) {
            return Err("Invalid workflow name".into());
        }
        if let Some(definition) = Self::builtin_workflows()
            .into_iter()
            .find(|def| def.name == name)
        {
            return Ok(Some(definition));
        }
        let path = self.directory.join(format!("{name}.json"));
        let content = match std::fs::read_to_string(path) {
            Ok(content) => content,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(error) => return Err(error.to_string()),
        };
        let value: serde_json::Value =
            serde_json::from_str(&content).map_err(|error| error.to_string())?;
        let mut def = if value.get("steps").is_some() {
            serde_json::from_value::<WorkflowDef>(value).map_err(|error| error.to_string())?
        } else {
            let config: WorkflowConfig =
                serde_json::from_value(value).map_err(|error| error.to_string())?;
            WorkflowDef {
                name: name.to_owned(),
                description: format!("Custom workflow: {}", config.name),
                steps: vec![WorkflowStepDef {
                    name: "run-agent".into(),
                    agent: Some(if config.model.is_some() {
                        format!("agent-workflow-{name}")
                    } else {
                        config.agent
                    }),
                    prompt: config.prompt,
                    depends_on: vec![],
                }],
            }
        };
        // The requested file identity is canonical for listing, get and dispatch.
        def.name = name.to_owned();
        def.order()?;
        Ok(Some(def))
    }

    pub(super) fn definitions(&self) -> Result<Vec<WorkflowDef>, String> {
        let mut definitions = Self::builtin_workflows();
        let entries = match std::fs::read_dir(&self.directory) {
            Ok(entries) => entries,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(definitions),
            Err(error) => return Err(error.to_string()),
        };
        for entry in entries {
            let path = entry.map_err(|error| error.to_string())?.path();
            if path.extension().is_none_or(|extension| extension != "json") {
                continue;
            }
            let Some(name) = path.file_stem().and_then(|stem| stem.to_str()) else {
                continue;
            };
            if !valid_name(name) || definitions.iter().any(|def| def.name == name) {
                continue;
            }
            if let Some(definition) = self.definition(name)? {
                definitions.push(definition);
            }
        }
        definitions.sort_by(|left, right| left.name.cmp(&right.name));
        Ok(definitions)
    }

    /// Preserve legacy model-configured webhook agents without inventing a model.
    pub fn prepare_legacy_agent(&self, name: &str) -> Result<(), String> {
        if !valid_name(name) {
            return Err("Invalid workflow name".into());
        }
        if Self::builtin_workflows().iter().any(|def| def.name == name) {
            return Ok(());
        }
        let content = std::fs::read_to_string(self.directory.join(format!("{name}.json")))
            .map_err(|error| error.to_string())?;
        let value: serde_json::Value =
            serde_json::from_str(&content).map_err(|error| error.to_string())?;
        if value.get("steps").is_some() {
            return Ok(());
        }
        let config: WorkflowConfig =
            serde_json::from_value(value).map_err(|error| error.to_string())?;
        let Some(model) = config.model else {
            return Ok(());
        };
        if model.trim().is_empty() {
            return Err("Legacy workflow model cannot be empty".into());
        }
        let id = format!("agent-workflow-{name}");
        if sqlite::get_agent(&self.db, &id)
            .map_err(|error| error.to_string())?
            .is_none()
        {
            sqlite::create_agent(
                &self.db,
                &AgentRow {
                    id,
                    name: config.agent,
                    model,
                    system_prompt: Some(config.prompt),
                    description: Some(format!("Automated workflow agent for '{name}'")),
                    created_at: None,
                    compaction_model: None,
                    theme: None,
                    active_plan_json: None,
                    parent_id: None,
                },
            )
            .map_err(|error| error.to_string())?;
        }
        Ok(())
    }

    pub fn builtin_workflows() -> Vec<WorkflowDef> {
        let step =
            |name: &str, agent: &str, prompt: &str, dependency: Option<&str>| WorkflowStepDef {
                name: name.into(),
                agent: Some(agent.into()),
                prompt: prompt.into(),
                depends_on: dependency.into_iter().map(String::from).collect(),
            };
        vec![
            WorkflowDef {
                name: "ci-validation".into(),
                description: "Run cargo check, clippy -- -D warnings, and test suite verification"
                    .into(),
                steps: vec![
                    step(
                        "cargo-check",
                        "worker",
                        "Run cargo check --all-targets",
                        None,
                    ),
                    step(
                        "cargo-clippy",
                        "reviewer",
                        "Run cargo clippy --all-targets -- -D warnings",
                        Some("cargo-check"),
                    ),
                    step(
                        "cargo-test",
                        "tester",
                        "Run cargo test --workspace",
                        Some("cargo-clippy"),
                    ),
                ],
            },
            WorkflowDef {
                name: "dependency-audit".into(),
                description: "Audit workspace dependencies for security vulnerabilities".into(),
                steps: vec![step("cargo-audit", "security", "Run cargo audit", None)],
            },
        ]
    }
}
