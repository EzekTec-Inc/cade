//! Contract tests use loopback HTTP only: no provider credentials or external services.
use cade_ai::{
    AiConfig, CompletionRequest, ConcurrentRouter, LlmMessage, LlmProvider, LlmRouter,
    MessageImage, SharedModelRegistry, StreamChunk,
    openai::{ApiProtocol, ReasoningStrategy, TokenParameter},
    runtime::{ModelMetadata, RegisteredModel, RuntimeRegistry},
};
use futures::StreamExt;
use serde_json::{Value, json};
use std::sync::Arc;
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::TcpListener,
    sync::{Notify, mpsc},
};

struct Reply {
    status: u16,
    content_type: &'static str,
    body: String,
    gate: Option<Arc<Notify>>,
}
impl Reply {
    fn json(body: Value) -> Self {
        Self {
            status: 200,
            content_type: "application/json",
            body: body.to_string(),
            gate: None,
        }
    }
    fn sse(events: &[Value]) -> Self {
        Self {
            status: 200,
            content_type: "text/event-stream",
            body: events.iter().map(|e| format!("data:{e}\r\n\r\n")).collect(),
            gate: None,
        }
    }
    fn denied() -> Self {
        Self {
            status: 403,
            ..Self::json(json!({"error": {"message": "fixture denied"}}))
        }
    }
}
#[derive(Debug)]
struct Captured {
    method: String,
    target: String,
    headers: String,
    body: Value,
}
struct HttpFixture {
    base: String,
    captured: mpsc::UnboundedReceiver<Captured>,
    task: tokio::task::JoinHandle<()>,
}
impl HttpFixture {
    async fn start(replies: Vec<Reply>) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let base = format!("http://{}", listener.local_addr().unwrap());
        let (tx, captured) = mpsc::unbounded_channel();
        let task = tokio::spawn(async move {
            for reply in replies {
                let (mut socket, _) = listener.accept().await.unwrap();
                let mut bytes = Vec::new();
                let header_end;
                loop {
                    let mut chunk = [0; 4096];
                    let read = socket.read(&mut chunk).await.unwrap();
                    assert!(read > 0, "request ended before headers");
                    bytes.extend_from_slice(&chunk[..read]);
                    if let Some(end) = bytes.windows(4).position(|s| s == b"\r\n\r\n") {
                        header_end = end + 4;
                        break;
                    }
                }
                let headers = String::from_utf8(bytes[..header_end].to_vec()).unwrap();
                let length: usize = headers
                    .lines()
                    .find_map(|line| {
                        let (key, value) = line.split_once(':')?;
                        key.eq_ignore_ascii_case("content-length")
                            .then(|| value.trim().parse().unwrap())
                    })
                    .unwrap_or(0);
                while bytes.len() < header_end + length {
                    let mut chunk = [0; 4096];
                    let read = socket.read(&mut chunk).await.unwrap();
                    assert!(read > 0, "request ended before body");
                    bytes.extend_from_slice(&chunk[..read]);
                }
                let mut request_line = headers.lines().next().unwrap().split_whitespace();
                let method = request_line.next().unwrap().into();
                let target = request_line.next().unwrap().into();
                let body = if length == 0 {
                    Value::Null
                } else {
                    serde_json::from_slice(&bytes[header_end..header_end + length]).unwrap()
                };
                tx.send(Captured {
                    method,
                    target,
                    headers,
                    body,
                })
                .unwrap();
                if let Some(gate) = reply.gate {
                    gate.notified().await;
                }
                let headers = format!(
                    "HTTP/1.1 {} Fixture\r\nContent-Type: {}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                    reply.status,
                    reply.content_type,
                    reply.body.len()
                );
                socket.write_all(headers.as_bytes()).await.unwrap();
                // Fragment inside SSE JSON/UTF-8 to exercise the actual byte decoder.
                for bytes in reply.body.as_bytes().chunks(13) {
                    if socket.write_all(bytes).await.is_err() {
                        break;
                    }
                    tokio::task::yield_now().await;
                }
            }
        });
        Self {
            base,
            captured,
            task,
        }
    }
    async fn request(&mut self) -> Captured {
        tokio::time::timeout(std::time::Duration::from_secs(3), self.captured.recv())
            .await
            .unwrap()
            .unwrap()
    }
    async fn finish(self) {
        tokio::time::timeout(std::time::Duration::from_secs(3), self.task)
            .await
            .unwrap()
            .unwrap();
    }
}

fn config(provider: &str) -> AiConfig {
    AiConfig {
        anthropic_api_key: None,
        openai_api_key: None,
        google_api_key: None,
        deepseek_api_key: None,
        ollama_base_url: "http://127.0.0.1:1".into(),
        llm_provider: provider.into(),
    }
}
fn models() -> SharedModelRegistry {
    Arc::new(parking_lot::RwLock::new(RuntimeRegistry::default()))
}
fn request(model: &str) -> CompletionRequest {
    CompletionRequest {
        model: model.into(),
        messages: vec![LlmMessage {
            role: "user".into(),
            content: "hello".into(),
            tool_call_id: None,
            tool_calls: None,
            images: None,
            cache_control: None,
        }],
        tools: Vec::new(),
        max_tokens: 321,
        reasoning_effort: None,
    }
}
fn registered(id: &str, metadata: ModelMetadata) -> RegisteredModel {
    RegisteredModel {
        id: id.into(),
        aliases: Vec::new(),
        metadata,
    }
}
fn responses_metadata() -> ModelMetadata {
    ModelMetadata {
        protocol: Some(ApiProtocol::Responses),
        token_parameter: Some(TokenParameter::MaxOutputTokens),
        reasoning: Some(ReasoningStrategy::NestedReasoningObject),
        tools: Some(true),
        native_structured: Some(true),
        ..Default::default()
    }
}
fn response(text: &str) -> Value {
    json!({"status": "completed", "output": [{"type": "message", "content": [{"type": "output_text", "text": text}]}]})
}

