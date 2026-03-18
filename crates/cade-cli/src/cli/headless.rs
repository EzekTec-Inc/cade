use anyhow::Result;
use futures::future::join_all;
use serde_json::json;

use cade_agent::agent::{CadeClient, client::CadeMessage};
use cade_agent::mcp::McpManager;
use cade_core::permissions::PermissionManager;
use cade_agent::tools::dispatch;

/// Strip control characters that could act as ANSI/terminal escape sequences
/// when printed in headless mode. Newlines and tabs are preserved; other
/// bytes in the 0x00–0x1F and 0x7F range are dropped.
fn sanitize_for_terminal(s: &str) -> String {
    s.chars()
        .filter(|&ch| {
            let c = ch as u32;
            if ch == '\n' || ch == '\t' {
                true
            } else if c <= 0x1F || c == 0x7F {
                false
            } else {
                true
            }
        })
        .collect()
}

// ── Headless run statistics ───────────────────────────────────────────────────

#[derive(Debug, Default)]
pub struct HeadlessStats {
    pub turn_count:   u32,
    pub tool_count:   u32,
    pub duration_ms:  u128,
}

// ── Tool classification ───────────────────────────────────────────────────────

/// Returns true for tools that mutate shared agent state and must run sequentially.
///
/// These tools interact with the agent's memory or skills system and cannot be
/// safely parallelised with other calls in the same turn:
///   - `update_memory`     — writes to the agent memory block store
///   - `load_skill`        — reads skills and triggers a follow-up turn
///   - `install_skill`     — installs skills (file writes + agent state)
///   - `run_skill_script`  — executes a skill script (side-effects)
///   - `load_skill_ref`    — lazy-loads a reference doc
fn is_sequential_tool(name: &str) -> bool {
    matches!(name, "update_memory" | "load_skill" | "install_skill" | "run_skill_script" | "load_skill_ref")
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
                let safe = sanitize_for_terminal(text);
                print!("{safe}");
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

// ── Tool loop helpers ─────────────────────────────────────────────────────────

/// Execute a single tool call, respecting permissions and intercepting
/// native tools (update_memory, load_skill).
///
/// Returns `(call_id, output, is_error)`.
async fn run_one_tool(
    client: &CadeClient,
    agent_id: &str,
    call_id: String,
    tool_name: String,
    args: serde_json::Value,
    permissions: &PermissionManager,
    mcp: &McpManager,
) -> (String, String, bool) {
    // Permission check
    if permissions.is_blocked(&tool_name, &args) {
        let reason = permissions.block_reason(&tool_name, &args);
        tracing::warn!("{reason}");
        return (call_id, reason, true);
    }

    // Intercept: update_memory
    if tool_name == "update_memory" {
        let label     = args["label"].as_str().unwrap_or("").trim().to_string();
        let value     = args["value"].as_str().unwrap_or("").to_string();
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
        let description = args["description"].as_str().map(String::from);
        let (msg, err) = match client.upsert_memory(agent_id, &label, &final_value, description.as_deref()).await {
            Ok(_)  => (format!("Memory block '{label}' updated"), false),
            Err(e) => (format!("Failed: {e}"), true),
        };
        return (call_id, msg, err);
    }

    // Intercept: load_skill
    if tool_name == "load_skill" {
        let id     = args["id"].as_str().unwrap_or("").trim().to_string();
        let skills = cade_core::skills::discover_all_skills(
            &std::env::current_dir().unwrap_or_default(), None, None
        );
        let (msg, err) = match skills.into_iter().find(|s| s.id == id) {
            Some(s) => (s.to_context_block(), false),
            None    => (format!("Skill '{id}' not found"), true),
        };
        return (call_id, msg, err);
    }

    // Intercept: run_skill_script
    if tool_name == "run_skill_script" {
        let skill_id    = args["skill_id"].as_str().unwrap_or("").trim().to_string();
        let script      = args["script"].as_str().unwrap_or("").trim().to_string();
        let script_args: Vec<String> = args["args"]
            .as_array()
            .map(|a| a.iter().filter_map(|v| v.as_str().map(|s| s.to_string())).collect())
            .unwrap_or_default();

        if skill_id.is_empty() || script.is_empty() {
            return (call_id, "error: 'skill_id' and 'script' are required".to_string(), true);
        }

        let skills = cade_core::skills::discover_all_skills(
            &std::env::current_dir().unwrap_or_default(), None, None
        );
        let skill = match skills.into_iter().find(|s| s.id == skill_id) {
            Some(s) => s,
            None    => return (call_id, format!("Skill '{skill_id}' not found"), true),
        };
        let sk = match skill.scripts.iter().find(|s| s.name == script) {
            Some(s) => s.clone(),
            None => {
                let available: Vec<&str> = skill.scripts.iter().map(|s| s.name.as_str()).collect();
                let list = if available.is_empty() { "none".to_string() } else { available.join(", ") };
                return (call_id, format!("Script '{script}' not found in skill '{skill_id}'. Available: {list}"), true);
            }
        };

        tracing::info!("Running skill script: {} {}", sk.path.display(), script_args.join(" "));
        let mut cmd = tokio::process::Command::new(&sk.path);
        cade_core::agent_env::apply_agent_env(&mut cmd);
        match cmd.args(&script_args).output().await {
            Err(e) => return (call_id, format!("Failed to run script: {e}"), true),
            Ok(out) => {
                let stdout   = String::from_utf8_lossy(&out.stdout).to_string();
                let stderr   = String::from_utf8_lossy(&out.stderr).to_string();
                let combined = if stderr.is_empty() { stdout } else { format!("{stdout}\n[stderr]\n{stderr}") };
                let is_error = !out.status.success();
                return (call_id, combined, is_error);
            }
        }
    }

    // Intercept: load_skill_ref
    if tool_name == "load_skill_ref" {
        let skill_id = args["skill_id"].as_str().unwrap_or("").trim().to_string();
        let doc      = args["doc"].as_str().unwrap_or("").trim().to_string();

        if skill_id.is_empty() || doc.is_empty() {
            return (call_id, "error: 'skill_id' and 'doc' are required".to_string(), true);
        }

        let skills = cade_core::skills::discover_all_skills(
            &std::env::current_dir().unwrap_or_default(), None, None
        );
        let skill = match skills.into_iter().find(|s| s.id == skill_id) {
            Some(s) => s,
            None    => return (call_id, format!("Skill '{skill_id}' not found"), true),
        };
        let r = match skill.references.iter().find(|r| {
            r.name == doc || r.path.file_name().and_then(|n| n.to_str()).unwrap_or("") == doc
        }) {
            Some(r) => r.clone(),
            None => {
                let available: Vec<&str> = skill.references.iter().map(|r| r.name.as_str()).collect();
                let list = if available.is_empty() { "none".to_string() } else { available.join(", ") };
                return (call_id, format!("Reference '{doc}' not found in skill '{skill_id}'. Available: {list}"), true);
            }
        };

        match std::fs::read_to_string(&r.path) {
            Ok(content) => return (call_id, format!("# Reference: {doc} (skill: {skill_id})\n\n{content}"), false),
            Err(e)      => return (call_id, format!("Failed to read reference '{doc}': {e}"), true),
        }
    }

    // Generic tool dispatch
    tracing::info!("Executing tool: {tool_name}");
    let result = dispatch(call_id.clone(), &tool_name, &args, mcp).await;
    tracing::debug!("Tool '{}': {} bytes", tool_name, result.output.len());
    (call_id, result.output, result.is_error)
}

// ── Text-mode tool loop ───────────────────────────────────────────────────────

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

    if tool_calls.is_empty() {
        return Ok(());
    }

    // Split into sequential (state-mutating) and parallel (independent) calls.
    // Sequential tools are handled one at a time in original order.
    // If all calls in the batch are sequential, fall through to sequential path.
    let all_sequential = tool_calls.iter().all(|(_, name, _)| is_sequential_tool(name));

    if all_sequential || tool_calls.len() == 1 {
        // ── Sequential path ───────────────────────────────────────────────────
        for (call_id, tool_name, args) in tool_calls {
            let (cid, out, is_err) = run_one_tool(
                client, agent_id, call_id, tool_name, args, permissions, mcp
            ).await;

            let follow = client
                .stream_tool_return(agent_id, &cid, &out, is_err, |msg| {
                    if let Some(text) = msg.assistant_text() {
                        print!("{text}");
                        let _ = std::io::Write::flush(&mut std::io::stdout());
                    }
                })
                .await?;

            collect_assistant_text(&follow, output);
            stats.turn_count += 1;
            stats.tool_count += 1;
            Box::pin(process_tool_calls(client, agent_id, follow, permissions, output, mcp, stats)).await?;
        }
    } else {
        // ── Parallel path ─────────────────────────────────────────────────────
        // Execute all non-sequential tools concurrently, keep sequential ones
        // in their original positions but run them after the parallel batch.
        let total = tool_calls.len();
        let mut parallel_batch: Vec<(String, String, serde_json::Value)> = Vec::new();
        let mut sequential_remainder: Vec<(String, String, serde_json::Value)> = Vec::new();

        for tc in tool_calls {
            if is_sequential_tool(&tc.1) {
                sequential_remainder.push(tc);
            } else {
                parallel_batch.push(tc);
            }
        }

        tracing::info!(
            "Parallel tool dispatch: {} concurrent + {} sequential",
            parallel_batch.len(), sequential_remainder.len()
        );

        // Spawn all parallel tools concurrently
        let futures: Vec<_> = parallel_batch
            .into_iter()
            .map(|(call_id, tool_name, args)| {
                let client    = client.clone();
                let agent_id  = agent_id.to_string();
                let mcp       = mcp;
                let perms     = permissions.clone();
                async move {
                    run_one_tool(&client, &agent_id, call_id, tool_name, args, &perms, &mcp).await
                }
            })
            .collect();

        let results: Vec<(String, String, bool)> = join_all(futures).await;
        stats.tool_count += results.len() as u32;

        // Submit all parallel results back.
        // The server counts received vs expected: it only calls the LLM once all
        // expected results for this turn have arrived. So we send N-1 results
        // silently (they return empty messages), then send the last one which
        // triggers the LLM response.
        let result_count = results.len();
        let mut follow_msgs: Vec<CadeMessage> = Vec::new();

        for (i, (call_id, out, is_err)) in results.into_iter().enumerate() {
            let is_last = i == result_count - 1 && sequential_remainder.is_empty();
            if is_last {
                // Last result — triggers LLM response
                let follow = client
                    .stream_tool_return(agent_id, &call_id, &out, is_err, |msg| {
                        if let Some(text) = msg.assistant_text() {
                            print!("{text}");
                            let _ = std::io::Write::flush(&mut std::io::stdout());
                        }
                    })
                    .await?;
                follow_msgs = follow;
            } else {
                // Non-last results — server buffers them, returns []
                client.send_tool_return(agent_id, &call_id, &out, is_err).await?;
            }
        }

        // Now handle any sequential tools that were in this batch
        for (call_id, tool_name, args) in sequential_remainder {
            let (cid, out, is_err) = run_one_tool(
                client, agent_id, call_id, tool_name, args, permissions, mcp
            ).await;

            let follow = client
                .stream_tool_return(agent_id, &cid, &out, is_err, |msg| {
                    if let Some(text) = msg.assistant_text() {
                        print!("{text}");
                        let _ = std::io::Write::flush(&mut std::io::stdout());
                    }
                })
                .await?;
            follow_msgs = follow;
            stats.tool_count += 1;
        }

        collect_assistant_text(&follow_msgs, output);
        stats.turn_count += total as u32;
        Box::pin(process_tool_calls(client, agent_id, follow_msgs, permissions, output, mcp, stats)).await?;
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

    if tool_calls.is_empty() {
        return Ok(());
    }

    let all_sequential = tool_calls.iter().all(|(_, name, _)| is_sequential_tool(name));

    if all_sequential || tool_calls.len() == 1 {
        // ── Sequential path ───────────────────────────────────────────────────
        for (call_id, tool_name, args) in tool_calls {
            emit(json!({ "type": "tool_call", "tool": tool_name, "args": args }));

            let (cid, result_output, is_error) = run_one_tool(
                client, agent_id, call_id, tool_name.clone(), args, permissions, mcp
            ).await;

            emit(json!({
                "type": "tool_result",
                "tool": tool_name,
                "output": result_output,
                "is_error": is_error
            }));

            stats.tool_count += 1;
            let follow = client
                .stream_tool_return(agent_id, &cid, &result_output, is_error, |msg| {
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
    } else {
        // ── Parallel path ─────────────────────────────────────────────────────
        let total = tool_calls.len();
        let mut parallel_batch  = Vec::new();
        let mut sequential_remainder = Vec::new();

        for tc in tool_calls {
            emit(json!({ "type": "tool_call", "tool": tc.1, "args": tc.2 }));
            if is_sequential_tool(&tc.1) {
                sequential_remainder.push(tc);
            } else {
                parallel_batch.push(tc);
            }
        }

        tracing::info!(
            "Parallel tool dispatch (stream-json): {} concurrent + {} sequential",
            parallel_batch.len(), sequential_remainder.len()
        );

        let futures: Vec<_> = parallel_batch
            .into_iter()
            .map(|(call_id, tool_name, args)| {
                let client   = client.clone();
                let agent_id = agent_id.to_string();
                let mcp      = mcp;
                let perms    = permissions.clone();
                async move {
                    let r = run_one_tool(&client, &agent_id, call_id, tool_name.clone(), args, &perms, &mcp).await;
                    (tool_name, r)
                }
            })
            .collect();

        let results: Vec<(String, (String, String, bool))> = join_all(futures).await;
        stats.tool_count += results.len() as u32;

        // Emit tool results
        for (tool_name, (_, out, is_err)) in &results {
            emit(json!({
                "type": "tool_result",
                "tool": tool_name,
                "output": out,
                "is_error": is_err
            }));
        }

        // Submit results back — server batches them until all expected arrive
        let result_count = results.len();
        let mut follow_msgs: Vec<CadeMessage> = Vec::new();

        for (i, (_, (call_id, out, is_err))) in results.into_iter().enumerate() {
            let is_last = i == result_count - 1 && sequential_remainder.is_empty();
            if is_last {
                let follow = client
                    .stream_tool_return(agent_id, &call_id, &out, is_err, |msg| {
                        if let Some(text) = msg.assistant_text() {
                            emit(json!({
                                "type": "message",
                                "messageType": "assistant_message",
                                "content": text
                            }));
                        }
                    })
                    .await?;
                follow_msgs = follow;
            } else {
                client.send_tool_return(agent_id, &call_id, &out, is_err).await?;
            }
        }

        for (call_id, tool_name, args) in sequential_remainder {
            emit(json!({ "type": "tool_call", "tool": tool_name, "args": args }));
            let (cid, out, is_err) = run_one_tool(
                client, agent_id, call_id, tool_name.clone(), args, permissions, mcp
            ).await;
            emit(json!({
                "type": "tool_result", "tool": tool_name,
                "output": out, "is_error": is_err
            }));
            let follow = client
                .stream_tool_return(agent_id, &cid, &out, is_err, |msg| {
                    if let Some(text) = msg.assistant_text() {
                        emit(json!({
                            "type": "message",
                            "messageType": "assistant_message",
                            "content": text
                        }));
                    }
                })
                .await?;
            follow_msgs = follow;
            stats.tool_count += 1;
        }

        collect_assistant_text(&follow_msgs, output);
        stats.turn_count += total as u32;
        Box::pin(process_tool_calls_stream_json(
            client, agent_id, follow_msgs, permissions, output, mcp, stats, emit
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

// region:    --- Tests

#[cfg(test)]
mod tests {
    #[allow(unused)]
    type Result<T> = core::result::Result<T, Box<dyn std::error::Error>>; // For tests.

    use super::sanitize_for_terminal;

    #[test]
    fn preserves_normal_text_and_newlines() {
        assert_eq!(sanitize_for_terminal("hello\nworld\t!"), "hello\nworld\t!");
    }

    #[test]
    fn strips_ansi_escape_sequences() {
        let s = "ok\x1b[31mRED\x1b[0mnormal";
        assert_eq!(sanitize_for_terminal(s), "ok[31mRED[0mnormal");
    }

    #[test]
    fn strips_null_and_control_chars() {
        let s = "a\x00b\x01c\x7fd";
        assert_eq!(sanitize_for_terminal(s), "abcd");
    }

    #[test]
    fn preserves_unicode() {
        let s = "héllo wörld 日本語";
        assert_eq!(sanitize_for_terminal(s), s);
    }
}

// endregion: --- Tests
