use anyhow::{bail, Result};
use async_stream::stream;
use async_trait::async_trait;
use futures::StreamExt;
use reqwest::Client;
use serde_json::{json, Value};
use std::pin::Pin;
use tokio_stream::Stream;

use super::{bare_model, CompletionRequest, CompletionResponse, LlmProvider, LlmToolCall, StreamChunk};

const OPENAI_URL: &str = "https://api.openai.com/v1/chat/completions";

pub struct OpenAiProvider {
    client: Client,
    api_key: String,
    /// Override base URL for OpenAI-compatible endpoints (e.g. Together, Groq)
    base_url: String,
}

impl OpenAiProvider {
    pub fn new(api_key: String, base_url: Option<String>) -> Self {
        Self {
            client: Client::new(),
            api_key,
            base_url: base_url.unwrap_or_else(|| OPENAI_URL.to_string()),
        }
    }

    fn to_openai_messages(req: &CompletionRequest) -> Value {
        let messages: Vec<Value> = req.messages.iter().map(|m| {
            match m.role.as_str() {
                "tool" => json!({
                    "role": "tool",
                    "tool_call_id": m.tool_call_id,
                    "content": m.content
                }),
                "assistant" if m.tool_calls.is_some() => {
                    let tcs: Vec<Value> = m.tool_calls.as_ref().unwrap().iter().map(|tc| json!({
                        "id": tc.id,
                        "type": "function",
                        "function": { "name": tc.name, "arguments": tc.arguments.to_string() }
                    })).collect();
                    json!({"role": "assistant", "content": m.content, "tool_calls": tcs})
                }
                _ => json!({"role": m.role, "content": m.content}),
            }
        }).collect();
        json!(messages)
    }

    fn parse_response(body: &Value) -> CompletionResponse {
        let choice = &body["choices"][0];
        let finish_reason = choice["finish_reason"].as_str().unwrap_or("stop").to_string();
        let msg = &choice["message"];
        let content = msg["content"].as_str().map(|s| s.to_string());
        let tool_calls: Vec<LlmToolCall> = msg["tool_calls"]
            .as_array()
            .unwrap_or(&vec![])
            .iter()
            .map(|tc| LlmToolCall {
                id:   tc["id"].as_str().unwrap_or("").to_string(),
                name: tc["function"]["name"].as_str().unwrap_or("").to_string(),
                arguments: serde_json::from_str(
                    tc["function"]["arguments"].as_str().unwrap_or("{}")
                ).unwrap_or_default(),
            })
            .collect();
        CompletionResponse { content, tool_calls, finish_reason }
    }

    fn build_tools(req: &CompletionRequest) -> Value {
        let tools: Vec<Value> = req.tools.iter().map(|s| json!({
            "type": "function",
            "function": {
                "name": s["name"],
                "description": s["description"],
                "parameters": s["parameters"]
            }
        })).collect();
        json!(tools)
    }
}

#[async_trait]
impl LlmProvider for OpenAiProvider {
    async fn complete(&self, req: &CompletionRequest) -> Result<CompletionResponse> {
        let mut body = json!({
            "model": bare_model(&req.model),
            "messages": Self::to_openai_messages(req),
            "max_tokens": req.max_tokens
        });
        if !req.tools.is_empty() {
            body["tools"] = Self::build_tools(req);
        }

        let resp = self.client
            .post(&self.base_url)
            .bearer_auth(&self.api_key)
            .json(&body)
            .send()
            .await?;

        if !resp.status().is_success() {
            let status = resp.status();
            bail!("OpenAI API {status}: {}", resp.text().await.unwrap_or_default());
        }
        Ok(Self::parse_response(&resp.json::<Value>().await?))
    }

    async fn stream(
        &self,
        req: &CompletionRequest,
    ) -> Result<Pin<Box<dyn Stream<Item = Result<StreamChunk>> + Send>>> {
        let mut body = json!({
            "model": bare_model(&req.model),
            "messages": Self::to_openai_messages(req),
            "max_tokens": req.max_tokens,
            "stream": true
        });
        if !req.tools.is_empty() {
            body["tools"] = Self::build_tools(req);
        }

        let resp = self.client
            .post(&self.base_url)
            .bearer_auth(&self.api_key)
            .json(&body)
            .send()
            .await?;

        if !resp.status().is_success() {
            bail!("OpenAI stream {}: {}", resp.status(), resp.text().await.unwrap_or_default());
        }

        let mut byte_stream = resp.bytes_stream();
        let s = stream! {
            let mut buf = String::new();
            let mut tool_id = String::new();
            let mut tool_name = String::new();
            let mut tool_args = String::new();

            while let Some(chunk) = byte_stream.next().await {
                let chunk = match chunk { Ok(c) => c, Err(e) => { yield Err(anyhow::anyhow!("{e}")); break; } };
                buf.push_str(&String::from_utf8_lossy(&chunk));

                while let Some(pos) = buf.find('\n') {
                    let line = buf[..pos].trim().to_string();
                    buf = buf[pos + 1..].to_string();
                    let data = match line.strip_prefix("data: ") { Some(d) => d, None => continue };
                    if data == "[DONE]" { yield Ok(StreamChunk::Done); return; }
                    let v: Value = match serde_json::from_str(data) { Ok(v) => v, Err(_) => continue };
                    let delta = &v["choices"][0]["delta"];

                    if let Some(text) = delta["content"].as_str() {
                        if !text.is_empty() { yield Ok(StreamChunk::Text(text.to_string())); }
                    }
                    if let Some(tcs) = delta["tool_calls"].as_array() {
                        for tc in tcs {
                            if let Some(id) = tc["id"].as_str() { tool_id = id.to_string(); }
                            if let Some(n) = tc["function"]["name"].as_str() { tool_name = n.to_string(); }
                            if let Some(a) = tc["function"]["arguments"].as_str() { tool_args.push_str(a); }
                        }
                    }
                    if let Some("stop" | "tool_calls") = v["choices"][0]["finish_reason"].as_str() {
                        if !tool_name.is_empty() {
                            let args = serde_json::from_str(&tool_args).unwrap_or_default();
                            yield Ok(StreamChunk::ToolCall(LlmToolCall { id: tool_id.clone(), name: tool_name.clone(), arguments: args }));
                            tool_name.clear(); tool_id.clear(); tool_args.clear();
                        }
                        yield Ok(StreamChunk::Done);
                    }
                }
            }
        };
        Ok(Box::pin(s))
    }
}