#[tokio::test]
async fn registered_responses_complete_preserves_nested_ids_images_and_final_calls() {
    let mut http = HttpFixture::start(vec![Reply::json(json!({
        "status": "incomplete", "incomplete_details": {"reason": "max_output_tokens"},
        "output": [
            {"type": "message", "content": [{"type": "output_text", "text": "one"}, {"type": "output_text", "text": "two"}]},
            {"type": "function_call", "id": "item_id", "call_id": "replay_id", "name": "inspect", "arguments": "{\"path\":\"src\"}", "status":"completed"}
        ]
    }))]).await;
    let mut router = LlmRouter::empty("private".into(), models());
    router.register_model(registered(
        "private/tenant/deployment",
        responses_metadata(),
    ));
    assert!(router.add_configured_provider(
        "private",
        "openai-compatible",
        Some("test-key".into()),
        Some(format!("{}/v9?tenant=fixture", http.base)),
        &config("private")
    ));
    let mut req = request("private/tenant/deployment");
    req.messages[0].images = Some(vec![MessageImage {
        media_type: "image/png".into(),
        data: "AA==".into(),
    }]);
    req.reasoning_effort = Some("high".into());
    req.tools = vec![json!({"name": "inspect", "parameters": {"type": "object"}})];
    let result = router.complete(&req).await.unwrap();
    assert_eq!(result.content.as_deref(), Some("onetwo"));
    assert_eq!(result.finish_reason, "max_output_tokens");
    assert_eq!(result.tool_calls[0].id, "replay_id");
    assert_eq!(result.tool_calls[0].arguments["path"], "src");
    let captured = http.request().await;
    assert_eq!(captured.target, "/v9/responses?tenant=fixture");
    assert_eq!(captured.body["model"], "tenant/deployment");
    assert_eq!(captured.body["max_output_tokens"], 321);
    assert_eq!(
        captured.body["input"][0]["content"][0]["type"],
        "input_image"
    );
    assert_eq!(
        captured.body["input"][0]["content"][1]["type"],
        "input_text"
    );
    assert_eq!(captured.body["tools"][0]["name"], "inspect");
    assert_eq!(captured.body["reasoning"]["effort"], "high");
    assert!(captured.body.get("messages").is_none());
    http.finish().await;
}

#[tokio::test]
async fn registered_responses_structured_without_tools_uses_same_protocol() {
    let mut http = HttpFixture::start(vec![Reply::json(response("{\"answer\":42}"))]).await;
    let registry = models();
    registry.write().register(registered(
        "custom/not-a-known-family",
        responses_metadata(),
    ));
    let provider =
        cade_ai::openai::OpenAiProvider::new("fixture".into(), Some(format!("{}/v1", http.base)))
            .with_registry("custom".into(), registry);
    let result = provider
        .complete_structured(
            &request("not-a-known-family"),
            json!({"type": "object", "properties": {"answer": {"type": "integer"}}}),
        )
        .await
        .unwrap();
    assert_eq!(result["answer"], 42);
    let captured = http.request().await;
    assert_eq!(captured.target, "/v1/responses");
    assert_eq!(captured.body["text"]["format"]["type"], "json_schema");
    assert!(
        captured.body["text"]["format"]["schema"]["required"]
            .as_array()
            .unwrap()
            .contains(&json!("answer"))
    );
    assert!(captured.body.get("response_format").is_none());
    http.finish().await;
}

#[tokio::test]
async fn responses_stream_terminal_event_pairs_final_arguments_usage_and_reasoning() {
    let mut http = HttpFixture::start(vec![Reply::sse(&[
        json!({"type": "response.reasoning_summary_text.delta", "delta": "réfléchir"}),
        json!({"type": "response.output_text.delta", "delta": "answer"}),
        json!({"type": "response.output_item.added", "output_index": 0, "item": {"type": "function_call", "call_id": "call_a", "name": "inspect", "arguments": ""}}),
        json!({"type": "response.function_call_arguments.delta", "output_index": 0, "delta": "{\"path\":"}),
        json!({"type": "response.output_item.done", "output_index": 0, "item": {"type": "function_call", "call_id": "call_a", "name": "inspect", "arguments": "{\"path\":\"final\"}"}}),
        json!({"type": "response.completed", "response": {"status": "completed", "usage": {"input_tokens": 10, "output_tokens": 3, "input_tokens_details": {"cached_tokens": 2}},
            "output": [{"type": "function_call", "call_id": "call_a", "name": "inspect", "arguments": "{\"path\":\"final\"}"}]}}),
    ])]).await;
    let provider =
        cade_ai::openai::OpenAiProvider::new("".into(), Some(format!("{}/responses", http.base)));
    let chunks = provider
        .stream(&request("arbitrary"))
        .await
        .unwrap()
        .collect::<Vec<_>>()
        .await
        .into_iter()
        .collect::<cade_ai::Result<Vec<_>>>()
        .unwrap();
    assert!(
        chunks
            .iter()
            .any(|c| matches!(c, StreamChunk::Reasoning(s) if s == "réfléchir"))
    );
    let calls: Vec<_> = chunks
        .iter()
        .filter_map(|c| {
            if let StreamChunk::ToolCall(call) = c {
                Some(call)
            } else {
                None
            }
        })
        .collect();
    assert_eq!(calls.len(), 1);
    assert_eq!(calls[0].arguments["path"], "final");
    assert_eq!(calls[0].id, "call_a");
    assert!(chunks.iter().any(|c| matches!(c, StreamChunk::Usage(u) if u.input_tokens == 8 && u.cache_read_tokens == 2 && u.output_tokens == 3)));
    assert_eq!(
        chunks
            .iter()
            .filter(|c| matches!(c, StreamChunk::Done))
            .count(),
        1
    );
    assert!(matches!(chunks.last(), Some(StreamChunk::Done)));
    assert_eq!(http.request().await.target, "/responses");
    http.finish().await;
}

#[tokio::test]
async fn responses_stream_errors_are_not_successful_done_or_cross_protocol_text() {
    for event in [
        json!({"type": "response.failed", "response": {"error": {"message": "blocked"}}}),
        json!({"choices": [{"delta": {"content": "wrong protocol"}}]}),
    ] {
        let mut http = HttpFixture::start(vec![Reply::sse(&[event])]).await;
        let provider = cade_ai::openai::OpenAiProvider::new(
            "".into(),
            Some(format!("{}/responses", http.base)),
        );
        let chunks = provider
            .stream(&request("anything"))
            .await
            .unwrap()
            .collect::<Vec<_>>()
            .await;
        assert_eq!(chunks.len(), 1);
        assert!(chunks[0].is_err());
        http.request().await;
        http.finish().await;
    }
}

