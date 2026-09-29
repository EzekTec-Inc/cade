use crate::Result;
use crate::support::text::sanitize_for_terminal;
use cade_agent::agent::{HttpTransport, client::CadeMessage};
use cade_agent::mcp::McpManager;
use cade_core::hooks::HookEngine;
use cade_core::permissions::PermissionManager;
use serde_json::json;

/// A terminal-rendering callback event.
pub enum HeadlessEvent<'a> {
    Text(&'a str),
    ToolCall(&'a str),
}

#[derive(Debug, Default)]
pub struct HeadlessStats {
    pub turn_count: u32,
    pub tool_count: u32,
    pub duration_ms: u128,
}

/// Run one server-owned agent turn and render its ordered runtime events.
///
/// Permissions, hooks, MCP lifecycle, and tool execution belong to the
/// canonical runtime; the headless CLI only formats assistant output.
#[allow(clippy::type_complexity)]
pub async fn run_headless(
    client: &HttpTransport,
    agent_id: &str,
    prompt: &str,
    permissions: &PermissionManager,
    _mcp: &std::sync::Arc<McpManager>,
    _hooks: &HookEngine,
    on_output: Option<std::sync::Arc<dyn for<'a> Fn(HeadlessEvent<'a>) + Send + Sync>>,
    _max_tokens_budget: Option<u64>,
    _allowed_paths: Option<Vec<String>>,
) -> Result<(String, HeadlessStats)> {
    run_headless_with_cancel(
        client,
        agent_id,
        prompt,
        permissions,
        _mcp,
        _hooks,
        on_output,
        _max_tokens_budget,
        _allowed_paths,
        None,
    )
    .await
}

#[allow(clippy::too_many_arguments, clippy::type_complexity)]
pub async fn run_headless_with_cancel(
    client: &HttpTransport,
    agent_id: &str,
    prompt: &str,
    permissions: &PermissionManager,
    _mcp: &std::sync::Arc<McpManager>,
    _hooks: &HookEngine,
    on_output: Option<std::sync::Arc<dyn for<'a> Fn(HeadlessEvent<'a>) + Send + Sync>>,
    _max_tokens_budget: Option<u64>,
    _allowed_paths: Option<Vec<String>>,
    cancel: Option<&std::sync::Arc<std::sync::atomic::AtomicBool>>,
) -> Result<(String, HeadlessStats)> {
    let started = std::time::Instant::now();
    let output = std::sync::Arc::new(parking_lot::Mutex::new(String::new()));
    let output_for_event = output.clone();
    let messages = client
        .start_run_cancellable_with_mode(
            agent_id,
            prompt,
            None,
            Some(&permissions.mode().to_string()),
            move |message| {
                if let Some(text) = message.assistant_text() {
                    output_for_event.lock().push_str(text);
                    if let Some(callback) = &on_output {
                        callback(HeadlessEvent::Text(text));
                    } else {
                        print!("{}", sanitize_for_terminal(text));
                        let _ = std::io::Write::flush(&mut std::io::stdout());
                    }
                }
            },
            cancel,
        )
        .await?;
    Ok((
        output.lock().trim().to_owned(),
        statistics(&messages, started.elapsed().as_millis()),
    ))
}

/// Run a server-owned turn and expose the shared run-event vocabulary as JSONL.
pub async fn run_headless_stream_json(
    client: &HttpTransport,
    agent_id: &str,
    model: &str,
    prompt: &str,
    _permissions: &PermissionManager,
    _mcp: &std::sync::Arc<McpManager>,
    _hooks: &HookEngine,
) {
    use std::io::Write;

    let started = std::time::Instant::now();
    let output = std::sync::Arc::new(parking_lot::Mutex::new(String::new()));
    let output_for_event = output.clone();
    println!(
        "{}",
        json!({ "type": "init", "agent_id": agent_id, "model": model })
    );
    let _ = std::io::stdout().flush();

    let result = client
        .start_run(agent_id, prompt, None, move |message| {
            if let Some(text) = message.assistant_text() {
                output_for_event.lock().push_str(text);
            }
            println!(
                "{}",
                json!({
                    "type": "message",
                    "messageType": message.msg_type(),
                    "event": message.data,
                    "runId": message.run_id(),
                    "seqId": message.seq_id(),
                })
            );
            let _ = std::io::stdout().flush();
        })
        .await;

    match result {
        Ok(messages) => println!(
            "{}",
            json!({
                "type": "result",
                "subtype": "success",
                "is_error": false,
                "duration_ms": started.elapsed().as_millis() as u64,
                "num_turns": statistics(&messages, 0).turn_count,
                "result": output.lock().trim(),
                "agent_id": agent_id,
            })
        ),
        Err(error) => println!(
            "{}",
            json!({
                "type": "result",
                "subtype": "error",
                "is_error": true,
                "error": error.to_string(),
                "agent_id": agent_id,
            })
        ),
    }
    let _ = std::io::stdout().flush();
}

fn statistics(messages: &[CadeMessage], duration_ms: u128) -> HeadlessStats {
    HeadlessStats {
        turn_count: messages
            .iter()
            .filter(|message| message.msg_type() == "stream_start")
            .count() as u32,
        tool_count: messages
            .iter()
            .filter(|message| message.msg_type() == "tool_call_message")
            .count() as u32,
        duration_ms,
    }
}
