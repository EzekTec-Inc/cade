use anyhow::{Context, Result, bail};
use reqwest::Client;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

/// Letta REST API client
pub struct LettaClient {
    client: Client,
    base_url: String,
    api_key: String,
}

// ── Agent ────────────────────────────────────────────────────────────────────

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

// ── Messages ─────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LettaMessage {
    pub id: Option<String>,
    pub message_type: Option<String>,
    #[serde(flatten)]
    pub data: Value,
}

#[derive(Debug, Serialize)]
pub struct SendMessageRequest {
    pub input: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub conversation_id: Option<String>,
}

// ── Tools ─────────────────────────────────────────────────────────────────────

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
            .build()
            .context("build HTTP client")?;
        Ok(Self { client, base_url, api_key })
    }

    fn url(&self, path: &str) -> String {
        format!("{}/v1{}", self.base_url.trim_end_matches('/'), path)
    }

    fn auth_header(&self) -> (&'static str, String) {
        ("Authorization", format!("Bearer {}", self.api_key))
    }

    // ── Health ────────────────────────────────────────────────────────────────

    pub async fn health(&self) -> Result<bool> {
        let resp = self.client
            .get(self.url("/health"))
            .header(self.auth_header().0, self.auth_header().1)
            .send()
            .await?;
        Ok(resp.status().is_success())
    }

    // ── Agents ────────────────────────────────────────────────────────────────

    pub async fn create_agent(&self, req: CreateAgentRequest) -> Result<AgentState> {
        let resp = self.client
            .post(self.url("/agents"))
            .header(self.auth_header().0, self.auth_header().1)
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
            .header(self.auth_header().0, self.auth_header().1)
            .send()
            .await?;

        if !resp.status().is_success() {
            let status = resp.status();
            bail!("get_agent {agent_id} failed {status}");
        }
        Ok(resp.json::<AgentState>().await?)
    }

    pub async fn list_agents(&self) -> Result<Vec<AgentState>> {
        let resp = self.client
            .get(self.url("/agents"))
            .header(self.auth_header().0, self.auth_header().1)
            .send()
            .await?;

        if !resp.status().is_success() {
            bail!("list_agents failed {}", resp.status());
        }
        Ok(resp.json::<Vec<AgentState>>().await?)
    }

    // ── Messages ──────────────────────────────────────────────────────────────

    pub async fn send_message(&self, agent_id: &str, input: &str) -> Result<Vec<LettaMessage>> {
        let req = SendMessageRequest {
            input: input.to_string(),
            conversation_id: None,
        };

        let resp = self.client
            .post(self.url(&format!("/agents/{agent_id}/messages")))
            .header(self.auth_header().0, self.auth_header().1)
            .json(&req)
            .send()
            .await?;

        if !resp.status().is_success() {
            let status = resp.status();
            let body = resp.text().await.unwrap_or_default();
            bail!("send_message failed {status}: {body}");
        }

        let body: Value = resp.json().await?;
        let messages = body["messages"]
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

        Ok(messages)
    }

    // ── Tools ─────────────────────────────────────────────────────────────────

    pub async fn create_tool(&self, req: CreateToolRequest) -> Result<ToolDef> {
        let resp = self.client
            .post(self.url("/tools"))
            .header(self.auth_header().0, self.auth_header().1)
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
            .header(self.auth_header().0, self.auth_header().1)
            .send()
            .await?;

        if !resp.status().is_success() {
            bail!("list_tools failed {}", resp.status());
        }
        Ok(resp.json::<Vec<ToolDef>>().await?)
    }
}
