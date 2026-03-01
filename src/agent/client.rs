use anyhow::{Context, Result, bail};
use futures::StreamExt;
use reqwest::Client;
use reqwest_eventsource::{Event, EventSource};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

/// CADE REST API client
#[derive(Clone)]
pub struct CadeClient {
    client: Client,
    base_url: String,
    api_key: String,
}

// ── Agent ─────────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentState {
    pub id: String,
    pub name: String,
    pub model: Option<String>,
    pub description: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct CreateAgentRequest {
    pub name: Option<String>,
    pub model: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub system_prompt: Option<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub memory_blocks: Vec<MemoryBlock>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub tool_ids: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MemoryBlock {
    pub label: String,
    pub value: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
}

// ── Messages ──────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CadeMessage {
    pub id: Option<String>,
    pub message_type: Option<String>,
    #[serde(flatten)]
    pub data: Value,
}

impl CadeMessage {
    /// Return the message_type string, or empty if absent
    pub fn msg_type(&self) -> &str {
        self.data
            .get("message_type")
            .and_then(|v| v.as_str())
            .or(self.message_type.as_deref())
            .unwrap_or("")
    }

    /// Extract tool call info from a tool_call_message.
    /// Returns `(tool_call_id, tool_name, arguments_value)`.
    pub fn as_tool_call(&self) -> Option<(String, String, Value)> {
        if self.msg_type() != "tool_call_message" {
            return None;
        }
        let tc = self.data.get("tool_call")?;
        let id = tc.get("id")
            .and_then(|v| v.as_str())
            .or_else(|| self.id.as_deref())
            .unwrap_or("unknown")
            .to_string();
        let name = tc.get("name").and_then(|v| v.as_str())?.to_string();
        let args = tc
            .get("arguments")
            .cloned()
            .unwrap_or(json!({}));
        // arguments may be a JSON string (needs parsing) or already an object
        let args = if let Some(s) = args.as_str() {
            serde_json::from_str(s).unwrap_or(json!({}))
        } else {
            args
        };
        Some((id, name, args))
    }

    /// Extract the text of an assistant_message
    pub fn assistant_text(&self) -> Option<&str> {
        if self.msg_type() != "assistant_message" {
            return None;
        }
        self.data.get("content").and_then(|v| v.as_str())
    }

    /// Extract reasoning text from a reasoning_message
    pub fn reasoning_text(&self) -> Option<&str> {
        if self.msg_type() != "reasoning_message" {
            return None;
        }
        self.data.get("reasoning").and_then(|v| v.as_str())
    }
}

#[allow(dead_code)]
#[derive(Debug, Serialize)]
pub struct SendMessageRequest {
    pub input: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub conversation_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub role: Option<String>,
}

/// A tool return sent back to the agent after local execution
#[allow(dead_code)]
#[derive(Debug, Serialize)]
pub struct ToolReturnRequest {
    /// Must be "tool"
    pub role: String,
    pub tool_return: ToolReturn,
}

#[derive(Debug, Serialize)]
pub struct ToolReturn {
    pub tool_call_id: String,
    pub content: String,
    pub status: String, // "success" | "error"
}

// ── Tools (server-registered) ─────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolDef {
    pub id: String,
    pub name: String,
    pub description: Option<String>,
}

/// Only fields accepted by POST /v1/tools.
/// `name` and `description` are NOT top-level — the API derives the name
/// from the Python function name in source_code.
#[derive(Debug, Serialize)]
pub struct CreateToolRequest {
    pub source_code: String,
    pub source_type: String,
    /// Full OpenAI-compatible function schema (name + description + parameters).
    /// Providing it overrides the auto-generated schema from source_code.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub json_schema: Option<Value>,
    /// Optional tags for organisation
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub tags: Vec<String>,
}

// ── Client impl ───────────────────────────────────────────────────────────────

