use crate::Result;
use futures::future::join_all;
use serde_json::json;

use crate::support::text::sanitize_for_terminal;
use cade_agent::agent::{HttpTransport, client::CadeMessage};
use cade_agent::mcp::McpManager;
use cade_agent::tools::{ToolRuntime, dispatch};
use cade_core::hooks::{HookEngine, HookOutcome};
use cade_core::permissions::PermissionManager;

// -- Headless run statistics

#[derive(Debug, Default)]
pub struct HeadlessStats {
    pub turn_count: u32,
    pub tool_count: u32,
    pub duration_ms: u128,
}

// -- Tool classification

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
    matches!(
        name,
        "update_memory" | "load_skill" | "install_skill" | "run_skill_script" | "load_skill_ref"
    )
}

// -- Text mode (default)

/// Run a single headless prompt with streaming, driving the tool loop to completion.
/// Prints streaming output to stdout. Returns the final assistant text + stats.
#[allow(clippy::type_complexity)]
pub async fn run_headless(
    client: &HttpTransport,
    agent_id: &str,
    prompt: &str,
    permissions: &PermissionManager,
    mcp: &std::sync::Arc<McpManager>,
    hooks: &HookEngine,
    on_output: Option<std::sync::Arc<dyn Fn(&str) + Send + Sync>>,
) -> Result<(String, HeadlessStats)> {
    tracing::debug!("headless: agent={agent_id}");

    // UserPromptSubmit hook — can block the turn entirely.
    if !hooks.is_empty()
        && let HookOutcome::Block { reason } = hooks.user_prompt_submit(prompt).await
    {
        return Err(crate::Error::custom(format!(
            "Prompt blocked by hook: {reason}"
        )));
    }

    let t0 = std::time::Instant::now();
    let mut final_output = String::new();
    let mut stats = HeadlessStats::default();

    // Stream the initial message
    let messages = client
        .stream_message(agent_id, prompt, |msg| {
            if let Some(text) = msg.assistant_text() {
                if let Some(ref cb) = on_output {
                    cb(text);
                } else {
                    let safe = sanitize_for_terminal(text);
                    print!("{safe}");
                    let _ = std::io::Write::flush(&mut std::io::stdout());
                }
            }
        })
        .await?;

    stats.turn_count += 1;
    collect_assistant_text(&messages, &mut final_output);
    process_tool_calls(
        client,
        agent_id,
        messages,
        permissions,
        &mut final_output,
        mcp,
        &mut stats,
        hooks,
        &on_output,
    )
    .await?;

    // Stop hook — can annotate the final output but does not trigger a continuation turn.
    if !hooks.is_empty()
        && let HookOutcome::Block { reason } =
            hooks.stop("end_turn", prompt, &final_output, None).await
    {
        final_output.push_str("\n\n");
        final_output.push_str(&format!("[Stop hook: {reason}]"));
    }

    stats.duration_ms = t0.elapsed().as_millis();
    Ok((final_output.trim().to_string(), stats))
}

// -- stream-json mode