#[tokio::test]
async fn responses_replay_preserves_opaque_reasoning_for_gpt5_and_gpt6() {
    for model in ["gpt-5", "gpt-6-sol"] {
        for streaming in [false, true] {
            let reasoning = json!({"type":"reasoning", "id":"rs_fixture", "summary":[], "encrypted_content":"opaque-fixture"});
            let call = json!({"type":"function_call", "id":"fc_fixture", "call_id":"call_fixture", "name":"inspect", "arguments":"{\"path\":\"src\"}", "status":"completed"});
            let first = json!({"status":"completed", "output":[reasoning, call]});
            let reply = if streaming {
                Reply::sse(&[
                    json!({"type":"response.output_item.done", "output_index":0, "item":reasoning}),
                    json!({"type":"response.output_item.done", "output_index":1, "item":call}),
                    json!({"type":"response.completed", "response":first}),
                ])
            } else {
                Reply::json(first)
            };
            let mut http = HttpFixture::start(vec![reply, Reply::json(response("done"))]).await;
            let provider =
                cade_ai::openai::OpenAiProvider::new("".into(), Some(format!("{}/v1", http.base)))
                    .with_registry("openai".into(), models());
            let mut req = request(model);
            req.tools = vec![json!({"name":"inspect", "parameters":{"type":"object"}})];
            req.reasoning_effort = Some("high".into());
            let calls = if streaming {
                provider
                    .stream(&req)
                    .await
                    .unwrap()
                    .collect::<Vec<_>>()
                    .await
                    .into_iter()
                    .collect::<cade_ai::Result<Vec<_>>>()
                    .unwrap()
                    .into_iter()
                    .filter_map(|chunk| match chunk {
                        StreamChunk::ToolCall(call) => Some(call),
                        _ => None,
                    })
                    .collect()
            } else {
                provider.complete(&req).await.unwrap().tool_calls
            };
            let mut assistant = req.messages[0].clone();
            assistant.role = "assistant".into();
            assistant.content.clear();
            assistant.tool_calls = Some(calls);
            // Public serialized message roundtrip, without knowing the opaque envelope format.
            req.messages
                .push(serde_json::from_value(serde_json::to_value(assistant).unwrap()).unwrap());
            let mut result = req.messages[0].clone();
            result.role = "tool".into();
            result.content = "fixture result".into();
            result.tool_call_id = Some("call_fixture".into());
            req.messages.push(result);
            provider.complete(&req).await.unwrap();
            let initial = http.request().await;
            assert_eq!(initial.target, "/v1/responses");
            let followup = http.request().await;
            assert_eq!(
                followup.body["input"][1], reasoning,
                "{model}, streaming={streaming}"
            );
            assert_eq!(followup.body["input"][2]["call_id"], "call_fixture");
            assert_eq!(followup.body["input"][2]["id"], "fc_fixture");
            assert_eq!(followup.body["input"][3]["type"], "function_call_output");
            assert!(
                !followup.body["input"][0]["content"]
                    .as_str()
                    .unwrap()
                    .contains("opaque-fixture")
            );
            http.finish().await;
        }
    }
}

#[tokio::test]
async fn responses_incomplete_calls_and_sentinel_never_dispatch() {
    for model in ["gpt-5", "gpt-6-sol"] {
        let item = json!({"type":"function_call", "call_id":"call_a", "name":"inspect", "arguments":"", "status":"incomplete"});
        let incomplete = json!({"status":"incomplete", "incomplete_details":{"reason":"max_output_tokens"}, "output":[item]});
        let replies = [
            Reply::json(incomplete.clone()),
            Reply::sse(&[json!({"type":"response.incomplete", "response":incomplete})]),
            Reply::sse(&[
                json!({"type":"response.output_item.done", "output_index":0, "item":item}),
                json!({"type":"response.incomplete", "response":incomplete}),
            ]),
            Reply {
                body: format!(
                    "data: {}\n\ndata: [DONE]\n\n",
                    json!({"type":"response.output_item.added", "output_index":0, "item":{"type":"function_call", "call_id":"call_a", "name":"inspect", "arguments":"{}", "status":"in_progress"}})
                ),
                ..Reply::sse(&[])
            },
            Reply::sse(&[
                json!({"type":"response.output_item.done", "output_index":0, "item":{"type":"function_call", "call_id":"call_a", "name":"inspect", "arguments":"{}", "status":"completed"}}),
                json!({"type":"response.failed", "response":{"status":"failed", "error":{"message":"fixture failure"}}}),
            ]),
        ];
        for (index, reply) in replies.into_iter().enumerate() {
            let mut http = HttpFixture::start(vec![reply]).await;
            let provider =
                cade_ai::openai::OpenAiProvider::new("".into(), Some(format!("{}/v1", http.base)))
                    .with_registry("openai".into(), models());
            if index == 0 {
                assert!(provider.complete(&request(model)).await.is_err());
            } else {
                let chunks = provider
                    .stream(&request(model))
                    .await
                    .unwrap()
                    .collect::<Vec<_>>()
                    .await;
                assert!(
                    chunks.iter().any(Result::is_err),
                    "{model}, case {index}: {chunks:?}"
                );
                assert!(
                    !chunks
                        .iter()
                        .any(|c| matches!(c, Ok(StreamChunk::ToolCall(_) | StreamChunk::Done))),
                    "{model}, case {index}: {chunks:?}"
                );
            }
            http.request().await;
            http.finish().await;
        }
    }
}

#[tokio::test]
async fn responses_parallel_delta_fallback_replays_reasoning_in_output_order() {
    let reasoning_a =
        json!({"type":"reasoning", "id":"rs_a", "summary":[], "encrypted_content":"opaque-a"});
    let reasoning_b =
        json!({"type":"reasoning", "id":"rs_b", "summary":[], "encrypted_content":"opaque-b"});
    let reasoning_tail = json!({"type":"reasoning", "id":"rs_tail", "summary":[]});
    let mut http = HttpFixture::start(vec![Reply::sse(&[
        json!({"type":"response.output_item.added", "output_index":0, "item":{"type":"reasoning", "id":"rs_a", "summary":[], "status":"in_progress"}}),
        json!({"type":"response.output_item.done", "output_index":0, "item":reasoning_a}),
        json!({"type":"response.output_item.added", "output_index":1, "item":{"type":"function_call", "id":"fc_a", "call_id":"call_a", "name":"inspect", "arguments":""}}),
        json!({"type":"response.function_call_arguments.delta", "output_index":1, "delta":"{\"path\":"}),
        json!({"type":"response.function_call_arguments.done", "output_index":1, "arguments":"{\"path\":\"a\"}"}),
        json!({"type":"response.output_item.done", "output_index":1, "item":{"type":"function_call", "call_id":"call_a"}}),
        json!({"type":"response.output_item.done", "output_index":2, "item":reasoning_b}),
        json!({"type":"response.output_item.done", "output_index":3, "item":{"type":"function_call", "id":"fc_b", "call_id":"call_b", "name":"inspect", "arguments":"{\"path\":\"b\"}"}}),
        json!({"type":"response.output_item.done", "output_index":4, "item":reasoning_tail}),
        // Compatibility terminal without output; completed items are the fallback.
        json!({"type":"response.done", "response":{"status":"completed"}}),
    ]), Reply::json(response("done"))]).await;
    let provider =
        cade_ai::openai::OpenAiProvider::new("".into(), Some(format!("{}/responses", http.base)));
    let mut req = request("gpt-6-sol");
    let chunks = provider
        .stream(&req)
        .await
        .unwrap()
        .collect::<Vec<_>>()
        .await
        .into_iter()
        .collect::<cade_ai::Result<Vec<_>>>()
        .unwrap();
    let calls: Vec<_> = chunks
        .into_iter()
        .filter_map(|chunk| match chunk {
            StreamChunk::ToolCall(call) => Some(call),
            _ => None,
        })
        .collect();
    assert_eq!(calls.len(), 2);
    assert_eq!(calls[0].arguments["path"], "a");
    assert_eq!(calls[1].arguments["path"], "b");
    let mut assistant = req.messages[0].clone();
    assistant.role = "assistant".into();
    assistant.content.clear();
    assistant.tool_calls = Some(calls);
    req.messages.push(assistant);
    let budget = cade_ai::PromptBudgetManager::new();
    let with_continuation = budget.turn_cost("openai/gpt-6-sol", &req.messages);
    let mut without = req.messages.clone();
    for call in without[1].tool_calls.as_mut().unwrap() {
        call.thought_signature = None;
    }
    assert!(with_continuation > budget.turn_cost("openai/gpt-6-sol", &without));
    for id in ["call_a", "call_b"] {
        let mut output = req.messages[0].clone();
        output.role = "tool".into();
        output.content = "result".into();
        output.tool_call_id = Some(id.into());
        req.messages.push(output);
    }
    provider.complete(&req).await.unwrap();
    http.request().await;
    let replay = http.request().await.body;
    // The synthetic completion status belongs to calls, never to opaque reasoning.
    assert_eq!(replay["input"][1], reasoning_a);
    assert_eq!(replay["input"][2]["id"], "fc_a");
    assert_eq!(replay["input"][3], reasoning_b);
    assert_eq!(replay["input"][4]["id"], "fc_b");
    assert_eq!(replay["input"][5], reasoning_tail);
    http.finish().await;
}

