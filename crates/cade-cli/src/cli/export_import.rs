/// Agent export / import — portable JSON snapshots of an agent's full state.
///
/// Export payload schema:
/// ```json
/// {
///   "cade_export_version": 1,
///   "exported_at": "<ISO-8601>",
///   "agent": { "id", "name", "model", "description", "system_prompt" },
///   "memory":  [ { "label", "value", "description" } ],
///   "conversations": [
///     {
///       "id": "<conv-id>",
///       "title": "<title>",
///       "messages": [ { "role", "content" } ]
///     }
///   ]
/// }
/// ```
use anyhow::{Context, Result};
use serde_json::{json, Value};

use cade_agent::agent::client::{CadeClient, CreateAgentRequest, MemoryBlock};

// -- Export

/// Export the agent identified by `agent_id` to a JSON value.
/// Fetches: agent metadata, memory blocks, conversations + their messages.
pub async fn export_agent(client: &CadeClient, agent_id: &str) -> Result<Value> {
    // 1. Agent metadata
    let agent = client.get_agent(agent_id).await
        .with_context(|| format!("get_agent {agent_id}"))?;

    // 2. Memory blocks
    let memory = client.get_memory(agent_id).await
        .unwrap_or_default();

    let memory_json: Vec<Value> = memory.iter().map(|b| json!({
        "label":       b.label,
        "value":       b.value,
        "description": b.description,
    })).collect();

    // 3. Conversations + messages
    let convs = client.list_conversations(agent_id).await.unwrap_or_default();
    let mut conversations_json: Vec<Value> = Vec::new();

    for conv in &convs {
        let conv_id   = conv["id"].as_str().unwrap_or("").to_string();
        let conv_title = conv["title"].as_str().unwrap_or("").to_string();

        // GET /v1/agents/:id/messages?conversation_id=<id>
        let msgs = client.get_conversation_messages(agent_id, &conv_id).await
            .unwrap_or_default();

        conversations_json.push(json!({
            "id":       conv_id,
            "title":    conv_title,
            "messages": msgs,
        }));
    }

    // Also grab messages that don't belong to any conversation (legacy / default)
    let default_msgs = client.get_conversation_messages(agent_id, "").await
        .unwrap_or_default();
    if !default_msgs.is_empty() {
        conversations_json.push(json!({
            "id":       null,
            "title":    "default",
            "messages": default_msgs,
        }));
    }

    let now = chrono::Utc::now().to_rfc3339();

    Ok(json!({
        "cade_export_version": 1,
        "exported_at": now,
        "agent": {
            "id":            agent.id,
            "name":          agent.name,
            "model":         agent.model,
            "description":   agent.description,
            "system_prompt": agent.system_prompt,
        },
        "memory":        memory_json,
        "conversations": conversations_json,
    }))
}

/// Serialize and write the export payload to `output_path`.
/// Use `"-"` to write to stdout.
pub async fn export_agent_to_file(
    client: &CadeClient,
    agent_id: &str,
    output_path: &str,
) -> Result<()> {
    let payload = export_agent(client, agent_id).await?;
    let pretty  = serde_json::to_string_pretty(&payload)?;

    if output_path == "-" {
        println!("{pretty}");
    } else {
        std::fs::write(output_path, &pretty)
            .with_context(|| format!("write export to {output_path}"))?;
        println!("✓ Exported agent '{}' → {output_path}", payload["agent"]["name"].as_str().unwrap_or(agent_id));
    }
    Ok(())
}

// -- Import

/// Import an agent from a JSON export file.
/// Creates a NEW agent (never overwrites an existing one).
/// Returns the new agent's ID.
pub async fn import_agent_from_file(client: &CadeClient, input_path: &str) -> Result<String> {
    let content = if input_path == "-" {
        let mut s = String::new();
        use std::io::Read;
        std::io::stdin().read_to_string(&mut s)?;
        s
    } else {
        std::fs::read_to_string(input_path)
            .with_context(|| format!("read import file {input_path}"))?
    };

    let payload: Value = serde_json::from_str(&content)
        .context("parse export JSON")?;

    import_agent(client, &payload).await
}