impl CadeClient {
    pub fn new(base_url: String, api_key: String) -> Result<Self> {
        let client = Client::builder()
            .user_agent("cade/0.1.0")
            .timeout(std::time::Duration::from_secs(300))
            .build()
            .context("build HTTP client")?;
        Ok(Self { client, base_url, api_key })
    }

    fn url(&self, path: &str) -> String {
        format!("{}/v1{}", self.base_url.trim_end_matches('/'), path)
    }

    fn auth(&self) -> (&'static str, String) {
        ("Authorization", format!("Bearer {}", self.api_key))
    }

    // ── Health + server config ────────────────────────────────────────────────

    pub async fn health(&self) -> Result<bool> {
        let resp = self.client
            .get(self.url("/health"))
            .header(self.auth().0, self.auth().1)
            .send()
            .await?;
        Ok(resp.status().is_success())
    }

    /// Fetch the server's auto-detected provider and default model.
    /// Falls back to a local default if the endpoint is unavailable (e.g. Letta Cloud).
    // ── Provider management ───────────────────────────────────────────────────

    pub async fn list_providers(&self) -> anyhow::Result<serde_json::Value> {
        let resp = self.client
            .get(self.url("/providers"))
            .header(self.auth().0, self.auth().1)
            .send().await?;
        Ok(resp.json().await?)
    }

    pub async fn add_provider(
        &self,
        name: &str,
        kind: &str,
        api_key: Option<&str>,
        base_url: Option<&str>,
    ) -> anyhow::Result<serde_json::Value> {
        let mut body = serde_json::json!({ "name": name, "kind": kind });
        if let Some(k) = api_key  { body["api_key"]  = k.into(); }
        if let Some(u) = base_url { body["base_url"] = u.into(); }
        let resp = self.client
            .post(self.url("/providers"))
            .header(self.auth().0, self.auth().1)
            .json(&body)
            .send().await?;
        if !resp.status().is_success() {
            let txt = resp.text().await.unwrap_or_default();
            anyhow::bail!("add_provider failed: {txt}");
        }
        Ok(resp.json().await?)
    }

    pub async fn remove_provider(&self, name: &str) -> anyhow::Result<()> {
        let resp = self.client
            .delete(self.url(&format!("/providers/{name}")))
            .header(self.auth().0, self.auth().1)
            .send().await?;
        if !resp.status().is_success() && resp.status().as_u16() != 404 {
            let txt = resp.text().await.unwrap_or_default();
            anyhow::bail!("remove_provider failed: {txt}");
        }
        Ok(())
    }

    pub async fn list_provider_presets(&self) -> Vec<serde_json::Value> {
        let resp = self.client
            .get(self.url("/providers/presets"))
            .header(self.auth().0, self.auth().1)
            .send().await;
        let Ok(r) = resp else { return vec![] };
        let Ok(body): Result<serde_json::Value, _> = r.json().await else { return vec![] };
        body["presets"].as_array().cloned().unwrap_or_default()
    }

    /// Returns live provider names from `GET /v1/providers` (liveness-aware).
    pub async fn available_providers(&self) -> Vec<String> {
        let resp = self.client
            .get(self.url("/providers"))
            .header(self.auth().0, self.auth().1)
            .send().await;
        let Ok(r) = resp else { return vec!["ollama".to_string()] };
        let Ok(body): Result<serde_json::Value, _> = r.json().await else {
            return vec!["ollama".to_string()]
        };
        body["providers"].as_array()
            .map(|arr| {
                arr.iter()
                    .filter(|v| v["live"].as_bool().unwrap_or(false))
                    .filter_map(|v| v["name"].as_str().map(String::from))
                    .collect()
            })
            .unwrap_or_else(|| vec!["ollama".to_string()])
    }

    /// Response from `GET /v1/models`.
    pub async fn list_models(&self) -> anyhow::Result<serde_json::Value> {
        let resp = self.client
            .get(self.url("/models"))
            .header(self.auth().0, self.auth().1)
            .send().await?;
        if !resp.status().is_success() {
            anyhow::bail!("list_models failed {}", resp.status());
        }
        Ok(resp.json().await?)
    }

