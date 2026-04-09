use super::*;
use std::path::PathBuf;
use std::sync::Arc;
use serde_json::Value;
use cade_core::skills::discover_all_skills;
use cade_core::tool_ids::*;
use crate::agent::client::HttpTransport;
use crate::backends::{ExecutionBackend, LocalBackend};
use crate::mcp::McpManager;
use crate::tools::git_checkpoint;
use crate::tools::{dispatch, memory};

impl ToolRuntime {
    async fn handle_message_agent(&self, args: &Value) -> (String, bool) {
        let target = args["target"].as_str().unwrap_or("").trim().to_string();
        let message = args["message"].as_str().unwrap_or("").to_string();

        if target.is_empty() || message.is_empty() {
            return (
                "Error: 'target' and 'message' are required".to_string(),
                true,
            );
        }

        // Try to resolve target to an agent ID
        let target_id = match self.client.list_agents().await {
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
            .client
            .stream_message(&target_id, &message, |_| {})
            .await
        {
            Ok(messages) => {
                // Ensure we get all tool outputs if it used tools
                let mut out = String::new();
                for msg in messages {
                    if let Some(text) = msg.assistant_text()
                        && !text.is_empty()
                    {
                        out.push_str(text);
                    }
                }
                if out.trim().is_empty() {
                    ("Target agent returned an empty response".to_string(), false)
                } else {
                    (out.trim().to_string(), false)
                }
            }
            Err(e) => (format!("Failed to message agent: {e}"), true),
        }
    }

}