#[tokio::test]
async fn responses_invalid_later_call_rejects_the_whole_turn() {
    for args in ["", "{", "null", "[]", "\"text\""] {
        let valid = json!({"type":"function_call", "call_id":"call_a", "name":"inspect", "arguments":"{}", "status":"completed"});
        let invalid = json!({"type":"function_call", "call_id":"call_b", "name":"inspect", "arguments":args, "status":"completed"});
        let mut http = HttpFixture::start(vec![Reply::sse(&[
            json!({"type":"response.output_item.done", "output_index":0, "item":valid}),
            json!({"type":"response.output_item.done", "output_index":1, "item":invalid}),
            json!({"type":"response.completed", "response":{"status":"completed", "output":[valid, invalid]}}),
        ])]).await;
        let provider = cade_ai::openai::OpenAiProvider::new(
            "".into(),
            Some(format!("{}/responses", http.base)),
        );
        let chunks = provider
            .stream(&request("gpt-5"))
            .await
            .unwrap()
            .collect::<Vec<_>>()
            .await;
        assert!(chunks.iter().any(Result::is_err));
        assert!(
            !chunks
                .iter()
                .any(|chunk| matches!(chunk, Ok(StreamChunk::ToolCall(_) | StreamChunk::Done)))
        );
        http.request().await;
        http.finish().await;
    }
}

#[tokio::test]
async fn responses_continuation_is_not_sent_as_a_gemini_signature_after_model_switch() {
    let mut openai = HttpFixture::start(vec![Reply::json(json!({"status":"completed", "output":[
        {"type":"reasoning", "id":"rs_fixture", "summary":[], "encrypted_content":"opaque-fixture"},
        {"type":"function_call", "call_id":"call_a", "name":"inspect", "arguments":"{\"path\":\"src\"}"}
    ]}))]).await;
    let provider =
        cade_ai::openai::OpenAiProvider::new("".into(), Some(format!("{}/responses", openai.base)));
    let calls = provider
        .complete(&request("gpt-5"))
        .await
        .unwrap()
        .tool_calls;
    let mut gemini = HttpFixture::start(vec![Reply::json(
        json!({"candidates":[{"content":{"parts":[{"text":"done"}]}}]}),
    )])
    .await;
    let provider =
        cade_ai::gemini::GeminiProvider::new("fixture".into(), Some(format!("{}/v1", gemini.base)));
    let mut req = request("gemini-2.5-flash");
    let mut assistant = req.messages[0].clone();
    assistant.role = "assistant".into();
    assistant.content.clear();
    assistant.tool_calls = Some(calls);
    req.messages.push(assistant);
    let mut output = req.messages[0].clone();
    output.role = "tool".into();
    output.content = "result".into();
    output.tool_call_id = Some("call_a".into());
    req.messages.push(output);
    provider.complete(&req).await.unwrap();
    let sent = gemini.request().await.body.to_string();
    assert!(sent.contains("skip_thought_signature_validator"));
    assert!(!sent.contains("opaque-fixture"));
    assert!(!sent.contains("cade:openai-responses"));
    openai.request().await;
    openai.finish().await;
    gemini.finish().await;
}

#[tokio::test]
async fn gemini_custom_gateway_complete_stream_and_structured_share_generation_parameters() {
    let mut http = HttpFixture::start(vec![
        Reply::json(json!({"candidates": [{"finishReason": "STOP", "content": {"parts": [{"text": "first"}, {"text": "second"}]}}]})),
        Reply::sse(&[json!({"candidates": [{"finishReason": "STOP", "content": {"parts": [{"text": "stream"}]}}]})]),
        Reply::json(json!({"candidates": [{"content": {"parts": [{"text": "{\"ok\":true}"}]}}]})),
    ]).await;
    let registry = models();
    registry.write().register(registered(
        "custom/opaque-deployment",
        ModelMetadata {
            thinking: Some("budget".into()),
            native_structured: Some(true),
            thinking_budgets: Some([("medium".into(), 128)].into()),
            ..Default::default()
        },
    ));
    let provider = cade_ai::gemini::GeminiProvider::new(
        "key&?=secret".into(),
        Some(format!("{}/v9", http.base)),
    )
    .with_registry("custom".into(), registry);
    let mut req = request("opaque-deployment");
    req.reasoning_effort = Some("medium".into());
    assert_eq!(
        provider.complete(&req).await.unwrap().content.as_deref(),
        Some("firstsecond")
    );
    assert!(
        provider
            .stream(&req)
            .await
            .unwrap()
            .collect::<Vec<_>>()
            .await
            .iter()
            .all(Result::is_ok)
    );
    assert_eq!(
        provider
            .complete_structured(
                &req,
                json!({"type": "object", "properties": {"ok": {"type": "boolean"}}})
            )
            .await
            .unwrap()["ok"],
        true
    );
    for index in 0..3 {
        let captured = http.request().await;
        let url = reqwest::Url::parse(&format!("{}{}", http.base, captured.target)).unwrap();
        assert_eq!(
            url.path(),
            if index == 1 {
                "/v9/models/opaque-deployment:streamGenerateContent"
            } else {
                "/v9/models/opaque-deployment:generateContent"
            }
        );
        assert!(
            url.query_pairs()
                .any(|(k, v)| k == "key" && v == "key&?=secret")
        );
        assert_eq!(
            url.query_pairs().any(|(k, v)| k == "alt" && v == "sse"),
            index == 1
        );
        assert_eq!(captured.body["generationConfig"]["maxOutputTokens"], 321);
        assert_eq!(
            captured.body["generationConfig"]["thinkingConfig"]["thinkingBudget"],
            128
        );
        if index == 2 {
            assert_eq!(
                captured.body["generationConfig"]["responseMimeType"],
                "application/json"
            );
        }
    }
    http.finish().await;
}

