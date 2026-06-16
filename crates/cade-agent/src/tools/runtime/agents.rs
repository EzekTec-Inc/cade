use super::*;
use serde_json::Value;

impl ToolRuntime {
    pub(crate) async fn handle_message_agent(&self, args: &Value) -> (String, bool) {
        let target = args["target"].as_str().unwrap_or("").trim().to_string();
        let message = args["message"].as_str().unwrap_or("").to_string();

        if target.is_empty() || message.is_empty() {
            return (
                "Error: 'target' and 'message' are required".to_string(),
                true,
            );
        }

        // Try to resolve target to an agent ID
        let target_id = match self.storage.list_agents().await {
            Ok(agents) => {
                if let Some(agent) = agents.iter().find(|a| a.id == target || a.name == target) {
                    agent.id.clone()
                } else {
                    return (format!("Error: Agent '{target}' not found"), true);
                }
            }
            Err(e) => return (format!("Failed to query agents: {e}"), true),
        };

        match self
            .storage
            .message_agent(&self.agent_id, &target_id, &message)
            .await
        {
            Ok(response) => {
                if response.trim().is_empty() {
                    ("Target agent returned an empty response".to_string(), false)
                } else {
                    (response.trim().to_string(), false)
                }
            }
            Err(e) => (format!("Failed to message agent: {e}"), true),
        }
    }
}