/// Import from an already-parsed JSON payload. Returns the new agent ID.
pub async fn import_agent(client: &CadeClient, payload: &Value) -> Result<String> {
    let version = payload["cade_export_version"].as_u64().unwrap_or(0);
    if version != 1 {
        anyhow::bail!("Unsupported export version: {version} (expected 1)");
    }

    let agent_data = &payload["agent"];
    let orig_name  = agent_data["name"].as_str().unwrap_or("imported-agent");
    let model      = agent_data["model"].as_str().unwrap_or("").to_string();
    let description = agent_data["description"].as_str().map(String::from);
    let system_prompt = agent_data["system_prompt"].as_str().map(String::from);

    // Derive a unique import name
    let import_name = format!("{orig_name} (imported)");

    // Build memory blocks for the create request
    let memory_blocks: Vec<MemoryBlock> = payload["memory"]
        .as_array()
        .unwrap_or(&vec![])
        .iter()
        .filter_map(|b| {
            let label = b["label"].as_str()?.to_string();
            let value = b["value"].as_str().unwrap_or("").to_string();
            let description = b["description"].as_str().map(String::from);
            Some(MemoryBlock { label, value, description, tier: None })
        })
        .collect();

    // Create the new agent
    let req = CreateAgentRequest {
        name:          Some(import_name.clone()),
        model:         model.clone(),
        description,
        system_prompt,
        memory_blocks,
        tool_ids:      vec![],
    };

    let new_agent = client.create_agent(req).await
        .context("create agent during import")?;

    println!("✓ Created agent '{import_name}' ({})", new_agent.id);

    // Restore conversations — we don't replay messages back through the LLM;
    // we skip conversation restoration as the server inserts messages only via
    // the inference path. We surface a note instead.
    let conv_count = payload["conversations"]
        .as_array()
        .map(|a| a.len())
        .unwrap_or(0);

    if conv_count > 0 {
        println!("  ℹ  {conv_count} conversation(s) in the export are preserved in the file.");
        println!("     Message history is not replayed into the new agent to avoid");
        println!("     re-triggering tool calls. Use the export file as an archive.");
    }

    let mem_count = payload["memory"]
        .as_array()
        .map(|a| a.len())
        .unwrap_or(0);
    println!("  ✓ {mem_count} memory block(s) restored.");

    Ok(new_agent.id)
}

// -- Helper: resolve agent by name-or-id

/// Resolve `name_or_id` to an agent ID:
/// - if it matches an exact agent ID → use it
/// - otherwise do a case-insensitive partial name search
pub async fn resolve_agent_id(client: &CadeClient, name_or_id: &str) -> Result<String> {
    // Try direct ID first
    if client.get_agent(name_or_id).await.is_ok() {
        return Ok(name_or_id.to_string());
    }
    // Fall back to name search
    let all = client.list_agents().await.context("list agents")?;
    let q   = name_or_id.to_lowercase();
    let matched: Vec<_> = all.iter()
        .filter(|a| a.name.to_lowercase().contains(&q))
        .collect();
    match matched.len() {
        0 => anyhow::bail!("No agent found matching '{name_or_id}'"),
        1 => Ok(matched[0].id.clone()),
        n => anyhow::bail!(
            "{n} agents match '{name_or_id}': {}",
            matched.iter().map(|a| format!("{} ({})", a.name, a.id)).collect::<Vec<_>>().join(", ")
        ),
    }
}

// -- Default output path

/// Generate a default export filename from agent name + timestamp.
pub fn default_export_path(agent_name: &str) -> String {
    let slug: String = agent_name
        .chars()
        .map(|c| if c.is_alphanumeric() { c.to_lowercase().next().unwrap_or(c) } else { '-' })
        .collect::<String>()
        .trim_matches('-')
        .to_string();
    let ts = chrono::Utc::now().format("%Y%m%dT%H%M%S");
    format!("{slug}-{ts}.json")
}