#[tokio::test]
async fn concurrent_router_preserves_configured_failover_and_native_structured_delegation() {
    let mut broker =
        HttpFixture::start(vec![Reply::denied(), Reply::denied(), Reply::denied()]).await;
    let mut native = HttpFixture::start(vec![
        Reply::json(response("native")),
        Reply::sse(&[
            json!({"type": "response.output_text.delta", "delta": "native stream"}),
            json!({"type": "response.completed", "response": {"status": "completed", "usage": {"input_tokens": 4, "output_tokens": 2}}}),
        ]),
        Reply::json(response("{\"source\":\"native\"}")),
    ])
    .await;
    let registry = models();
    registry.write().failover_providers = vec!["broker".into()];
    let mut router = LlmRouter::empty("broker".into(), registry);
    router.register_model(registered("native/unfamiliar.7", responses_metadata()));
    assert!(router.add_configured_provider(
        "broker",
        "openai-compatible",
        None,
        Some(format!("{}/chat/completions", broker.base)),
        &config("broker")
    ));
    assert!(router.add_configured_provider(
        "native",
        "openai-compatible",
        None,
        Some(format!("{}/v1", native.base)),
        &config("broker")
    ));
    let adapter = ConcurrentRouter(Arc::new(tokio::sync::RwLock::new(router)));
    assert!(adapter.validate_model("missing/a-model").is_err());
    assert!(adapter.validate_model("broker/").is_err());
    assert!(adapter.validate_model("broker/native/unfamiliar.7").is_ok());
    let req = request("broker/native/unfamiliar.7");
    assert_eq!(
        adapter.complete(&req).await.unwrap().content.as_deref(),
        Some("native")
    );
    let chunks = adapter
        .stream(&req)
        .await
        .unwrap()
        .collect::<Vec<_>>()
        .await;
    assert!(
        chunks
            .iter()
            .any(|c| matches!(c, Ok(StreamChunk::Text(text)) if text == "native stream"))
    );
    assert!(chunks.iter().any(|chunk| matches!(chunk, Ok(StreamChunk::Usage(usage)) if usage.model == "native/unfamiliar.7")));
    assert_eq!(
        adapter
            .complete_structured(
                &req,
                json!({"type": "object", "properties": {"source": {"type": "string"}}})
            )
            .await
            .unwrap()["source"],
        "native"
    );
    for index in 0..3 {
        assert_eq!(broker.request().await.body["model"], "native/unfamiliar.7");
        let captured = native.request().await;
        assert_eq!(captured.target, "/v1/responses");
        assert_eq!(captured.body["model"], "unfamiliar.7");
        if index == 2 {
            assert_eq!(captured.body["text"]["format"]["type"], "json_schema");
        }
    }
    broker.finish().await;
    native.finish().await;
}

#[tokio::test]
async fn concurrent_router_releases_lock_during_http_and_keeps_inflight_snapshot() {
    let release = Arc::new(Notify::new());
    let mut http = HttpFixture::start(vec![Reply {
        gate: Some(Arc::clone(&release)),
        ..Reply::json(
            json!({"choices": [{"message": {"content": "old provider"}, "finish_reason": "stop"}]}),
        )
    }])
    .await;
    let mut router = LlmRouter::empty("private".into(), models());
    router.add_configured_provider(
        "private",
        "openai-compatible",
        None,
        Some(format!("{}/chat/completions", http.base)),
        &config("private"),
    );
    let shared = Arc::new(tokio::sync::RwLock::new(router));
    let adapter = Arc::new(ConcurrentRouter(Arc::clone(&shared)));
    let completion = tokio::spawn({
        let adapter = Arc::clone(&adapter);
        async move { adapter.complete(&request("private/unknown-model")).await }
    });
    http.request().await;
    {
        let mut writer = tokio::time::timeout(std::time::Duration::from_secs(1), shared.write())
            .await
            .expect("network I/O held the router lock");
        assert!(writer.remove_provider("private"));
        assert!(adapter.validate_model("private/unknown-model").is_err());
    }
    release.notify_one();
    assert_eq!(
        completion.await.unwrap().unwrap().content.as_deref(),
        Some("old provider")
    );
    assert!(adapter.validate_model("private/unknown-model").is_err());
    http.finish().await;
}

