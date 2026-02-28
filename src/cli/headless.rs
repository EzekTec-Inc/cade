use anyhow::Result;
use crate::agent::LettaClient;
use crate::permissions::PermissionManager;

pub async fn run_headless(
    client: &LettaClient,
    agent_id: &str,
    prompt: &str,
    permissions: &PermissionManager,
) -> Result<String> {
    tracing::debug!("headless: agent={agent_id} prompt='{prompt}'");

    let messages = client.send_message(agent_id, prompt).await?;

    let mut output = String::new();
    for msg in &messages {
        if let Some(content) = msg.data.get("content") {
            if let Some(text) = content.as_str() {
                if !text.is_empty() {
                    output.push_str(text);
                    output.push('\n');
                }
            }
        }
        // Handle tool calls in headless mode
        if let Some(tool_name) = msg.data.get("tool_name").and_then(|v| v.as_str()) {
            if permissions.is_blocked(tool_name) {
                tracing::warn!("Tool '{tool_name}' blocked by permission mode");
                continue;
            }
            // Tool execution is handled server-side via registered Letta tools
            // Local tool dispatch will be added in Phase 2
        }
    }

    Ok(output.trim().to_string())
}
