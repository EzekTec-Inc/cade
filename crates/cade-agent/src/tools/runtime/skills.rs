use super::*;
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

        match self
            .storage
            .install_plugin(&self.agent_id, &url, &plugin_id)
            .await
        {
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

    pub(crate) async fn handle_run_skill_script(&self, _args: &Value) -> (String, bool) {
        (
            "run_skill_script is deprecated and removed from your schema. Please use the standard `bash` tool to execute scripts directly from their path instead.".to_string(),
            true,
        )
    }

    pub(crate) fn handle_load_skill_ref(&self, _args: &Value) -> (String, bool) {
        (
            "load_skill_ref is deprecated and removed from your schema. Please use the standard `read` tool to read reference documents directly from their path instead.".to_string(),
            true,
        )
    }
}