#[tokio::test]
async fn discovery_uses_custom_gateways_pagination_and_actual_registered_limits() {
    let mut openai = HttpFixture::start(vec![Reply::json(
        json!({"data": [{"id": "tenant/unfamiliar-model"}, {"id": "brand-new-name"}]}),
    )])
    .await;
    let mut gemini = HttpFixture::start(vec![Reply::json(json!({"models": [{"name": "models/new-family", "supportedGenerationMethods": ["generateContent"], "inputTokenLimit": 77777, "outputTokenLimit": 1234}], "nextPageToken": "next&token"})),
        Reply::json(json!({"models": [{"name": "models/embed-only", "supportedGenerationMethods": ["embedContent"]}]}))]).await;
    let mut anthropic = HttpFixture::start(vec![Reply::json(
        json!({"data": [{"id": "not-claude", "display_name": "Private deployment"}]}),
    )])
    .await;
    let registry = models();
    let mut router = LlmRouter::empty("private".into(), Arc::clone(&registry));
    router.register_model(registered(
        "private/tenant/unfamiliar-model",
        ModelMetadata {
            context_window: Some(55555),
            max_tokens: Some(999),
            tools: Some(false),
            ..Default::default()
        },
    ));
    for (name, kind, base) in [
        (
            "private",
            "openai-compatible",
            format!("{}/mounted/chat/completions", openai.base),
        ),
        (
            "google-gateway",
            "gemini",
            format!("{}/v9/models", gemini.base),
        ),
        (
            "anthropic-gateway",
            "anthropic",
            format!("{}/tenant/v1/messages", anthropic.base),
        ),
    ] {
        assert!(router.add_configured_provider(
            name,
            kind,
            Some("fixture-key".into()),
            Some(base),
            &config("private")
        ));
    }
    let entries = router.list_dynamic_models().await;
    let known = entries
        .iter()
        .find(|m| m.id == "private/tenant/unfamiliar-model")
        .unwrap();
    assert_eq!(known.max_tokens, 999);
    assert_eq!(known.context_window, 55555);
    let discovered = entries
        .iter()
        .find(|m| m.id == "google-gateway/new-family")
        .unwrap();
    assert_eq!(discovered.max_tokens, 1234);
    assert_eq!(discovered.context_window, 77777);
    assert!(entries.iter().any(|m| m.id == "private/brand-new-name"));
    assert!(!entries.iter().any(|m| m.id.contains("embed-only")));
    assert_eq!(
        registry.read().metadata("private", "brand-new-name").tools,
        None
    );
    assert!(
        !registry
            .read()
            .has_known_limits("private", "brand-new-name")
    );
    assert_eq!(openai.request().await.target, "/mounted/models");
    assert_eq!(gemini.request().await.target, "/v9/models");
    let page = gemini.request().await;
    assert!(page.target.contains("pageToken=next%26token"));
    assert!(
        page.headers
            .to_ascii_lowercase()
            .contains("x-goog-api-key: fixture-key")
    );
    let request = anthropic.request().await;
    assert_eq!(request.method, "GET");
    assert_eq!(request.target, "/tenant/v1/models");
    assert!(
        request
            .headers
            .to_ascii_lowercase()
            .contains("x-api-key: fixture-key")
    );
    openai.finish().await;
    gemini.finish().await;
    anthropic.finish().await;
}

#[tokio::test]
async fn explicit_chat_endpoint_pairs_decoder_and_preserves_provider_named_upstream_id() {
    let mut http = HttpFixture::start(vec![Reply::json(
        json!({"choices": [{"finish_reason": "stop", "message": {"content": "chat"}}]}),
    )])
    .await;
    let mut router = LlmRouter::empty("private".into(), models());
    router.register_model(registered(
        "private/private/opaque",
        ModelMetadata {
            protocol: Some(ApiProtocol::Responses),
            token_parameter: Some(TokenParameter::MaxCompletionTokens),
            ..Default::default()
        },
    ));
    router.add_configured_provider(
        "private",
        "openai-compatible",
        None,
        Some(format!("{}/chat/completions?gateway=fixture", http.base)),
        &config("private"),
    );
    assert_eq!(
        router
            .complete(&request("private/private/opaque"))
            .await
            .unwrap()
            .content
            .as_deref(),
        Some("chat")
    );
    let captured = http.request().await;
    assert_eq!(captured.target, "/chat/completions?gateway=fixture");
    assert_eq!(captured.body["model"], "private/opaque");
    assert_eq!(captured.body["max_completion_tokens"], 321);
    assert!(captured.body.get("messages").is_some());
    assert!(captured.body.get("input").is_none());
    http.finish().await;
}

#[tokio::test]
async fn concurrent_router_does_not_replay_after_stream_output_or_terminal_error() {
    let mut primary = HttpFixture::start(vec![Reply::sse(&[
        json!({"choices": [{"delta": {"content": "partial"}}]}),
        json!({"error": {"message": "stream failed"}}),
    ])])
    .await;
    let native = HttpFixture::start(vec![Reply::json(response("must not be called"))]).await;
    let registry = models();
    registry.write().failover_providers = vec!["broker".into()];
    let mut router = LlmRouter::empty("broker".into(), registry);
    router.add_configured_provider(
        "broker",
        "openai-compatible",
        None,
        Some(format!("{}/chat/completions", primary.base)),
        &config("broker"),
    );
    router.add_configured_provider(
        "native",
        "openai-compatible",
        None,
        Some(format!("{}/responses", native.base)),
        &config("broker"),
    );
    let adapter = ConcurrentRouter(Arc::new(tokio::sync::RwLock::new(router)));
    let chunks = adapter
        .stream(&request("broker/native/opaque"))
        .await
        .unwrap()
        .collect::<Vec<_>>()
        .await;
    assert!(matches!(&chunks[0], Ok(StreamChunk::Text(text)) if text == "partial"));
    assert!(chunks.last().unwrap().is_err());
    assert!(
        !chunks
            .iter()
            .any(|chunk| matches!(chunk, Ok(StreamChunk::Done)))
    );
    primary.request().await;
    assert!(native.captured.is_empty());
    native.task.abort();
    primary.finish().await;
}

#[tokio::test]
async fn editable_registered_aliases_override_family_routes_and_explicit_tools_false_rejects_locally()
 {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("models.json");
    std::fs::write(&path, json!({"fallback": {"max_tokens": 512, "context_window": 4096}, "models": [
        {"id": "private/tenant/custom-id", "aliases": ["gpt-4o", "friendly-name"], "tools": false, "context_window": 123456, "max_tokens": 777},
        {"id": "deepseek/deepseek-reasoner", "tools": true}
    ]}).to_string()).unwrap();
    let registry = Arc::new(parking_lot::RwLock::new(RuntimeRegistry::load(Some(&path))));
    let mut router = LlmRouter::empty("private".into(), registry);
    assert!(router.add_configured_provider(
        "private",
        "openai-compatible",
        None,
        Some("http://127.0.0.1:1/chat/completions".into()),
        &config("private")
    ));
    assert_eq!(
        router.resolve_provider_name("gpt-4o").unwrap(),
        ("private".into(), "tenant/custom-id".into())
    );
    let mut req = request("friendly-name");
    req.tools = vec![json!({"name": "inspect", "parameters": {"type": "object"}})];
    assert!(
        router
            .complete(&req)
            .await
            .unwrap_err()
            .to_string()
            .contains("without tool support")
    );
    assert!(
        router
            .validate_model("private/another-unlisted-model")
            .is_ok()
    );
    assert!(router.validate_model("private/has\ncontrol").is_err());
    let mut http = HttpFixture::start(vec![Reply::json(json!({"choices": [{"finish_reason": "stop", "message": {"content": "configured capability"}}]}))]).await;
    router.add_configured_provider(
        "deepseek",
        "openai-compatible",
        Some("fixture-key".into()),
        Some(format!("{}/chat/completions", http.base)),
        &config("private"),
    );
    req.model = "deepseek/deepseek-reasoner".into();
    assert!(router.complete(&req).await.is_ok());
    assert_eq!(
        http.request().await.body["tools"][0]["function"]["name"],
        "inspect"
    );
    http.finish().await;
}

