//! Real OpenAI transport -> durable Conversation -> context -> subsequent transport.
use super::*;
use cade_ai::{LlmProvider, openai::OpenAiProvider, runtime::RuntimeRegistry};
use std::sync::Arc;
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::TcpListener,
    sync::mpsc,
};

async fn fixture(
    streaming: bool,
    first: Value,
) -> (Arc<dyn LlmProvider>, mpsc::UnboundedReceiver<Value>) {
    let last = json!({"status":"completed", "output":[{"type":"message", "content":[{"type":"output_text", "text":"finished"}]}]});
    fixture_replies(streaming, vec![first, last]).await
}

async fn fixture_replies(
    streaming: bool,
    replies: Vec<Value>,
) -> (Arc<dyn LlmProvider>, mpsc::UnboundedReceiver<Value>) {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}/v1", listener.local_addr().unwrap());
    let (tx, rx) = mpsc::unbounded_channel();
    tokio::spawn(async move {
        for reply in replies {
            let (mut socket, _) = listener.accept().await.unwrap();
            let mut bytes = Vec::new();
            let end = loop {
                let mut buf = [0; 4096];
                let n = socket.read(&mut buf).await.unwrap();
                assert!(n > 0);
                bytes.extend_from_slice(&buf[..n]);
                if let Some(i) = bytes.windows(4).position(|s| s == b"\r\n\r\n") {
                    break i + 4;
                }
            };
            let headers = String::from_utf8_lossy(&bytes[..end]);
            assert!(headers.starts_with("POST /v1/responses "));
            let length: usize = headers
                .lines()
                .find_map(|line| {
                    let (key, value) = line.split_once(':')?;
                    key.eq_ignore_ascii_case("content-length")
                        .then(|| value.trim().parse().unwrap())
                })
                .unwrap();
            while bytes.len() < end + length {
                let mut buf = [0; 4096];
                let n = socket.read(&mut buf).await.unwrap();
                assert!(n > 0);
                bytes.extend_from_slice(&buf[..n]);
            }
            tx.send(serde_json::from_slice(&bytes[end..end + length]).unwrap())
                .unwrap();
            let (kind, body) = if streaming {
                let event = if reply["status"] == "incomplete" {
                    "response.incomplete"
                } else {
                    "response.completed"
                };
                (
                    "text/event-stream",
                    format!("data: {}\n\n", json!({"type":event, "response":reply})),
                )
            } else {
                ("application/json", reply.to_string())
            };
            socket.write_all(format!("HTTP/1.1 200 OK\r\nContent-Type: {kind}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len()).as_bytes()).await.unwrap();
        }
    });
    let provider = OpenAiProvider::new("".into(), Some(base)).with_registry(
        "openai".into(),
        Arc::new(parking_lot::RwLock::new(RuntimeRegistry::default())),
    );
    (Arc::new(provider), rx)
}

#[tokio::test]
async fn responses_low_budget_compaction_preserves_anchored_persisted_tool_chain() {
    low_budget_tool_chain(false).await;
}

#[tokio::test]
async fn responses_low_budget_row_cutoff_preserves_current_exchange() {
    low_budget_tool_chain(true).await;
}

async fn low_budget_tool_chain(large_output: bool) {
    for model in ["openai/gpt-5", "openai/gpt-6-sol"] {
        for streaming in [false, true] {
            let workspace = tempfile::tempdir().unwrap();
            let tool_output = |name: &str| {
                format!(
                    "result of {name} {}",
                    if large_output && name == "parallel.txt" {
                        "x".repeat(11_000)
                    } else {
                        String::new()
                    }
                )
            };
            for name in ["first.txt", "parallel.txt", "latest.txt"] {
                std::fs::write(workspace.path().join(name), tool_output(name)).unwrap();
            }
            let user = format!(
                "Read first.txt then latest.txt. Reference data: {}",
                (0..700u64)
                    .map(|i| format!("{:08x} ", i.wrapping_mul(2_654_435_761) as u32))
                    .collect::<String>()
            );
            let reasoning = |id: &str| json!({"type":"reasoning", "id":format!("rs_{id}"), "summary":[], "encrypted_content":format!("opaque-{id}")});
            let turn = |id: &str, path: &str| json!({"status":"completed", "output":[reasoning(id), {"type":"function_call", "call_id":id, "name":"read_file", "arguments":json!({"path":path}).to_string(), "status":"completed"}]});
            let mut first = turn("first", "first.txt");
            if large_output {
                first["output"].as_array_mut().unwrap().push(json!({"type":"function_call", "call_id":"parallel", "name":"read_file", "arguments":"{\"path\":\"parallel.txt\"}", "status":"completed"}));
            }
            let (provider, mut requests) = fixture_replies(streaming, vec![first, turn("latest", "latest.txt"), json!({"status":"completed", "output":[{"type":"message", "content":[{"type":"output_text", "text":"finished"}]}]})]).await;
            let mut state = super::tests::build_state_with_llm(provider);
            let config = Arc::get_mut(&mut state.config).unwrap();
            config.max_context_budget = Some(8_600);
            config.max_tokens_per_turn = Some(128);
            sqlite::create_agent(
                &state.db,
                &sqlite::AgentRow {
                    id: "low-budget-parent".into(),
                    name: "Low budget continuation".into(),
                    model: model.into(),
                    description: None,
                    system_prompt: None,
                    created_at: None,
                    compaction_model: None,
                    theme: None,
                    active_plan_json: None,
                    parent_id: None,
                },
            )
            .unwrap();
            let conversation =
                sqlite::create_conversation(&state.db, "low-budget-parent", "compacted tool chain")
                    .unwrap();
            if streaming {
                let mut handle = runtime::ServerAgentRuntime::new(state.clone())
                    .start_with_options(
                        runtime::RunRequest {
                            agent_id: "low-budget-parent".into(),
                            conversation_id: Some(conversation.id.clone()),
                            input: user.clone(),
                            permission_mode: Some("plan".into()),
                        },
                        runtime::RunExecutionOptions {
                            cwd: Some(workspace.path().to_path_buf()),
                            ..Default::default()
                        },
                    )
                    .await
                    .unwrap();
                tokio::time::timeout(std::time::Duration::from_secs(20), async {
                    while let Some(Ok(_)) = handle.events.recv().await {}
                })
                .await
                .expect("Run must finish");
            } else {
                let result = super::super::messages::send_message(
                    State(state.clone()),
                    Path("low-budget-parent".into()),
                    Json(json!({"input":user, "conversation_id":conversation.id})),
                )
                .await;
                assert!(result.status().is_success());
                let returns = if large_output {
                    vec![
                        ("first", "first.txt"),
                        ("parallel", "parallel.txt"),
                        ("latest", "latest.txt"),
                    ]
                } else {
                    vec![("first", "first.txt"), ("latest", "latest.txt")]
                };
                for (id, path) in returns {
                    *state.context_cache.lock() = crate::server::state::SafeLruCache::new(
                        crate::server::state::CONTEXT_CACHE_CAPACITY,
                    );
                    let result = super::super::messages::send_message(State(state.clone()), Path("low-budget-parent".into()), Json(json!({"role":"tool", "conversation_id":conversation.id, "tool_return":{"tool_call_id":id, "tool_name":"read_file", "content":tool_output(path)}}))).await;
                    assert!(result.status().is_success());
                }
            }
            requests.recv().await.unwrap();
            for (id, path) in [("first", "first.txt"), ("latest", "latest.txt")] {
                let followup =
                    tokio::time::timeout(std::time::Duration::from_secs(3), requests.recv())
                        .await
                        .unwrap()
                        .unwrap();
                let input = followup["input"].as_array().unwrap();
                let anchor = input
                    .iter()
                    .find(|item| item["role"] == "user")
                    .expect("Compacted continuation must retain its user anchor");
                assert!(
                    anchor["content"]
                        .as_str()
                        .unwrap()
                        .starts_with("Read first.txt then latest.txt.")
                );
                assert!(
                    anchor["content"].as_str().unwrap().chars().count() <= 1024,
                    "The user anchor must be bounded, not the entire large user turn"
                );
                let index = input
                    .iter()
                    .position(|item| item == &reasoning(id))
                    .expect("Compaction must retain the latest opaque reasoning item");
                assert_eq!(input[index + 1]["call_id"], id);
                let result = input
                    .iter()
                    .find(|item| item["type"] == "function_call_output" && item["call_id"] == id)
                    .unwrap();
                assert!(
                    result["output"]
                        .as_str()
                        .unwrap()
                        .contains(&format!("result of {path}"))
                );
                for call in input.iter().filter(|item| item["type"] == "function_call") {
                    assert_eq!(
                        input
                            .iter()
                            .filter(|item| item["type"] == "function_call_output"
                                && item["call_id"] == call["call_id"])
                            .count(),
                        1,
                        "Every retained parallel call needs exactly one result"
                    );
                }
                if large_output && id == "first" {
                    let parallel = input
                        .iter()
                        .find(|item| {
                            item["type"] == "function_call_output" && item["call_id"] == "parallel"
                        })
                        .unwrap();
                    assert!(
                        parallel["output"]
                            .as_str()
                            .unwrap()
                            .contains("result of parallel.txt")
                    );
                }
            }
            let telemetry = state
                .agent_context_telemetry
                .read()
                .await
                .get("low-budget-parent")
                .unwrap()
                .clone();
            if large_output {
                let rows = sqlite::get_context_window(
                    &state.db,
                    "low-budget-parent",
                    Some(&conversation.id),
                    8_000,
                )
                .unwrap();
                assert_eq!(
                    rows[0].role, "tool",
                    "Fixture must exercise a DB cut inside the parallel exchange"
                );
                assert!(!rows.iter().any(|row| row.role == "user"));
            } else {
                assert!(
                    telemetry.turns_omitted > 0,
                    "Fixture must exercise actual inline compaction"
                );
            }
            assert!(
                telemetry.history_tokens.saturating_mul(3) <= telemetry.message_budget_chars,
                "Retained anchor and tool chain must fit the fixture budget: {telemetry:?}"
            );
        }
    }
}

#[tokio::test]
async fn responses_incomplete_turn_is_not_persisted_or_executed_by_run() {
    let workspace = tempfile::tempdir().unwrap();
    let first = json!({"status":"incomplete", "incomplete_details":{"reason":"max_output_tokens"}, "output":[
        {"type":"function_call", "call_id":"call_fixture", "name":"write_file", "arguments":"{\"path\":\"must-not-exist.txt\",\"content\":\"partial turn\"}", "status":"incomplete"}
    ]});
    let (provider, mut requests) = fixture(true, first).await;
    let state = super::tests::build_state_with_llm(provider);
    sqlite::create_agent(
        &state.db,
        &sqlite::AgentRow {
            id: "incomplete-parent".into(),
            name: "Incomplete response".into(),
            model: "openai/gpt-6-sol".into(),
            description: None,
            system_prompt: None,
            created_at: None,
            compaction_model: None,
            theme: None,
            active_plan_json: None,
            parent_id: None,
        },
    )
    .unwrap();
    let conversation =
        sqlite::create_conversation(&state.db, "incomplete-parent", "incomplete turn").unwrap();
    let mut handle = runtime::ServerAgentRuntime::new(state.clone())
        .start_with_options(
            runtime::RunRequest {
                agent_id: "incomplete-parent".into(),
                conversation_id: Some(conversation.id.clone()),
                input: "Write the file".into(),
                permission_mode: Some("accept_edits".into()),
            },
            runtime::RunExecutionOptions {
                cwd: Some(workspace.path().to_path_buf()),
                ..Default::default()
            },
        )
        .await
        .unwrap();
    let events = tokio::time::timeout(std::time::Duration::from_secs(20), async {
        let mut events = Vec::new();
        while let Some(Ok(event)) = handle.events.recv().await {
            if event.data != "[DONE]" {
                events.push(serde_json::from_str::<Value>(&event.data).unwrap());
            }
        }
        events
    })
    .await
    .unwrap();
    assert!(events.iter().any(|event| event["message_type"] == "error"));
    assert!(!events.iter().any(|event| matches!(
        event["message_type"].as_str(),
        Some("tool_call_message" | "tool_result_message")
    )));
    assert!(!workspace.path().join("must-not-exist.txt").exists());
    let generated_rows: i64 = state.db.get().unwrap().query_row("SELECT COUNT(*) FROM messages WHERE conversation_id = ?1 AND role IN ('assistant', 'tool')", [&conversation.id], |row| row.get(0)).unwrap();
    assert_eq!(generated_rows, 0);
    assert!(requests.recv().await.is_some());
    assert!(
        requests.try_recv().is_err(),
        "Failed turns must not continue generation"
    );
}

#[tokio::test]
async fn responses_continuation_survives_persisted_conversation_for_stream_and_blocking() {
    for model in ["openai/gpt-5", "openai/gpt-6-sol"] {
        for streaming in [false, true] {
            let workspace = tempfile::tempdir().unwrap();
            std::fs::write(
                workspace.path().join("fixture.txt"),
                "persisted fixture result",
            )
            .unwrap();
            let reasoning = json!({"type":"reasoning", "id":"rs_fixture", "summary":[], "encrypted_content":"opaque-fixture"});
            let first = json!({"status":"completed", "output":[reasoning, {"type":"function_call", "call_id":"call_fixture", "name":"read_file", "arguments":"{\"path\":\"fixture.txt\"}", "status":"completed"}]});
            let (provider, mut requests) = fixture(streaming, first).await;
            let state = super::tests::build_state_with_llm(provider);
            sqlite::create_agent(
                &state.db,
                &sqlite::AgentRow {
                    id: "responses-parent".into(),
                    name: "Responses persistence".into(),
                    model: model.into(),
                    description: None,
                    system_prompt: None,
                    created_at: None,
                    compaction_model: None,
                    theme: None,
                    active_plan_json: None,
                    parent_id: None,
                },
            )
            .unwrap();
            let conversation =
                sqlite::create_conversation(&state.db, "responses-parent", "tool replay").unwrap();
            if streaming {
                let mut handle = runtime::ServerAgentRuntime::new(state.clone())
                    .start_with_options(
                        runtime::RunRequest {
                            agent_id: "responses-parent".into(),
                            conversation_id: Some(conversation.id.clone()),
                            input: "Read fixture.txt".into(),
                            permission_mode: Some("plan".into()),
                        },
                        runtime::RunExecutionOptions {
                            cwd: Some(workspace.path().to_path_buf()),
                            ..Default::default()
                        },
                    )
                    .await
                    .unwrap();
                let events = tokio::time::timeout(std::time::Duration::from_secs(20), async {
                    let mut events = Vec::new();
                    while let Some(Ok(event)) = handle.events.recv().await {
                        events.push(event.data);
                    }
                    events
                })
                .await
                .expect("Run must finish");
                assert!(
                    events
                        .iter()
                        .any(|event| event.contains("persisted fixture result")),
                    "{events:?}"
                );
                assert!(
                    !events.iter().any(|event| event.contains("opaque-fixture")),
                    "Opaque continuation must not be visible stream text"
                );
            } else {
                let result = super::super::messages::send_message(
                    State(state.clone()),
                    Path("responses-parent".into()),
                    Json(json!({"input":"Read fixture.txt", "conversation_id":conversation.id})),
                )
                .await;
                assert!(result.status().is_success());
                *state.context_cache.lock() = crate::server::state::SafeLruCache::new(
                    crate::server::state::CONTEXT_CACHE_CAPACITY,
                );
                let result = super::super::messages::send_message(State(state.clone()), Path("responses-parent".into()), Json(json!({"role":"tool", "conversation_id":conversation.id, "tool_return":{"tool_call_id":"call_fixture", "tool_name":"read_file", "content":"persisted fixture result"}}))).await;
                assert!(result.status().is_success());
            }
            let initial = requests.recv().await.unwrap();
            assert_eq!(initial["model"], model.strip_prefix("openai/").unwrap());
            let followup = tokio::time::timeout(std::time::Duration::from_secs(3), requests.recv())
                .await
                .unwrap()
                .unwrap();
            let input = followup["input"].as_array().unwrap();
            let index = input
                .iter()
                .position(|item| item["type"] == "reasoning")
                .expect("Persisted reasoning item must reach next provider request");
            assert_eq!(input[index], reasoning, "{model}, streaming={streaming}");
            assert_eq!(input[index + 1]["call_id"], "call_fixture");
            assert_eq!(input[index + 2]["type"], "function_call_output");
            assert!(
                input[index + 2]["output"]
                    .as_str()
                    .unwrap()
                    .contains("persisted fixture result")
            );
            let stored: String = state.db.get().unwrap().query_row("SELECT content FROM messages WHERE conversation_id = ?1 AND role = 'assistant' ORDER BY history_seq LIMIT 1", [&conversation.id], |row| row.get(0)).unwrap();
            let stored: Value = serde_json::from_str(&stored).unwrap();
            assert!(
                stored["tool_calls"][0]["thought_signature"]
                    .as_str()
                    .unwrap()
                    .contains("opaque-fixture")
            );
            assert!(
                !stored["content"]
                    .as_str()
                    .unwrap()
                    .contains("opaque-fixture")
            );
        }
    }
}
