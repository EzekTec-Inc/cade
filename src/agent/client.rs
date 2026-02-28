use anyhow::{Context, Result, bail};
use futures::StreamExt;
use reqwest::Client;
use reqwest_eventsource::{Event, EventSource};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

/// Letta REST API client
pub struct LettaClient {
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
pub struct LettaMessage {
    pub id: Option<String>,
    pub message_type: Option<String>,
    #[serde(flatten)]
    pub data: Value,
}

impl LettaMessage {
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

#[derive(Debug, Serialize)]
pub struct CreateToolRequest {
    pub name: String,
    pub description: String,
    pub source_code: String,
    pub source_type: String,
    pub json_schema: Value,
}

// ── Client impl ───────────────────────────────────────────────────────────────

impl LettaClient {
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

    // ── Health ────────────────────────────────────────────────────────────────

    pub async fn health(&self) -> Result<bool> {
        let resp = self.client
            .get(self.url("/health"))
            .header(self.auth().0, self.auth().1)
            .send()
            .await?;
        Ok(resp.status().is_success())
    }

    // ── Agents ────────────────────────────────────────────────────────────────

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

    // ── Messages ──────────────────────────────────────────────────────────────

    /// Send a user message and return the response messages
    pub async fn send_message(&self, agent_id: &str, input: &str) -> Result<Vec<LettaMessage>> {
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
    ) -> Result<Vec<LettaMessage>>
    where
        F: Fn(&LettaMessage),
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

        let mut messages: Vec<LettaMessage> = Vec::new();

        while let Some(event) = es.next().await {
            match event {
                Ok(Event::Open) => {}
                Ok(Event::Message(msg)) => {
                    if msg.data.trim() == "[DONE]" || msg.data.trim().is_empty() {
                        continue;
                    }
                    match serde_json::from_str::<LettaMessage>(&msg.data) {
                        Ok(lm) => {
                            on_event(&lm);
                            messages.push(lm);
                        }
                        Err(_) => {
                            // Try parsing as a wrapper object with a messages array
                            if let Ok(v) = serde_json::from_str::<Value>(&msg.data) {
                                if let Some(arr) = v["messages"].as_array() {
                                    for item in arr {
                                        if let Ok(lm) = serde_json::from_value::<LettaMessage>(item.clone()) {
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
    ) -> Result<Vec<LettaMessage>> {
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
    ) -> Result<Vec<LettaMessage>>
    where
        F: Fn(&LettaMessage),
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
            match event {
                Ok(Event::Open) => {}
                Ok(Event::Message(msg)) => {
                    if msg.data.trim() == "[DONE]" || msg.data.trim().is_empty() {
                        continue;
                    }
                    if let Ok(lm) = serde_json::from_str::<LettaMessage>(&msg.data) {
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

    async fn post_messages(&self, agent_id: &str, body: &Value) -> Result<Vec<LettaMessage>> {
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
            .map(|v| serde_json::from_value(v).unwrap_or(LettaMessage {
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
