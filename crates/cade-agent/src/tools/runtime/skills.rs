use super::*;
use cade_core::skills::discover_all_skills;
use serde_json::Value;

impl ToolRuntime {
    pub(crate) async fn handle_install_plugin(&self, args: &Value) -> (String, bool) {
        let url = args["url"].as_str().unwrap_or("").trim().to_string();
        let plugin_id = args["plugin_id"].as_str().unwrap_or("").trim().to_string();

        if url.is_empty() || plugin_id.is_empty() {
            return (
                "Error: 'url' and 'plugin_id' are required".to_string(),
                true,
            );
        }

        match self.storage.install_plugin(&self.agent_id, &url, &plugin_id).await {
            Ok(msg) => (msg, false),
            Err(e) => (format!("Failed to install plugin: {e}"), true),
        }
    }

    pub(crate) async fn handle_install_skill(&self, args: &Value) -> (String, bool) {
        let url = args["url"].as_str().unwrap_or("").trim().to_string();
        let scope = args["scope"].as_str().unwrap_or("project");
        let skill_name = args["skill"]
            .as_str()
            .map(|s| s.trim())
            .filter(|s| !s.is_empty());
        if url.is_empty() {
            return ("Error: 'url' is required".to_string(), true);
        }
        let target_dir = if scope == "global" {
            dirs::home_dir()
                .map(|h| h.join(".cade").join("skills"))
                .unwrap_or_else(|| self.cwd.join(".cade/skills"))
        } else {
            self.cwd.join(".cade/skills")
        };
        match cade_core::skills::install_skill_from_url(&url, &target_dir, skill_name).await {
            Ok(skill) => (
                format!(
                    "Skill '{}' installed as [{}] in {} scope. It is now available via load_skill(\"{}\").",
                    skill.name, skill.id, scope, skill.id
                ),
                false,
            ),
            Err(e) => (format!("Failed to install skill: {e}"), true),
        }
    }

    pub(crate) async fn handle_run_skill_script(&self, args: &Value) -> (String, bool) {
        let skill_id = args["skill_id"].as_str().unwrap_or("").trim().to_string();
        let script = args["script"].as_str().unwrap_or("").trim().to_string();
        let script_args: Vec<String> = args["args"]
            .as_array()
            .map(|a| {
                a.iter()
                    .filter_map(|v| v.as_str().map(String::from))
                    .collect()
            })
            .unwrap_or_default();

        if skill_id.is_empty() || script.is_empty() {
            return (
                "Error: 'skill_id' and 'script' are required".to_string(),
                true,
            );
        }

        let skills = discover_all_skills(&self.cwd, Some(&self.agent_id), None);
        let Some(skill) = skills.into_iter().find(|s| s.id == skill_id) else {
            return (format!("Skill '{skill_id}' not found"), true);
        };

        let Some(sk) = skill.scripts.iter().find(|s| s.name == script).cloned() else {
            let available: Vec<&str> = skill.scripts.iter().map(|s| s.name.as_str()).collect();
            let list = if available.is_empty() {
                "none".to_string()
            } else {
                available.join(", ")
            };
            return (
                format!("Script '{script}' not found in skill '{skill_id}'. Available: {list}"),
                true,
            );
        };

        let mut cmd = tokio::process::Command::new(&sk.path);
        cade_core::agent_env::apply_agent_env(&mut cmd);
        cade_core::askpass::apply_askpass_env(&mut cmd);
        match cmd.args(&script_args).output().await {
            Err(e) => (format!("Failed to run script: {e}"), true),
            Ok(out) => {
                let stdout = String::from_utf8_lossy(&out.stdout).to_string();
                let stderr = String::from_utf8_lossy(&out.stderr).to_string();
                let combined = if stderr.is_empty() {
                    stdout
                } else {
                    format!("{stdout}\n[stderr]\n{stderr}")
                };
                let is_err = !out.status.success();
                (combined, is_err)
            }
        }
    }

    pub(crate) fn handle_load_skill_ref(&self, args: &Value) -> (String, bool) {
        let skill_id = args["skill_id"].as_str().unwrap_or("").trim().to_string();
        let doc = args["doc"].as_str().unwrap_or("").trim().to_string();

        if skill_id.is_empty() || doc.is_empty() {
            return ("Error: 'skill_id' and 'doc' are required".to_string(), true);
        }

        let skills = discover_all_skills(&self.cwd, Some(&self.agent_id), None);
        let Some(skill) = skills.into_iter().find(|s| s.id == skill_id) else {
            return (format!("Skill '{skill_id}' not found"), true);
        };

        let Some(r) = skill
            .references
            .iter()
            .find(|r| {
                r.name == doc || r.path.file_name().and_then(|n| n.to_str()).unwrap_or("") == doc
            })
            .cloned()
        else {
            let available: Vec<&str> = skill.references.iter().map(|r| r.name.as_str()).collect();
            let list = if available.is_empty() {
                "none".to_string()
            } else {
                available.join(", ")
            };
            return (
                format!("Reference '{doc}' not found in skill '{skill_id}'. Available: {list}"),
                true,
            );
        };

        match std::fs::read_to_string(&r.path) {
            Ok(content) => (
                format!("# Reference: {doc} (skill: {skill_id})\n\n{content}"),
                false,
            ),
            Err(e) => (format!("Failed to read reference '{doc}': {e}"), true),
        }
    }
}