#[tokio::test]
async fn configured_capability_booleans_aliases_and_headers_control_real_transport() {
    let mut http = HttpFixture::start(vec![
        Reply::json(response("{\"ok\":true}")),
        Reply::json(response("{\"ok\":true}")),
        Reply::json(json!({"data":[{"id":"office/tenant/deployment-0125"}]})),
    ])
    .await;
    let providers = cade_ai::provider_registry::ProviderRegistry::from_json(
        &json!([{
            "name":"office", "aliases":["office-alt"], "kind":"openai-compatible",
            "chat_url":format!("{}/v1", http.base), "display_name":"Office fixture",
            "headers":{"X-Deployment":"fixture"}
        }])
        .to_string(),
    )
    .unwrap();
    let registry = RuntimeRegistry::from_json(
        &json!({
            "rules":[{"providers":["office"],"prefixes":[], "protocol":"responses",
                "tools":true, "native_structured":true, "developer_role":true}],
            "models":[{"id":"office/office/tenant/deployment-0125", "aliases":["friendly"],
                "tools":false,"native_structured":false,"developer_role":false,
                "preview_gateway":false,"tokenizer":"characters","chars_per_token":2}]
        })
        .to_string(),
    )
    .unwrap();
    let shared = Arc::new(parking_lot::RwLock::new(registry));
    let mut router =
        LlmRouter::empty("office".into(), Arc::clone(&shared)).with_provider_registry(providers);
    assert!(router.add_configured_provider(
        "office-alt",
        "openai-compatible",
        None,
        None,
        &config("office")
    ));
    assert_eq!(router.provider_names(), ["office", "office-alt"]);
    let schema = json!({"type":"object","properties":{"ok":{"type":"boolean"}}});
    let req = request("office-alt/office/tenant/deployment-0125");
    assert_eq!(
        router
            .complete_structured(&req, schema.clone())
            .await
            .unwrap()["ok"],
        true
    );
    let captured = http.request().await;
    assert_eq!(captured.target, "/v1/responses");
    assert_eq!(captured.body["model"], "office/tenant/deployment-0125");
    assert_eq!(captured.body["input"][0]["role"], "system");
    assert!(captured.body.get("text").is_none());
    assert!(
        captured
            .headers
            .to_ascii_lowercase()
            .contains("x-deployment: fixture")
    );
    let mut tool_request = request("friendly");
    tool_request.messages.insert(
        0,
        LlmMessage {
            role: "system".into(),
            content: "Keep provenance".into(),
            tool_call_id: None,
            tool_calls: None,
            images: None,
            cache_control: None,
        },
    );
    tool_request.tools = vec![json!({"name":"inspect","parameters":{"type":"object"}})];
    assert!(
        router
            .complete(&tool_request)
            .await
            .unwrap_err()
            .to_string()
            .contains("without tool support")
    );
    assert!(http.captured.is_empty());
    let mut updated = shared
        .read()
        .models
        .iter()
        .find(|m| m.id == "office/office/tenant/deployment-0125")
        .unwrap()
        .clone();
    updated.metadata.tools = Some(true);
    updated.metadata.native_structured = Some(true);
    updated.metadata.developer_role = Some(true);
    shared.write().try_register(updated).unwrap();
    assert_eq!(
        router
            .complete_structured(&tool_request, schema)
            .await
            .unwrap()["ok"],
        true
    );
    let captured = http.request().await;
    assert_eq!(captured.body["model"], "office/tenant/deployment-0125");
    assert_eq!(captured.body["input"][0]["role"], "developer");
    assert_eq!(captured.body["text"]["format"]["type"], "json_schema");
    assert_eq!(captured.body["tools"][0]["name"], "inspect");
    let models = router.list_dynamic_models().await;
    assert_eq!(models.len(), 1);
    assert_eq!(models[0].id, "office/office/tenant/deployment-0125");
    let captured = http.request().await;
    assert_eq!(captured.target, "/v1/models");
    assert!(
        captured
            .headers
            .to_ascii_lowercase()
            .contains("x-deployment: fixture")
    );
    http.finish().await;
}

#[tokio::test]
async fn configured_provider_label_survives_json_roundtrip_into_transport_error() {
    let mut http = HttpFixture::start(vec![Reply::denied()]).await;
    let configured = cade_ai::provider_registry::ProviderRegistry::from_json(
        &json!([{
            "name":"failure-gateway", "aliases":["failure-alias"], "kind":"openai-compatible",
            "chat_url":format!("{}/v1", http.base), "display_name":"Configured workspace gateway"
        }])
        .to_string(),
    )
    .unwrap();
    let serialized =
        serde_json::to_string(&vec![configured.get("failure-gateway").unwrap()]).unwrap();
    let reloaded = cade_ai::provider_registry::ProviderRegistry::from_json(&serialized).unwrap();
    let mut router =
        LlmRouter::empty("failure-gateway".into(), models()).with_provider_registry(reloaded);
    assert!(router.add_configured_provider(
        "failure-alias",
        "openai-compatible",
        None,
        None,
        &config("failure-gateway")
    ));
    let error = router
        .complete(&request("failure-alias/tenant/deployment"))
        .await
        .unwrap_err();
    match error {
        cade_ai::Error::Provider { status, msg } => {
            assert_eq!(status, 403);
            assert!(msg.starts_with("Configured workspace gateway "), "{msg}");
            assert!(msg.contains("fixture denied"), "{msg}");
            assert!(!msg.contains("openai-compatible"), "{msg}");
        }
        other => panic!("Expected labeled upstream status, got {other}"),
    }
    let captured = http.request().await;
    assert_eq!(captured.target, "/v1/chat/completions");
    assert_eq!(captured.body["model"], "tenant/deployment");
    http.finish().await;
}