    pub async fn server_default_model(&self) -> String {
        let fallback = "anthropic/claude-sonnet-4-5-20250929".to_string();
        let resp = match self.client
            .get(self.url("/config"))
            .header(self.auth().0, self.auth().1)
            .send()
            .await
        {
            Ok(r) if r.status().is_success() => r,
            _ => return fallback,
        };
        let body: serde_json::Value = match resp.json().await {
            Ok(v) => v,
            Err(_) => return fallback,
        };
        // Server returns bare model name; wrap with provider prefix for storage
        let provider = body["provider"].as_str().unwrap_or("anthropic");
        let model    = body["default_model"].as_str().unwrap_or("claude-sonnet-4-5-20250929");
        format!("{provider}/{model}")
    }

    // ── Agents ────────────────────────────────────────────────────────────────

    /// Attach a list of tool IDs to an agent on the server.
    /// Calls PATCH /v1/agents/:id/tools for each tool.
    pub async fn attach_agent_tools(&self, agent_id: &str, tool_ids: &[String]) -> Result<()> {
        let resp = self.client
            .post(self.url(&format!("/agents/{agent_id}/tools")))
            .header(self.auth().0, self.auth().1)
            .json(&serde_json::json!({ "tool_ids": tool_ids }))
            .send()
            .await?;
        if !resp.status().is_success() {
            let status = resp.status();
            tracing::warn!("attach_agent_tools {status} — continuing without explicit attachment");
        }
        Ok(())
    }

    /// Switch the model for an existing agent. Returns the new model string.
    pub async fn patch_agent_model(&self, agent_id: &str, model: &str) -> Result<String> {
        let resp = self.client
            .patch(self.url(&format!("/agents/{agent_id}")))
            .header(self.auth().0, self.auth().1)
            .json(&serde_json::json!({ "model": model }))
            .send()
            .await?;
        if !resp.status().is_success() {
            let status = resp.status();
            let body = resp.text().await.unwrap_or_default();
            bail!("patch_agent failed {status}: {body}");
        }
        let body: serde_json::Value = resp.json().await?;
        Ok(body["model"].as_str().unwrap_or(model).to_string())
    }

    pub async fn create_agent(&self, req: CreateAgentRequest) -> Result<AgentState> {
        let resp = self.client
            .post(self.url("/agents"))
            .header(self.auth().0, self.auth().1)
            .json(&req)
            .send()
            .await?;
        if !resp.status().is_success() {
            let status = resp.status();
            let body = resp.text().await.unwrap_or_default();
            bail!("create_agent failed {status}: {body}");
        }
        Ok(resp.json::<AgentState>().await?)
    }

    pub async fn delete_agent(&self, agent_id: &str) -> Result<()> {
        let resp = self.client
            .delete(self.url(&format!("/agents/{agent_id}")))
            .header(self.auth().0, self.auth().1)
            .send().await?;
        if !resp.status().is_success() && resp.status().as_u16() != 404 {
            bail!("delete_agent failed {}", resp.status());
        }
        Ok(())
    }

    pub async fn get_agent(&self, agent_id: &str) -> Result<AgentState> {
        let resp = self.client
            .get(self.url(&format!("/agents/{agent_id}")))
            .header(self.auth().0, self.auth().1)
            .send()
            .await?;
        if !resp.status().is_success() {
            let status = resp.status();
            bail!("get_agent {agent_id} failed {status}");
        }
        Ok(resp.json::<AgentState>().await?)
    }

    /// Rename an agent (PATCH /v1/agents/:id with {"name": name}).
    pub async fn rename_agent(&self, agent_id: &str, name: &str) -> Result<()> {
        let resp = self.client
            .patch(self.url(&format!("/agents/{agent_id}")))
            .header(self.auth().0, self.auth().1)
            .json(&serde_json::json!({ "name": name }))
            .send()
            .await?;
        if !resp.status().is_success() {
            bail!("rename_agent failed {}", resp.status());
        }
        Ok(())
    }

