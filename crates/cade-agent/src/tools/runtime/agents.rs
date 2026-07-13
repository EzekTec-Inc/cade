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

    pub(crate) async fn handle_search_tools(&self, args: &Value) -> (String, bool) {
        let query = args["query"].as_str().unwrap_or("").trim().to_lowercase();
        if query.is_empty() {
            return ("Error: 'query' cannot be empty".to_string(), true);
        }

        let schemas = self.mcp.all_tool_schemas().await;
        let mut matched = Vec::new();

        for s in schemas {
            let name = s["name"].as_str().unwrap_or("").to_lowercase();
            let desc = s["description"].as_str().unwrap_or("").to_lowercase();
            if name.contains(&query) || desc.contains(&query) {
                matched.push(s);
            }
        }

        if matched.is_empty() {
            return (
                format!("No third-party MCP tools matched search query '{query}'. Try searching for a different keyword or capability."),
                false,
            );
        }

        let mut out = format!(
            "Found {} matching third-party MCP tool(s) for '{query}':\n\n",
            matched.len()
        );

        for s in matched {
            let name = s["name"].as_str().unwrap_or("?");
            let desc = s["description"]
                .as_str()
                .unwrap_or("No description provided.");
            let params = s.get("parameters").cloned().unwrap_or(Value::Object(Default::default()));

            out.push_str(&format!("### `{name}`\n"));
            out.push_str(&format!("**Description:** {desc}\n"));

            if let Some(obj) = params.as_object() {
                if let Some(props) = obj.get("properties").and_then(|p| p.as_object()) {
                    if !props.is_empty() {
                        out.push_str("**Parameters:**\n");
                        let mut sorted_props: Vec<(&String, &Value)> = props.iter().collect();
                        sorted_props.sort_by_key(|(k, _)| k.as_str());

                        for (p_name, p_schema) in sorted_props {
                            let p_type = p_schema["type"].as_str().unwrap_or("any");
                            let p_desc = p_schema["description"].as_str().unwrap_or("No description");
                            let req_list = obj.get("required").and_then(|r| r.as_array());
                            let is_required = req_list
                                .map(|arr| arr.iter().any(|v| v.as_str() == Some(p_name.as_str())))
                                .unwrap_or(false);
                            let req_str = if is_required { " *(required)*" } else { "" };
                            out.push_str(&format!("  - `{p_name}` ({p_type}){req_str}: {p_desc}\n"));
                        }
                    }
                }
            }
            out.push_str("\n");
        }

        (out.trim_end().to_string(), false)
    }
}
