use crate::Result;
use futures::StreamExt;
use reqwest::Client;
use reqwest_eventsource::{Event, EventSource};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

/// CADE REST API client
#[derive(Clone)]
pub struct HttpTransport {
    client: Client,
    base_url: String,
    api_key: String,
}

// -- Agent

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentState {
    pub id: String,
    pub name: String,
    pub model: Option<String>,
    pub description: Option<String>,
    pub system_prompt: Option<String>,
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
    /// "short" | "long" | "pinned" — present in responses from the server.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tier: Option<String>,
}

// -- Messages

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
        let id = tc
            .get("id")
            .and_then(|v| v.as_str())
            .or(self.id.as_deref())
            .unwrap_or("unknown")
            .to_string();
        let name = tc.get("name").and_then(|v| v.as_str())?.to_string();
        let args = tc.get("arguments").cloned().unwrap_or(json!({}));
        // arguments may be a JSON string (needs parsing) or already an object
        let args = if let Some(s) = args.as_str() {
            serde_json::from_str(s).unwrap_or(json!({}))
        } else {
            args
        };
        Some((id, name, args))
    }

    /// Extract run_id from stream_start or any message that carries it
    pub fn run_id(&self) -> Option<&str> {
        self.data.get("run_id").and_then(|v| v.as_str())
    }

    /// Extract seq_id from a streamed event
    pub fn seq_id(&self) -> Option<i64> {
        self.data.get("seq_id").and_then(|v| v.as_i64())
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

#[derive(Debug, Serialize)]
pub struct ToolReturn {
    pub tool_call_id: String,
    pub content: String,
    pub status: String, // "success" | "error"
}

// -- Tools (server-registered)

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

// -- Client impl

impl HttpTransport {
    pub fn new(base_url: String, api_key: String) -> Result<Self> {
        let client = Client::builder()
            .user_agent("cade/0.1.0")
            .timeout(std::time::Duration::from_secs(30))
            .connect_timeout(std::time::Duration::from_secs(10))
            .build()
            .map_err(|e| crate::Error::custom(format!("build HTTP client: {e}")))?;
        Ok(Self {
            client,
            base_url,
            api_key,
        })
    }

    fn url(&self, path: &str) -> String {
        format!("{}/v1{}", self.base_url.trim_end_matches('/'), path)
    }

    // -- Health + server config

    pub async fn health(&self) -> Result<bool> {
        let resp = self
            .client
            .get(self.url("/health"))
            .header("Authorization", format!("Bearer {}", self.api_key))
            .send()
            .await?;
        Ok(resp.status().is_success())
    }

    /// Returns the server's version string from the `X-Cade-Version` response
    /// header (or falls back to the JSON body `version` field).
    /// Returns `None` if the server doesn't report a version.
    pub async fn server_version(&self) -> Option<String> {
        let resp = self
            .client
            .get(self.url("/health"))
            .header("Authorization", format!("Bearer {}", self.api_key))
            .send()
            .await
            .ok()?;
        // Prefer the response header (set by the version middleware).
        if let Some(v) = resp.headers().get("x-cade-version") {
            return v.to_str().ok().map(String::from);
        }
        // Fallback: parse the JSON body version field.
        let body: serde_json::Value = resp.json().await.ok()?;
        body["version"].as_str().map(String::from)
    }

    /// Fetch the server's auto-detected provider and default model.
    /// Falls back to a local default if the endpoint is unavailable (e.g. CADE Cloud).

    pub async fn list_providers(&self) -> crate::Result<serde_json::Value> {
        let resp = self
            .client
            .get(self.url("/providers"))
            .header("Authorization", format!("Bearer {}", self.api_key))
            .send()
            .await?;
        Ok(resp.json().await?)
    }

    pub async fn add_provider(
        &self,
        name: &str,
        kind: &str,
        api_key: Option<&str>,
        base_url: Option<&str>,
    ) -> crate::Result<serde_json::Value> {
        let mut body = json!({ "name": name, "kind": kind });
        if let Some(k) = api_key {
            body["api_key"] = k.into();
        }
        if let Some(u) = base_url {
            body["base_url"] = u.into();
        }
        let resp = self
            .client
            .post(self.url("/providers"))
            .header("Authorization", format!("Bearer {}", self.api_key))
            .json(&body)
            .send()
            .await?;
        if !resp.status().is_success() {
            let txt = resp.text().await.unwrap_or_default();
            return Err(crate::Error::custom(format!("add_provider failed: {txt}")));
        }
        Ok(resp.json().await?)
    }

    pub async fn remove_provider(&self, name: &str) -> crate::Result<()> {
        let resp = self
            .client
            .delete(self.url(&format!("/providers/{name}")))
            .header("Authorization", format!("Bearer {}", self.api_key))
            .send()
            .await?;
        if !resp.status().is_success() && resp.status().as_u16() != 404 {
            let txt = resp.text().await.unwrap_or_default();
            return Err(crate::Error::custom(format!(
                "remove_provider failed: {txt}"
            )));
        }
        Ok(())
    }

    pub async fn list_provider_presets(&self) -> Vec<serde_json::Value> {
        let resp = self
            .client
            .get(self.url("/providers/presets"))
            .header("Authorization", format!("Bearer {}", self.api_key))
            .send()
            .await;
        let Ok(r) = resp else { return vec![] };
        let Ok(body): core::result::Result<serde_json::Value, _> = r.json().await else {
            return vec![];
        };
        body["presets"].as_array().cloned().unwrap_or_default()
    }

    /// Returns live provider names from `GET /v1/providers` (liveness-aware).
    pub async fn available_providers(&self) -> Vec<String> {
        let resp = self
            .client
            .get(self.url("/providers"))
            .header("Authorization", format!("Bearer {}", self.api_key))
            .send()
            .await;
        let Ok(r) = resp else {
            return vec!["ollama".to_string()];
        };
        let Ok(body): core::result::Result<serde_json::Value, _> = r.json().await else {
            return vec!["ollama".to_string()];
        };
        body["providers"]
            .as_array()
            .map(|arr| {
                arr.iter()
                    .filter(|v| v["live"].as_bool().unwrap_or(false))
                    .filter_map(|v| v["name"].as_str().map(String::from))
                    .collect()
            })
            .unwrap_or_else(|| vec!["ollama".to_string()])
    }

    /// Response from `GET /v1/models`.
    pub async fn list_models(&self) -> crate::Result<serde_json::Value> {
        let resp = self
            .client
            .get(self.url("/models"))
            .header("Authorization", format!("Bearer {}", self.api_key))
            .send()
            .await?;
        if !resp.status().is_success() {
            return Err(crate::Error::custom(format!(
                "list_models failed {}",
                resp.status()
            )));
        }
        Ok(resp.json().await?)
    }

    pub async fn server_default_model(&self) -> String {
        let fallback = "anthropic/claude-sonnet-4-5-20250929".to_string();
        let resp = match self
            .client
            .get(self.url("/config"))
            .header("Authorization", format!("Bearer {}", self.api_key))
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
        let model = body["default_model"]
            .as_str()
            .unwrap_or("claude-sonnet-4-5-20250929");
        format!("{provider}/{model}")
    }

    // -- Agents

    /// Attach a list of tool IDs to an agent on the server.
    pub async fn attach_agent_tools(&self, agent_id: &str, tool_ids: &[String]) -> Result<()> {
        let resp = self
            .client
            .post(self.url(&format!("/agents/{agent_id}/tools")))
            .header("Authorization", format!("Bearer {}", self.api_key))
            .json(&json!({ "tool_ids": tool_ids }))
            .send()
            .await?;
        if !resp.status().is_success() {
            let status = resp.status();
            tracing::warn!("attach_agent_tools {status} — continuing without explicit attachment");
        }
        Ok(())
    }

    /// Detach ALL tools from an agent.
    pub async fn detach_agent_tools(&self, agent_id: &str) -> Result<usize> {
        let resp = self
            .client
            .delete(self.url(&format!("/agents/{agent_id}/tools")))
            .header("Authorization", format!("Bearer {}", self.api_key))
            .send()
            .await?;
        if !resp.status().is_success() {
            let status = resp.status();
            tracing::warn!("detach_agent_tools {status}");
            return Ok(0);
        }
        let body: serde_json::Value = resp.json().await?;
        Ok(body["detached"].as_u64().unwrap_or(0) as usize)
    }

    /// List tools currently attached to an agent. Returns `[(id, name)]`.
    pub async fn get_agent_tools(&self, agent_id: &str) -> Result<Vec<(String, String)>> {
        let resp = self
            .client
            .get(self.url(&format!("/agents/{agent_id}/tools")))
            .header("Authorization", format!("Bearer {}", self.api_key))
            .send()
            .await?;
        if !resp.status().is_success() {
            return Err(crate::Error::custom(format!(
                "get_agent_tools failed {}",
                resp.status()
            )));
        }
        let list: Vec<serde_json::Value> = resp.json().await?;
        Ok(list
            .into_iter()
            .filter_map(|v| {
                let id = v["id"].as_str()?.to_string();
                let name = v["name"].as_str()?.to_string();
                Some((id, name))
            })
            .collect())
    }

    /// Switch the model for an existing agent. Returns the new model string.
    pub async fn patch_agent_model(&self, agent_id: &str, model: &str) -> Result<String> {
        let resp = self
            .client
            .patch(self.url(&format!("/agents/{agent_id}")))
            .header("Authorization", format!("Bearer {}", self.api_key))
            .json(&json!({ "model": model }))
            .send()
            .await?;
        if !resp.status().is_success() {
            let text = resp.text().await.unwrap_or_default();
            // Extract human-readable detail from {"detail":"..."} wrapper if present
            let msg = serde_json::from_str::<serde_json::Value>(&text)
                .ok()
                .and_then(|v| v["detail"].as_str().map(String::from))
                .unwrap_or(text);
            return Err(crate::Error::custom(msg.to_string()));
        }
        let body: serde_json::Value = resp.json().await?;
        Ok(body["model"].as_str().unwrap_or(model).to_string())
    }

    /// Update the system prompt for an existing agent.
    pub async fn patch_agent_system_prompt(&self, agent_id: &str, prompt: &str) -> Result<()> {
        let resp = self
            .client
            .patch(self.url(&format!("/agents/{agent_id}")))
            .header("Authorization", format!("Bearer {}", self.api_key))
            .json(&json!({ "system_prompt": prompt }))
            .send()
            .await?;
        if !resp.status().is_success() {
            let text = resp.text().await.unwrap_or_default();
            return Err(crate::Error::custom(text.to_string()));
        }
        Ok(())
    }

    /// Update the compaction model for an existing agent.
    pub async fn patch_agent_compaction_model(&self, agent_id: &str, model: Option<&str>) -> Result<()> {
        let resp = self
            .client
            .patch(self.url(&format!("/agents/{agent_id}")))
            .header("Authorization", format!("Bearer {}", self.api_key))
            .json(&json!({ "compaction_model": model.unwrap_or("") }))
            .send()
            .await?;
        if !resp.status().is_success() {
            let text = resp.text().await.unwrap_or_default();
            let msg = serde_json::from_str::<serde_json::Value>(&text)
                .ok()
                .and_then(|v| v["detail"].as_str().map(String::from))
                .unwrap_or(text);
            return Err(crate::Error::custom(msg.to_string()));
        }
        Ok(())
    }

    pub async fn create_agent(&self, req: CreateAgentRequest) -> Result<AgentState> {
        let resp = self
            .client
            .post(self.url("/agents"))
            .header("Authorization", format!("Bearer {}", self.api_key))
            .json(&req)
            .send()
            .await?;
        if !resp.status().is_success() {
            let status = resp.status();
            let body = resp.text().await.unwrap_or_default();
            return Err(crate::Error::custom(format!(
                "create_agent failed {status}: {body}"
            )));
        }
        Ok(resp.json::<AgentState>().await?)
    }

    pub async fn delete_agent(&self, agent_id: &str) -> Result<()> {
        let resp = self
            .client
            .delete(self.url(&format!("/agents/{agent_id}")))
            .header("Authorization", format!("Bearer {}", self.api_key))
            .send()
            .await?;
        if !resp.status().is_success() && resp.status().as_u16() != 404 {
            return Err(crate::Error::custom(format!(
                "delete_agent failed {}",
                resp.status()
            )));
        }
        Ok(())
    }

    pub async fn get_agent(&self, agent_id: &str) -> Result<AgentState> {
        let resp = self
            .client
            .get(self.url(&format!("/agents/{agent_id}")))
            .header("Authorization", format!("Bearer {}", self.api_key))
            .send()
            .await?;
        if !resp.status().is_success() {
            let status = resp.status();
            return Err(crate::Error::custom(format!(
                "get_agent {agent_id} failed {status}"
            )));
        }
        Ok(resp.json::<AgentState>().await?)
    }

    /// Rename an agent (PATCH /v1/agents/:id with {"name": name}).
    pub async fn rename_agent(&self, agent_id: &str, name: &str) -> Result<()> {
        let resp = self
            .client
            .patch(self.url(&format!("/agents/{agent_id}")))
            .header("Authorization", format!("Bearer {}", self.api_key))
            .json(&json!({ "name": name }))
            .send()
            .await?;
        if !resp.status().is_success() {
            return Err(crate::Error::custom(format!(
                "rename_agent failed {}",
                resp.status()
            )));
        }
        Ok(())
    }

    pub async fn list_agents(&self) -> Result<Vec<AgentState>> {
        let resp = self
            .client
            .get(self.url("/agents"))
            .header("Authorization", format!("Bearer {}", self.api_key))
            .send()
            .await?;
        if !resp.status().is_success() {
            return Err(crate::Error::custom(format!(
                "list_agents failed {}",
                resp.status()
            )));
        }
        Ok(resp.json::<Vec<AgentState>>().await?)
    }

    /// Load a skill server-side for an agent.
    pub async fn load_skill_on_server(&self, agent_id: &str, skill_id: &str) -> Result<()> {
        let resp = self
            .client
            .post(self.url(&format!("/agents/{agent_id}/skills/load")))
            .header("Authorization", format!("Bearer {}", self.api_key))
            .json(&serde_json::json!({ "id": skill_id }))
            .send()
            .await?;
        if !resp.status().is_success() {
            return Err(crate::Error::custom(format!(
                "load_skill_on_server failed: {}",
                resp.status()
            )));
        }
        Ok(())
    }

    /// Unload a skill server-side for an agent.
    pub async fn unload_skill_on_server(&self, agent_id: &str, skill_id: &str) -> Result<()> {
        let resp = self
            .client
            .post(self.url(&format!("/agents/{agent_id}/skills/unload")))
            .header("Authorization", format!("Bearer {}", self.api_key))
            .json(&serde_json::json!({ "id": skill_id }))
            .send()
            .await?;
        if !resp.status().is_success() {
            return Err(crate::Error::custom(format!(
                "unload_skill_on_server failed: {}",
                resp.status()
            )));
        }
        Ok(())
    }

    /// Disable a skill server-side for an agent (add to blacklist).
    /// The skill remains installed but is excluded from context injection.
    pub async fn disable_skill_on_server(&self, agent_id: &str, skill_id: &str) -> Result<()> {
        let resp = self
            .client
            .post(self.url(&format!("/agents/{agent_id}/skills/disable")))
            .header("Authorization", format!("Bearer {}", self.api_key))
            .json(&serde_json::json!({ "id": skill_id }))
            .send()
            .await?;
        if !resp.status().is_success() {
            return Err(crate::Error::custom(format!(
                "disable_skill_on_server failed: {}",
                resp.status()
            )));
        }
        Ok(())
    }

    /// Enable a skill server-side for an agent (remove from blacklist).
    pub async fn enable_skill_on_server(&self, agent_id: &str, skill_id: &str) -> Result<()> {
        let resp = self
            .client
            .post(self.url(&format!("/agents/{agent_id}/skills/enable")))
            .header("Authorization", format!("Bearer {}", self.api_key))
            .json(&serde_json::json!({ "id": skill_id }))
            .send()
            .await?;
        if !resp.status().is_success() {
            return Err(crate::Error::custom(format!(
                "enable_skill_on_server failed: {}",
                resp.status()
            )));
        }
        Ok(())
    }
}

pub mod extensions;
pub mod memory;
pub mod messages;