    #[allow(dead_code)]
    pub async fn list_agents(&self) -> Result<Vec<AgentState>> {
        let resp = self.client
            .get(self.url("/agents"))
            .header(self.auth().0, self.auth().1)
            .send()
            .await?;
        if !resp.status().is_success() {
            bail!("list_agents failed {}", resp.status());
        }
        Ok(resp.json::<Vec<AgentState>>().await?)
    }

    // ── Memory ────────────────────────────────────────────────────────────────

    /// Fetch all memory blocks for an agent.
    pub async fn get_memory(&self, agent_id: &str) -> Result<Vec<MemoryBlock>> {
        let resp = self.client
            .get(self.url(&format!("/agents/{agent_id}/memory")))
            .header(self.auth().0, self.auth().1)
            .send().await?;
        if !resp.status().is_success() {
            bail!("get_memory failed {}", resp.status());
        }
        let body: Value = resp.json().await?;
        let blocks = body["blocks"].as_array().cloned().unwrap_or_default();
        Ok(blocks.into_iter().filter_map(|v| serde_json::from_value(v).ok()).collect())
    }

    /// Delete a single memory block.
    pub async fn delete_memory(&self, agent_id: &str, label: &str) -> Result<()> {
        let resp = self.client
            .delete(self.url(&format!("/agents/{agent_id}/memory/{label}")))
            .header(self.auth().0, self.auth().1)
            .send().await?;
        if !resp.status().is_success() && resp.status().as_u16() != 404 {
            bail!("delete_memory failed {}", resp.status());
        }
        Ok(())
    }

    /// Upsert a single memory block.
    pub async fn upsert_memory(
        &self,
        agent_id: &str,
        label: &str,
        value: &str,
        description: Option<&str>,
    ) -> Result<()> {
        let mut body = json!({ "value": value });
        if let Some(desc) = description {
            body["description"] = json!(desc);
        }
        let resp = self.client
            .put(self.url(&format!("/agents/{agent_id}/memory/{label}")))
            .header(self.auth().0, self.auth().1)
            .json(&body)
            .send().await?;
        if !resp.status().is_success() {
            bail!("upsert_memory failed {}", resp.status());
        }
        Ok(())
    }

    // ── Context management ────────────────────────────────────────────────────

    /// Delete all messages for an agent (clear context window).
    pub async fn clear_messages(&self, agent_id: &str) -> Result<usize> {
        let resp = self.client
            .delete(self.url(&format!("/agents/{agent_id}/messages")))
            .header(self.auth().0, self.auth().1)
            .send().await?;
        if !resp.status().is_success() {
            bail!("clear_messages failed {}", resp.status());
        }
        let body: Value = resp.json().await?;
        Ok(body["deleted"].as_u64().unwrap_or(0) as usize)
    }

    /// Search message history for an agent.
    pub async fn search_messages(&self, agent_id: &str, query: &str) -> Result<Vec<Value>> {
        let resp = self.client
            .get(self.url(&format!("/agents/{agent_id}/messages")))
            .header(self.auth().0, self.auth().1)
            .query(&[("q", query)])
            .send().await?;
        if !resp.status().is_success() {
            bail!("search_messages failed {}", resp.status());
        }
        let body: Value = resp.json().await?;
        Ok(body["messages"].as_array().cloned().unwrap_or_default())
    }

    // ── Messages ──────────────────────────────────────────────────────────────

    /// Send a user message and return the response messages
    pub async fn send_message(&self, agent_id: &str, input: &str) -> Result<Vec<CadeMessage>> {
        let req = json!({ "input": input });
        self.post_messages(agent_id, &req).await
    }