/// Run headless with JSONL (stream-json) output — one JSON object per event.
/// Emits to stdout. Each line is a complete JSON object (JSONL format).
pub async fn run_headless_stream_json(
    client: &HttpTransport,
    agent_id: &str,
    model: &str,
    prompt: &str,
    permissions: &PermissionManager,
    mcp: &std::sync::Arc<McpManager>,
    hooks: &HookEngine,
) {
    use std::io::Write;
    let t0 = std::time::Instant::now();

    let emit = |obj: serde_json::Value| {
        println!("{obj}");
        let _ = std::io::stdout().flush();
    };

    // Init event
    emit(json!({ "type": "init", "agent_id": agent_id, "model": model }));

    // UserPromptSubmit hook — can block the turn entirely.
    if !hooks.is_empty()
        && let HookOutcome::Block { reason } = hooks.user_prompt_submit(prompt).await
    {
        emit(json!({
            "type":     "result",
            "subtype":  "error",
            "is_error": true,
            "error":    format!("Prompt blocked by hook: {reason}"),
            "agent_id": agent_id,
        }));
        return;
    }

    let mut final_output = String::new();
    let mut stats = HeadlessStats::default();
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
        Ok(m) => {
            stats.turn_count += 1;
            m
        }
        Err(e) => {
            emit(
                json!({ "type": "result", "subtype": "error", "error": e.to_string(),
                         "agent_id": agent_id }),
            );
            return;
        }
    };

    collect_assistant_text(&messages, &mut final_output);

    // Process tool calls — emit events for each call + result
    let result = process_tool_calls_stream_json(
        client,
        agent_id,
        messages,
        permissions,
        &mut final_output,
        mcp,
        &mut stats,
        &emit,
        hooks,
    )
    .await;

    if let Err(e) = result {
        emit(
            json!({ "type": "result", "subtype": "error", "error": e.to_string(),
                     "agent_id": agent_id }),
        );
        return;
    }

    emit(json!({ "type": "message", "messageType": "stop_reason", "stopReason": "end_turn" }));

    // Stop hook — can annotate the final output but does not trigger a continuation turn.
    if !hooks.is_empty()
        && let HookOutcome::Block { reason } =
            hooks.stop("end_turn", prompt, &final_output, None).await
    {
        final_output.push_str("\n\n");
        final_output.push_str(&format!("[Stop hook: {reason}]"));
    }

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

// -- Tool loop helpers

/// Execute a single tool call, respecting permissions and intercepting
/// native tools (update_memory, load_skill).
///
/// Returns `(call_id, output, is_error)`.
async fn run_one_tool(
    client: &HttpTransport,
    agent_id: &str,
    call_id: String,
    tool_name: String,
    args: serde_json::Value,
    permissions: &PermissionManager,
    mcp: &std::sync::Arc<McpManager>,
    hooks: &HookEngine,
) -> (String, String, bool) {
    let canonical_name = cade_agent::tools::manager::canonical_name(&tool_name);
    let is_mcp_write = cade_agent::tools::is_mcp_write_tool(canonical_name, mcp).await;
    // -- Unified permission resolution
    use cade_core::permissions::Verdict;
    match permissions.resolve(canonical_name, &args, is_mcp_write) {
        Verdict::Deny(reason) => {
            tracing::warn!("{reason}");
            return (call_id, reason, true);
        }
        Verdict::Ask(reason) => {
            // Headless mode cannot prompt — treat Ask as Deny
            tracing::warn!("headless: cannot prompt for approval, denying: {reason}");
            return (call_id, reason, true);
        }
        Verdict::Allow => {}
    }

    // -- PreToolUse hook — can block execution
    if !hooks.is_empty()
        && let HookOutcome::Block { reason } = hooks.pre_tool_use(&tool_name, &args).await
    {
        let msg = format!("Blocked by hook: {reason}");
        tracing::warn!("{msg}");
        return (call_id, msg, true);
    }

    // -- Unified dispatch via ToolRuntime (memory, skills, checkpoints, native tools)
    let cwd = std::env::current_dir().unwrap_or_default();
    let runtime = ToolRuntime::new(
        std::sync::Arc::new(client.clone()),
        std::sync::Arc::clone(mcp),
        agent_id.to_string(),
        cwd,
    );
    if let Some(result) = runtime.execute(call_id.clone(), &tool_name, &args).await {
        return finalize_tool_result(
            client,
            agent_id,
            hooks,
            call_id,
            tool_name,
            args,
            result.output,
            result.is_error,
        )
        .await;
    }

    // -- Fallback: interactive-only tools (run_subagent, ask_user_question) — dispatch natively
    tracing::info!("Executing tool: {tool_name}");
    let result = dispatch(call_id.clone(), &tool_name, &args, mcp).await;
    finalize_tool_result(
        client,
        agent_id,
        hooks,
        call_id,
        tool_name,
        args,
        result.output,
        result.is_error,
    )
    .await
}

