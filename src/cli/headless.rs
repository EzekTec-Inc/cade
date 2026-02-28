use anyhow::Result;

use crate::agent::{CadeClient, client::CadeMessage};
use crate::permissions::PermissionManager;
use crate::tools::dispatch;

/// Run a single headless prompt with streaming, driving the tool loop to completion.
/// Prints streaming output to stdout. Returns the final assistant text.
pub async fn run_headless(
    client: &CadeClient,
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
    client: &CadeClient,
    agent_id: &str,
    messages: Vec<CadeMessage>,
    permissions: &PermissionManager,
    output: &mut String,
) -> Result<()> {
    let tool_calls: Vec<(String, String, serde_json::Value)> = messages
        .iter()
        .filter_map(|m| m.as_tool_call())
        .collect();

    for (call_id, tool_name, args) in tool_calls {
        if permissions.is_blocked(&tool_name, &args) {
            let reason = permissions.block_reason(&tool_name, &args);
            tracing::warn!("{reason}");
            let follow = client
                .stream_tool_return(agent_id, &call_id, &reason, true, |_| {})
                .await?;
            collect_assistant_text(&follow, output);
            Box::pin(process_tool_calls(client, agent_id, follow, permissions, output)).await?;
            continue;
        }

        // Intercept update_memory — handled natively
        if tool_name == "update_memory" {
            let label = args["label"].as_str().unwrap_or("").trim().to_string();
            let value = args["value"].as_str().unwrap_or("").to_string();
            let operation = args["operation"].as_str().unwrap_or("set");
            let final_value = if operation == "append" {
                let existing = client.get_memory(agent_id).await
                    .unwrap_or_default()
                    .into_iter()
                    .find(|b| b.label == label)
                    .map(|b| b.value)
                    .unwrap_or_default();
                if existing.is_empty() { value } else { format!("{existing}\n{value}") }
            } else { value };
            let (msg, err) = match client.upsert_memory(agent_id, &label, &final_value).await {
                Ok(_) => (format!("Memory block '{label}' updated"), false),
                Err(e) => (format!("Failed: {e}"), true),
            };
            let follow = client.stream_tool_return(agent_id, &call_id, &msg, err, |_| {}).await?;
            collect_assistant_text(&follow, output);
            Box::pin(process_tool_calls(client, agent_id, follow, permissions, output)).await?;
            continue;
        }

        // load_skill — headless: return full body if skill found in current dir
        if tool_name == "load_skill" {
            let id = args["id"].as_str().unwrap_or("").trim().to_string();
            let skills = crate::skills::discover_all_skills(
                &std::env::current_dir().unwrap_or_default(), None, None
            );
            let (msg, err) = match skills.into_iter().find(|s| s.id == id) {
                Some(s) => (s.to_context_block(), false),
                None    => (format!("Skill '{id}' not found"), true),
            };
            let follow = client.stream_tool_return(agent_id, &call_id, &msg, err, |_| {}).await?;
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

fn collect_assistant_text(messages: &[CadeMessage], output: &mut String) {
    for msg in messages {
        if let Some(text) = msg.assistant_text() {
            if !text.is_empty() {
                output.push_str(text);
                output.push('\n');
            }
        }
    }
}