    /// Stream a user message using SSE. Calls `on_event` for each message as
    /// it arrives (for live rendering), and returns the full collected list.
    ///
    /// Uses `/v1/agents/{id}/messages/stream` — falls back to `send_message`
    /// if the stream endpoint returns a non-2xx status.
    pub async fn stream_message<F>(
        &self,
        agent_id: &str,
        input: &str,
        on_event: F,
    ) -> Result<Vec<CadeMessage>>
    where
        F: Fn(&CadeMessage),
    {
        self.stream_message_cancellable(agent_id, input, on_event, None).await
    }

    /// Like `stream_message` but checks an optional cancel flag before each SSE event.
    pub async fn stream_message_cancellable<F>(
        &self,
        agent_id: &str,
        input: &str,
        on_event: F,
        cancel: Option<&std::sync::Arc<std::sync::atomic::AtomicBool>>,
    ) -> Result<Vec<CadeMessage>>
    where
        F: Fn(&CadeMessage),
    {
        let url = self.url(&format!("/agents/{agent_id}/messages/stream"));
        let body = json!({ "input": input });

        let request = self
            .client
            .post(&url)
            .header(self.auth().0, self.auth().1)
            .json(&body);

        let mut es = EventSource::new(request)
            .map_err(|e| anyhow::anyhow!("EventSource: {e}"))?;

        let mut messages: Vec<CadeMessage> = Vec::new();

        while let Some(event) = es.next().await {
            // Check cancel flag on every event (fired ~per token while streaming)
            if cancel.map_or(false, |f| f.load(std::sync::atomic::Ordering::SeqCst)) {
                es.close();
                return Err(anyhow::anyhow!("__cancelled__"));
            }
            match event {
                Ok(Event::Open) => {}
                Ok(Event::Message(msg)) => {
                    let data = msg.data.trim();
                    if data.is_empty() {
                        continue;
                    }
                    if data == "[DONE]" {
                        // Explicitly close to prevent reqwest_eventsource from
                        // auto-reconnecting (SSE spec default), which would re-POST
                        // the same body and duplicate messages on the server.
                        es.close();
                        break;
                    }
                    match serde_json::from_str::<CadeMessage>(data) {
                        Ok(lm) => {
                            on_event(&lm);
                            messages.push(lm);
                        }
                        Err(_) => {
                            // Try parsing as a wrapper object with a messages array
                            if let Ok(v) = serde_json::from_str::<Value>(data) {
                                if let Some(arr) = v["messages"].as_array() {
                                    for item in arr {
                                        if let Ok(lm) = serde_json::from_value::<CadeMessage>(item.clone()) {
                                            on_event(&lm);
                                            messages.push(lm);
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
                Err(reqwest_eventsource::Error::StreamEnded) => break,
                Err(e) => {
                    // Non-streaming fallback: if stream endpoint fails, use regular
                    tracing::debug!("SSE error: {e}, falling back to send_message");
                    es.close();
                    let fallback = self.send_message(agent_id, input).await?;
                    for lm in &fallback {
                        on_event(lm);
                    }
                    return Ok(fallback);
                }
            }
        }

        Ok(messages)
    }

    /// Send a tool result back to the agent after local execution
    pub async fn send_tool_return(
        &self,
        agent_id: &str,
        tool_call_id: &str,
        output: &str,
        is_error: bool,
    ) -> Result<Vec<CadeMessage>> {
        let req = json!({
            "role": "tool",
            "tool_return": {
                "tool_call_id": tool_call_id,
                "content": output,
                "status": if is_error { "error" } else { "success" }
            }
        });
        self.post_messages(agent_id, &req).await
    }

    /// Stream a tool return response (same as send_tool_return but with live events)
    pub async fn stream_tool_return<F>(
        &self,
        agent_id: &str,
        tool_call_id: &str,
        output: &str,
        is_error: bool,
        on_event: F,
    ) -> Result<Vec<CadeMessage>>
    where
        F: Fn(&CadeMessage),
    {
        self.stream_tool_return_cancellable(agent_id, tool_call_id, output, is_error, on_event, None).await
    }

    /// Like `stream_tool_return` but checks an optional cancel flag between SSE events.
    pub async fn stream_tool_return_cancellable<F>(
        &self,
        agent_id: &str,
        tool_call_id: &str,
        output: &str,
        is_error: bool,
        on_event: F,
        cancel: Option<&std::sync::Arc<std::sync::atomic::AtomicBool>>,
    ) -> Result<Vec<CadeMessage>>
    where
        F: Fn(&CadeMessage),
    {
        let body = json!({
            "role": "tool",
            "tool_return": {
                "tool_call_id": tool_call_id,
                "content": output,
                "status": if is_error { "error" } else { "success" }
            }
        });
        let url = self.url(&format!("/agents/{agent_id}/messages/stream"));
        let request = self
            .client
            .post(&url)
            .header(self.auth().0, self.auth().1)
            .json(&body);

        let mut es = EventSource::new(request)
            .map_err(|e| anyhow::anyhow!("EventSource: {e}"))?;
        let mut messages = Vec::new();

        while let Some(event) = es.next().await {
            if cancel.map_or(false, |f| f.load(std::sync::atomic::Ordering::SeqCst)) {
                es.close();
                return Err(anyhow::anyhow!("__cancelled__"));
            }
            match event {
                Ok(Event::Open) => {}
                Ok(Event::Message(msg)) => {
                    let data = msg.data.trim();
                    if data.is_empty() {
                        continue;
                    }
                    if data == "[DONE]" {
                        // Close explicitly — prevents SSE auto-reconnect which would
                        // re-POST the tool_return body and duplicate the DB record.
                        es.close();
                        break;
                    }
                    if let Ok(lm) = serde_json::from_str::<CadeMessage>(data) {
                        on_event(&lm);
                        messages.push(lm);
                    }
                }
                Err(reqwest_eventsource::Error::StreamEnded) => break,
                Err(_) => {
                    // Fallback to non-streaming
                    es.close();
                    let fallback = self.send_tool_return(agent_id, tool_call_id, output, is_error).await?;
                    for lm in &fallback { on_event(lm); }
                    return Ok(fallback);
                }
            }
        }
        Ok(messages)
    }

    async fn post_messages(&self, agent_id: &str, body: &Value) -> Result<Vec<CadeMessage>> {
        let resp = self.client
            .post(self.url(&format!("/agents/{agent_id}/messages")))
            .header(self.auth().0, self.auth().1)
            .json(body)
            .send()
            .await?;

        if !resp.status().is_success() {
            let status = resp.status();
            let text = resp.text().await.unwrap_or_default();
            bail!("messages request failed {status}: {text}");
        }

        let raw: Value = resp.json().await?;
        let msgs = raw["messages"]
            .as_array()
            .cloned()
            .unwrap_or_default()
            .into_iter()
            .map(|v| serde_json::from_value(v).unwrap_or(CadeMessage {
                id: None,
                message_type: None,
                data: json!({}),
            }))
            .collect();
        Ok(msgs)
    }

    // ── Tools ─────────────────────────────────────────────────────────────────

    pub async fn create_tool(&self, req: CreateToolRequest) -> Result<ToolDef> {
        let resp = self.client
            .post(self.url("/tools"))
            .header(self.auth().0, self.auth().1)
            .json(&req)
            .send()
            .await?;
        if !resp.status().is_success() {
            let status = resp.status();
            let body = resp.text().await.unwrap_or_default();
            bail!("create_tool failed {status}: {body}");
        }
        Ok(resp.json::<ToolDef>().await?)
    }

    pub async fn list_tools(&self) -> Result<Vec<ToolDef>> {
        let resp = self.client
            .get(self.url("/tools"))
            .header(self.auth().0, self.auth().1)
            .send()
            .await?;
        if !resp.status().is_success() {
            bail!("list_tools failed {}", resp.status());
        }
        Ok(resp.json::<Vec<ToolDef>>().await?)
    }
}
