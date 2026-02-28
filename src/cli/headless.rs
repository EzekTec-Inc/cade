use anyhow::Result;

use crate::agent::{LettaClient, client::LettaMessage};
use crate::permissions::PermissionManager;
use crate::tools::dispatch;

/// Run a single headless prompt, driving the tool loop to completion.
/// Returns the final assistant text.
pub async fn run_headless(
    client: &LettaClient,
    agent_id: &str,
    prompt: &str,
    permissions: &PermissionManager,
) -> Result<String> {
    tracing::debug!("headless: agent={agent_id}");

    let messages = client.send_message(agent_id, prompt).await?;
    let mut output = String::new();

    process_messages(client, agent_id, messages, permissions, &mut output).await?;

    Ok(output.trim().to_string())
}

async fn process_messages(
    client: &LettaClient,
    agent_id: &str,
    messages: Vec<LettaMessage>,
    permissions: &PermissionManager,
    output: &mut String,
) -> Result<()> {
    let mut tool_calls: Vec<(String, String, serde_json::Value)> = Vec::new();

    for msg in &messages {
        match msg.msg_type() {
            "assistant_message" => {
                if let Some(text) = msg.assistant_text() {
                    if !text.is_empty() {
                        output.push_str(text);
                        output.push('\n');
                    }
                }
            }
            "tool_call_message" => {
                if let Some(tc) = msg.as_tool_call() {
                    tool_calls.push(tc);
                }
            }
            _ => {}
        }
    }

    for (call_id, tool_name, args) in tool_calls {
        if permissions.is_blocked(&tool_name) {
            tracing::warn!("Tool '{tool_name}' blocked (plan mode)");
            let follow = client
                .send_tool_return(agent_id, &call_id, &format!("Tool '{tool_name}' blocked"), true)
                .await?;
            Box::pin(process_messages(client, agent_id, follow, permissions, output)).await?;
            continue;
        }

        tracing::info!("Executing tool: {tool_name}");
        let result = dispatch(call_id.clone(), &tool_name, &args).await;
        tracing::debug!("Tool result ({}): {}", tool_name, super::truncate(&result.output, 200));

        let follow = client
            .send_tool_return(agent_id, &call_id, &result.output, result.is_error)
            .await?;
        Box::pin(process_messages(client, agent_id, follow, permissions, output)).await?;
    }

    Ok(())
}