#[tokio::test]
async fn anthropic_in_band_error_discards_completed_tools_and_ignores_later_events() {
    let mut http = HttpFixture::start(vec![Reply::sse(&[
        json!({"type":"message_start","message":{"usage":{"input_tokens":9}}}),
        json!({"type":"content_block_start","index":0,"content_block":{"type":"tool_use","id":"call_before_error","name":"inspect","input":{}}}),
        json!({"type":"content_block_delta","index":0,"delta":{"type":"input_json_delta","partial_json":"{\"path\":\"safe\"}"}}),
        json!({"type":"content_block_stop","index":0}),
        json!({"type":"content_block_delta","index":1,"delta":{"type":"text_delta","text":"partial text"}}),
        json!({"type":"error","error":{"type":"overloaded_error","message":"Fixture overloaded after tool generation"}}),
        json!({"type":"content_block_start","index":2,"content_block":{"type":"tool_use","id":"call_after_error","name":"inspect","input":{"path":"never"}}}),
        json!({"type":"content_block_stop","index":2}),
        json!({"type":"message_stop"}),
    ])]).await;
    let provider = cade_ai::anthropic::AnthropicProvider::new(
        "fixture-key".into(),
        Some(format!("{}/gateway/v1", http.base)),
    );
    let chunks = provider
        .stream(&request("opaque-deployment"))
        .await
        .unwrap()
        .collect::<Vec<_>>()
        .await;
    assert!(
        chunks
            .iter()
            .any(|chunk| matches!(chunk, Ok(StreamChunk::Text(text)) if text == "partial text"))
    );
    assert!(
        matches!(chunks.last(), Some(Err(cade_ai::Error::Provider { status:529, msg }))
        if msg.contains("overloaded_error") && msg.contains("Fixture overloaded after tool generation"))
    );
    assert_eq!(chunks.iter().filter(|chunk| chunk.is_err()).count(), 1);
    assert!(
        !chunks
            .iter()
            .any(|chunk| matches!(chunk, Ok(StreamChunk::ToolCall(_) | StreamChunk::Done)))
    );
    let captured = http.request().await;
    assert_eq!(captured.target, "/gateway/v1/messages");
    assert_eq!(captured.body["model"], "opaque-deployment");
    assert_eq!(captured.body["stream"], true);
    http.finish().await;
}

#[tokio::test]
async fn anthropic_premature_eof_is_incomplete_and_never_dispatches_tools() {
    for close_block in [false, true] {
        let mut events = vec![
            json!({"type":"message_start","message":{"usage":{"input_tokens":9}}}),
            json!({"type":"content_block_delta","index":1,"delta":{"type":"text_delta","text":"unfinished"}}),
            json!({"type":"content_block_start","index":0,"content_block":{"type":"tool_use","id":"call_unfinished","name":"inspect","input":{}}}),
            json!({"type":"content_block_delta","index":0,"delta":{"type":"input_json_delta","partial_json":"{\"path\":\"never\"}"}}),
        ];
        if close_block {
            events.push(json!({"type":"content_block_stop","index":0}));
        }
        let mut http = HttpFixture::start(vec![Reply::sse(&events)]).await;
        let provider = cade_ai::anthropic::AnthropicProvider::new(
            "fixture-key".into(),
            Some(format!("{}/v1/messages", http.base)),
        );
        let chunks = provider
            .stream(&request("opaque-deployment"))
            .await
            .unwrap()
            .collect::<Vec<_>>()
            .await;
        assert!(
            chunks
                .iter()
                .any(|chunk| matches!(chunk, Ok(StreamChunk::Text(text)) if text == "unfinished"))
        );
        assert!(
            matches!(chunks.last(), Some(Err(cade_ai::Error::Provider { status:502, msg }))
            if msg.contains("Incomplete") && msg.contains("EOF before message_stop"))
        );
        assert!(
            !chunks
                .iter()
                .any(|chunk| matches!(chunk, Ok(StreamChunk::ToolCall(_) | StreamChunk::Done)))
        );
        assert_eq!(chunks.iter().filter(|chunk| chunk.is_err()).count(), 1);
        assert_eq!(http.request().await.target, "/v1/messages");
        http.finish().await;
    }
}

#[tokio::test]
async fn anthropic_terminal_success_flushes_parallel_tools_usage_and_finish_once() {
    let mut http = HttpFixture::start(vec![Reply::sse(&[
        json!({"type":"message_start","message":{"usage":{"input_tokens":9,"cache_read_input_tokens":2,"cache_creation_input_tokens":1}}}),
        json!({"type":"content_block_start","index":0,"content_block":{"type":"tool_use","id":"call_a","name":"inspect","input":{}}}),
        json!({"type":"content_block_start","index":1,"content_block":{"type":"tool_use","id":"call_b","name":"inspect","input":{"path":"b"}}}),
        json!({"type":"content_block_delta","index":0,"delta":{"type":"input_json_delta","partial_json":"{\"path\":\"a\"}"}}),
        json!({"type":"content_block_stop","index":0}),
        json!({"type":"content_block_delta","index":2,"delta":{"type":"text_delta","text":"complete text"}}),
        json!({"type":"content_block_stop","index":1}),
        json!({"type":"message_delta","usage":{"output_tokens":3}}),
        json!({"type":"message_delta","delta":{"stop_reason":"tool_use"},"usage":{"output_tokens":5}}),
        json!({"type":"message_stop"}),
        json!({"type":"error","error":{"type":"api_error","message":"after terminal event"}}),
    ])]).await;
    let provider = cade_ai::anthropic::AnthropicProvider::new(
        "fixture-key".into(),
        Some(format!("{}/v1", http.base)),
    );
    let chunks = provider
        .stream(&request("opaque-deployment"))
        .await
        .unwrap()
        .collect::<Vec<_>>()
        .await
        .into_iter()
        .collect::<cade_ai::Result<Vec<_>>>()
        .unwrap();
    let calls: Vec<_> = chunks
        .iter()
        .filter_map(|chunk| match chunk {
            StreamChunk::ToolCall(call) => Some(call),
            _ => None,
        })
        .collect();
    assert_eq!(calls.len(), 2);
    assert_eq!(
        (&calls[0].id, &calls[0].arguments["path"]),
        (&"call_a".to_string(), &json!("a"))
    );
    assert_eq!(
        (&calls[1].id, &calls[1].arguments["path"]),
        (&"call_b".to_string(), &json!("b"))
    );
    let text_index = chunks
        .iter()
        .position(|chunk| matches!(chunk, StreamChunk::Text(_)))
        .unwrap();
    let call_index = chunks
        .iter()
        .position(|chunk| matches!(chunk, StreamChunk::ToolCall(_)))
        .unwrap();
    assert!(
        text_index < call_index,
        "completed blocks must not dispatch before the terminal event"
    );
    assert!(chunks.iter().any(|chunk| matches!(chunk, StreamChunk::Usage(usage)
        if usage.input_tokens == 9 && usage.output_tokens == 5 && usage.cache_read_tokens == 2 && usage.cache_write_tokens == 1)));
    assert!(
        chunks.iter().any(
            |chunk| matches!(chunk, StreamChunk::FinishReason(reason) if reason == "tool_use")
        )
    );
    assert_eq!(
        chunks
            .iter()
            .filter(|chunk| matches!(chunk, StreamChunk::Done))
            .count(),
        1
    );
    assert!(matches!(chunks.last(), Some(StreamChunk::Done)));
    assert_eq!(http.request().await.target, "/v1/messages");
    http.finish().await;
}