/// Apply PostToolUse / PostToolUseFailure hooks for a completed tool.
async fn finalize_tool_result(
    client: &HttpTransport,
    agent_id: &str,
    hooks: &HookEngine,
    call_id: String,
    tool_name: String,
    args: serde_json::Value,
    mut output: String,
    is_error: bool,
) -> (String, String, bool) {
    if !is_error {
        match tool_name.as_str() {
            "write_file" | "edit_file" | "apply_patch" | "Replace" | "WriteFileGemini" => {
                let path = args["file_path"]
                    .as_str()
                    .or(args["path"].as_str())
                    .unwrap_or("unknown");
                let msg = format!("Recently edited: {path}\n");
                let c = client.clone();
                let a = agent_id.to_string();
                tokio::spawn(async move {
                    let _ = c
                        .append_memory_with_limit(&a, "working_set", &msg, None, Some(3000))
                        .await;
                });
            }
            _ => {}
        }
    }

    if hooks.is_empty() {
        return (call_id, output, is_error);
    }

    let preceding_reasoning: Option<&str> = None;
    let preceding_assistant_message: Option<&str> = None;

    if is_error {
        hooks
            .post_tool_use_failure(
                &tool_name,
                &args,
                &output,
                preceding_reasoning,
                preceding_assistant_message,
            )
            .await;
    } else if let Some(extra) = hooks
        .post_tool_use(
            &tool_name,
            &args,
            &output,
            preceding_reasoning,
            preceding_assistant_message,
        )
        .await
    {
        output = format!("{}\n\n[Hook context: {extra}]", output);
    }

    (call_id, output, is_error)
}

// -- Text-mode tool loop

#[allow(clippy::type_complexity)]
async fn process_tool_calls(
    client: &HttpTransport,
    agent_id: &str,
    messages: Vec<CadeMessage>,
    permissions: &PermissionManager,
    output: &mut String,
    mcp: &std::sync::Arc<McpManager>,
    stats: &mut HeadlessStats,
    hooks: &HookEngine,
    on_output: &Option<std::sync::Arc<dyn Fn(&str) + Send + Sync>>,
) -> Result<()> {
    let tool_calls: Vec<(String, String, serde_json::Value)> =
        messages.iter().filter_map(|m| m.as_tool_call()).collect();

    if tool_calls.is_empty() {
        return Ok(());
    }

    // Split into sequential (state-mutating) and parallel (independent) calls.
    // Sequential tools are handled one at a time in original order.
    // If all calls in the batch are sequential, fall through to sequential path.
    let all_sequential = tool_calls
        .iter()
        .all(|(_, name, _)| is_sequential_tool(name));

    if all_sequential || tool_calls.len() == 1 {
        // -- Sequential path
        for (call_id, tool_name, args) in tool_calls {
            let (cid, out, is_err) = run_one_tool(
                client,
                agent_id,
                call_id,
                tool_name,
                args,
                permissions,
                mcp,
                hooks,
            )
            .await;

            let on_out_clone = on_output.clone();
            let follow = client
                .stream_tool_return(agent_id, &cid, &out, is_err, move |msg| {
                    if let Some(text) = msg.assistant_text() {
                        if let Some(ref cb) = on_out_clone {
                            cb(text);
                        } else {
                            print!("{text}");
                            let _ = std::io::Write::flush(&mut std::io::stdout());
                        }
                    }
                })
                .await?;

            collect_assistant_text(&follow, output);
            stats.turn_count += 1;
            stats.tool_count += 1;
            Box::pin(process_tool_calls(
                client,
                agent_id,
                follow,
                permissions,
                output,
                mcp,
                stats,
                hooks,
                on_output,
            ))
            .await?;
        }
    } else {
        // -- Parallel path
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
            parallel_batch.len(),
            sequential_remainder.len()
        );

        // Spawn all parallel tools concurrently
        let futures: Vec<_> = parallel_batch
            .into_iter()
            .map(|(call_id, tool_name, args)| {
                let client = client.clone();
                let agent_id = agent_id.to_string();
                let perms = permissions.clone();
                async move {
                    run_one_tool(
                        &client, &agent_id, call_id, tool_name, args, &perms, mcp, hooks,
                    )
                    .await
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
                let on_out_clone = on_output.clone();
                let follow = client
                    .stream_tool_return(agent_id, &call_id, &out, is_err, move |msg| {
                        if let Some(text) = msg.assistant_text() {
                            if let Some(ref cb) = on_out_clone {
                                cb(text);
                            } else {
                                print!("{text}");
                                let _ = std::io::Write::flush(&mut std::io::stdout());
                            }
                        }
                    })
                    .await?;
                follow_msgs = follow;
            } else {
                // Non-last results — server buffers them, returns []
                client
                    .send_tool_return(agent_id, &call_id, &out, is_err)
                    .await?;
            }
        }

        // Now handle any sequential tools that were in this batch
        for (call_id, tool_name, args) in sequential_remainder {
            let (cid, out, is_err) = run_one_tool(
                client,
                agent_id,
                call_id,
                tool_name,
                args,
                permissions,
                mcp,
                hooks,
            )
            .await;

            let on_out_clone = on_output.clone();
            let follow = client
                .stream_tool_return(agent_id, &cid, &out, is_err, move |msg| {
                    if let Some(text) = msg.assistant_text() {
                        if let Some(ref cb) = on_out_clone {
                            cb(text);
                        } else {
                            print!("{text}");
                            let _ = std::io::Write::flush(&mut std::io::stdout());
                        }
                    }
                })
                .await?;
            follow_msgs = follow;
            stats.tool_count += 1;
        }

        collect_assistant_text(&follow_msgs, output);
        stats.turn_count += total as u32;
        Box::pin(process_tool_calls(
            client,
            agent_id,
            follow_msgs,
            permissions,
            output,
            mcp,
            stats,
            hooks,
            on_output,
        ))
        .await?;
    }

    Ok(())
}

