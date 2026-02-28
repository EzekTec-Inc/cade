use anyhow::Result;

use crate::agent::{LettaClient, client::LettaMessage};
use crate::permissions::PermissionManager;
use crate::tools::dispatch;

/// Run a single headless prompt with streaming, driving the tool loop to completion.
/// Prints streaming output to stdout. Returns the final assistant text.
pub async fn run_headless(
    client: &LettaClient,
    agent_id: &str,
    prompt: &str,
    permissions: &PermissionManager,
) -> Result<String> {
    tracing::debug!("headless: agent={agent_id}");

    let mut final_output = String::new();

    // Stream the initial message
    let messages = client
        .stream_message(agent_id, prompt, |msg| {
            if let Some(text) = msg.assistant_text() {
                print!("{text}");
                let _ = std::io::Write::flush(&mut std::io::stdout());
            }
        })
        .await?;

    collect_assistant_text(&messages, &mut final_output);
    process_tool_calls(client, agent_id, messages, permissions, &mut final_output).await?;

    Ok(final_output.trim().to_string())
}

async fn process_tool_calls(
    client: &LettaClient,
    agent_id: &str,
    messages: Vec<LettaMessage>,
    permissions: &PermissionManager,
    output: &mut String,
) -> Result<()> {
    let tool_calls: Vec<(String, String, serde_json::Value)> = messages
        .iter()
        .filter_map(|m| m.as_tool_call())
        .collect();

    for (call_id, tool_name, args) in tool_calls {
        if permissions.is_blocked(&tool_name) {
            tracing::warn!("Tool '{tool_name}' blocked (plan mode)");
            let follow = client
                .stream_tool_return(agent_id, &call_id, &format!("Tool '{tool_name}' blocked"), true, |_| {})
                .await?;
            collect_assistant_text(&follow, output);
            Box::pin(process_tool_calls(client, agent_id, follow, permissions, output)).await?;
            continue;
        }

        tracing::info!("Executing tool: {tool_name}");
        let result = dispatch(call_id.clone(), &tool_name, &args).await;
        tracing::debug!("Tool '{}': {} bytes", tool_name, result.output.len());

        let follow = client
            .stream_tool_return(agent_id, &call_id, &result.output, result.is_error, |msg| {
                if let Some(text) = msg.assistant_text() {
                    print!("{text}");
                    let _ = std::io::Write::flush(&mut std::io::stdout());
                }
            })
            .await?;

        collect_assistant_text(&follow, output);
        Box::pin(process_tool_calls(client, agent_id, follow, permissions, output)).await?;
    }

    Ok(())
}

fn collect_assistant_text(messages: &[LettaMessage], output: &mut String) {
    for msg in messages {
        if let Some(text) = msg.assistant_text() {
            if !text.is_empty() {
                output.push_str(text);
                output.push('\n');
            }
        }
    }
}
