use anyhow::Result;
use serde_json::json;

use crate::agent::{CadeClient, client::CadeMessage};
use crate::mcp::McpManager;
use crate::permissions::PermissionManager;
use crate::tools::dispatch;

// ── Headless run statistics ───────────────────────────────────────────────────

#[derive(Debug, Default)]
pub struct HeadlessStats {
    pub turn_count:   u32,
    pub tool_count:   u32,
    pub duration_ms:  u128,
}

// ── Text mode (default) ───────────────────────────────────────────────────────

/// Run a single headless prompt with streaming, driving the tool loop to completion.
/// Prints streaming output to stdout. Returns the final assistant text + stats.
pub async fn run_headless(
    client: &CadeClient,
    agent_id: &str,
    prompt: &str,
    permissions: &PermissionManager,
    mcp: &McpManager,
) -> Result<(String, HeadlessStats)> {
    tracing::debug!("headless: agent={agent_id}");

    let t0 = std::time::Instant::now();
    let mut final_output = String::new();
    let mut stats = HeadlessStats::default();

    // Stream the initial message
    let messages = client
        .stream_message(agent_id, prompt, |msg| {
            if let Some(text) = msg.assistant_text() {
                print!("{text}");
                let _ = std::io::Write::flush(&mut std::io::stdout());
            }
        })
        .await?;

    stats.turn_count += 1;
    collect_assistant_text(&messages, &mut final_output);
    process_tool_calls(client, agent_id, messages, permissions, &mut final_output, mcp, &mut stats).await?;

    stats.duration_ms = t0.elapsed().as_millis();
    Ok((final_output.trim().to_string(), stats))
}

// ── stream-json mode ──────────────────────────────────────────────────────────

/// Run headless with JSONL (stream-json) output — one JSON object per event.
/// Emits to stdout. Each line is a complete JSON object (JSONL format).
pub async fn run_headless_stream_json(
    client: &CadeClient,
    agent_id: &str,
    model: &str,
    prompt: &str,
    permissions: &PermissionManager,
    mcp: &McpManager,
) {
    use std::io::Write;
    let t0 = std::time::Instant::now();

    let emit = |obj: serde_json::Value| {
        println!("{}", obj);
        let _ = std::io::stdout().flush();
    };

    // Init event
    emit(json!({ "type": "init", "agent_id": agent_id, "model": model }));

    let mut final_output = String::new();
    let mut stats        = HeadlessStats::default();
    let seq = std::sync::Arc::new(std::sync::atomic::AtomicU32::new(0));

    let seq2 = std::sync::Arc::clone(&seq);
    let messages = client
        .stream_message(agent_id, prompt, move |msg| {
            if let Some(text) = msg.assistant_text() {
                let s = seq2.fetch_add(1, std::sync::atomic::Ordering::SeqCst) + 1;
                emit(json!({
                    "type": "message",
                    "messageType": "assistant_message",
                    "content": text,
                    "seqId": s
                }));
            }
        })
        .await;

    let messages = match messages {
        Ok(m) => { stats.turn_count += 1; m }
        Err(e) => {
            emit(json!({ "type": "result", "subtype": "error", "error": e.to_string(),
                         "agent_id": agent_id }));
            return;
        }
    };

    collect_assistant_text(&messages, &mut final_output);

    // Process tool calls — emit events for each call + result
    let result = process_tool_calls_stream_json(
        client, agent_id, messages, permissions, &mut final_output, mcp, &mut stats, &emit
    ).await;

    if let Err(e) = result {
        emit(json!({ "type": "result", "subtype": "error", "error": e.to_string(),
                     "agent_id": agent_id }));
        return;
    }

    emit(json!({ "type": "message", "messageType": "stop_reason", "stopReason": "end_turn" }));
    emit(json!({
        "type":       "result",
        "subtype":    "success",
        "is_error":   false,
        "duration_ms": t0.elapsed().as_millis() as u64,
        "num_turns":  stats.turn_count,
        "result":     final_output.trim(),
        "agent_id":   agent_id,
    }));
}

async fn process_tool_calls(
    client: &CadeClient,
    agent_id: &str,
    messages: Vec<CadeMessage>,
    permissions: &PermissionManager,
    output: &mut String,
    mcp: &McpManager,
    stats: &mut HeadlessStats,
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
            stats.turn_count += 1;
            Box::pin(process_tool_calls(client, agent_id, follow, permissions, output, mcp, stats)).await?;
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
            let description = args["description"].as_str();
            let (msg, err) = match client.upsert_memory(agent_id, &label, &final_value, description).await {
                Ok(_) => (format!("Memory block '{label}' updated"), false),
                Err(e) => (format!("Failed: {e}"), true),
            };
            let follow = client.stream_tool_return(agent_id, &call_id, &msg, err, |_| {}).await?;
            collect_assistant_text(&follow, output);
            stats.turn_count += 1;
            Box::pin(process_tool_calls(client, agent_id, follow, permissions, output, mcp, stats)).await?;
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
            stats.turn_count += 1;
            Box::pin(process_tool_calls(client, agent_id, follow, permissions, output, mcp, stats)).await?;
            continue;
        }

        tracing::info!("Executing tool: {tool_name}");
        stats.tool_count += 1;
        let result = dispatch(call_id.clone(), &tool_name, &args, mcp).await;
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
        stats.turn_count += 1;
        Box::pin(process_tool_calls(client, agent_id, follow, permissions, output, mcp, stats)).await?;
    }

    Ok(())
}

// ── stream-json tool loop ─────────────────────────────────────────────────────

async fn process_tool_calls_stream_json(
    client: &CadeClient,
    agent_id: &str,
    messages: Vec<CadeMessage>,
    permissions: &PermissionManager,
    output: &mut String,
    mcp: &McpManager,
    stats: &mut HeadlessStats,
    emit: &impl Fn(serde_json::Value),
) -> Result<()> {
    let tool_calls: Vec<(String, String, serde_json::Value)> = messages
        .iter()
        .filter_map(|m| m.as_tool_call())
        .collect();

    for (call_id, tool_name, args) in tool_calls {
        emit(json!({ "type": "tool_call", "tool": tool_name, "args": args }));

        let (result_output, is_error) = if permissions.is_blocked(&tool_name, &args) {
            let reason = permissions.block_reason(&tool_name, &args);
            (reason, true)
        } else {
            stats.tool_count += 1;
            let r = dispatch(call_id.clone(), &tool_name, &args, mcp).await;
            (r.output, r.is_error)
        };

        emit(json!({
            "type": "tool_result",
            "tool": tool_name,
            "output": result_output,
            "is_error": is_error
        }));

        let follow = client
            .stream_tool_return(agent_id, &call_id, &result_output, is_error, |msg| {
                if let Some(text) = msg.assistant_text() {
                    emit(json!({
                        "type": "message",
                        "messageType": "assistant_message",
                        "content": text
                    }));
                }
            })
            .await?;

        collect_assistant_text(&follow, output);
        stats.turn_count += 1;
        Box::pin(process_tool_calls_stream_json(
            client, agent_id, follow, permissions, output, mcp, stats, emit
        )).await?;
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