// -- stream-json tool loop

async fn process_tool_calls_stream_json(
    client: &HttpTransport,
    agent_id: &str,
    messages: Vec<CadeMessage>,
    permissions: &PermissionManager,
    output: &mut String,
    mcp: &std::sync::Arc<McpManager>,
    stats: &mut HeadlessStats,
    emit: &impl Fn(serde_json::Value),
    hooks: &HookEngine,
) -> Result<()> {
    let tool_calls: Vec<(String, String, serde_json::Value)> =
        messages.iter().filter_map(|m| m.as_tool_call()).collect();

    if tool_calls.is_empty() {
        return Ok(());
    }

    let all_sequential = tool_calls
        .iter()
        .all(|(_, name, _)| is_sequential_tool(name));

    if all_sequential || tool_calls.len() == 1 {
        // -- Sequential path
        for (call_id, tool_name, args) in tool_calls {
            emit(json!({ "type": "tool_call", "tool": tool_name, "args": args }));

            let (cid, result_output, is_error) = run_one_tool(
                client,
                agent_id,
                call_id,
                tool_name.clone(),
                args,
                permissions,
                mcp,
                hooks,
            )
            .await;

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
                client,
                agent_id,
                follow,
                permissions,
                output,
                mcp,
                stats,
                emit,
                hooks,
            ))
            .await?;
        }
    } else {
        // -- Parallel path
        let total = tool_calls.len();
        let mut parallel_batch = Vec::new();
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
            parallel_batch.len(),
            sequential_remainder.len()
        );

        let futures: Vec<_> = parallel_batch
            .into_iter()
            .map(|(call_id, tool_name, args)| {
                let client = client.clone();
                let agent_id = agent_id.to_string();
                let perms = permissions.clone();
                async move {
                    let r = run_one_tool(
                        &client,
                        &agent_id,
                        call_id,
                        tool_name.clone(),
                        args,
                        &perms,
                        mcp,
                        hooks,
                    )
                    .await;
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
                client
                    .send_tool_return(agent_id, &call_id, &out, is_err)
                    .await?;
            }
        }

        for (call_id, tool_name, args) in sequential_remainder {
            emit(json!({ "type": "tool_call", "tool": tool_name, "args": args }));
            let (cid, out, is_err) = run_one_tool(
                client,
                agent_id,
                call_id,
                tool_name.clone(),
                args,
                permissions,
                mcp,
                hooks,
            )
            .await;
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
            client,
            agent_id,
            follow_msgs,
            permissions,
            output,
            mcp,
            stats,
            emit,
            hooks,
        ))
        .await?;
    }

    Ok(())
}

fn collect_assistant_text(messages: &[CadeMessage], output: &mut String) {
    for msg in messages {
        if let Some(text) = msg.assistant_text()
            && !text.is_empty()
        {
            output.push_str(text);
            output.push('\n');
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
