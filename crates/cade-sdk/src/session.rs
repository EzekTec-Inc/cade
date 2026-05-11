/// High-level agent session for SDK consumers.
///
/// `AgentSession` wraps `CadeClient` and `ToolRuntime` to provide a clean
/// API for embedding CADE in other applications.
use std::path::PathBuf;
use std::sync::Arc;

use cade_agent::agent::client::{HttpTransport, MemoryBlock};
use cade_agent::mcp::McpManager;
use cade_agent::tools::ToolRuntime;
use cade_core::permissions::{PermissionManager, PermissionMode};
use cade_core::skills::Skill;

use crate::Result;

// region:    --- SessionOptions

pub struct SessionOptions {
    /// URL of the cade-server (default: http://localhost:8284).
    pub server_url: String,
    /// CADE API key (default: empty = no auth).
    pub api_key: String,
    /// Resume an existing agent by ID.  `None` = let the server create one.
    pub agent_id: Option<String>,
    /// Model to use (e.g. "anthropic/claude-sonnet-4-5").
    pub model: Option<String>,
    /// Working directory for skill/tool path resolution.
    pub cwd: PathBuf,
    /// Permission mode (default: BypassPermissions for SDK use).
    pub permission_mode: PermissionMode,
    /// Allowed paths for granular RBAC file sandboxing.
    pub allowed_paths: Option<Vec<String>>,
}

impl Default for SessionOptions {
    fn default() -> Self {
        Self {
            server_url: "http://localhost:8284".to_string(),
            api_key: String::new(),
            agent_id: None,
            model: None,
            cwd: std::env::current_dir().unwrap_or_default(),
            permission_mode: PermissionMode::BypassPermissions,
            allowed_paths: None,
        }
    }
}

// endregion: --- SessionOptions

// region:    --- AgentSession

/// A stateful agent session.
pub struct AgentSession {
    client: Arc<HttpTransport>,
    runtime: ToolRuntime,
    agent_id: String,
    #[allow(dead_code)]
    permissions: PermissionManager,
}

impl AgentSession {
    // -- Constructor

    pub async fn create(opts: SessionOptions) -> Result<Self> {
        let client = Arc::new(
            HttpTransport::new(opts.server_url.clone(), opts.api_key.clone())
                .map_err(|e| crate::Error::custom(format!("connect: {e}")))?,
        );

        // Ensure server is running
        if !client.health().await.unwrap_or(false) {
            return Err(crate::Error::custom(
                "Cannot connect to cade-server. Is it running?",
            ));
        }

        // Resolve agent
        let agent_id = match opts.agent_id {
            Some(id) => id,
            None => {
                let model = opts
                    .model
                    .as_deref()
                    .unwrap_or("anthropic/claude-sonnet-4-5");
                let req = cade_agent::agent::client::CreateAgentRequest {
                    name: Some(format!("sdk-{}", uuid::Uuid::new_v4())),
                    model: model.to_string(),
                    description: Some("SDK agent".to_string()),
                    system_prompt: None,
                    memory_blocks: Vec::new(),
                    tool_ids: Vec::new(),
                };
                let agent = client
                    .create_agent(req)
                    .await
                    .map_err(|e| crate::Error::custom(format!("create agent: {e}")))?;
                agent.id
            }
        };

        let mcp = Arc::new(McpManager::empty());
        let mut runtime = ToolRuntime::new(
            Arc::clone(&client),
            Arc::clone(&mcp),
            agent_id.clone(),
            opts.cwd,
        );
        runtime.allowed_paths = opts.allowed_paths;
        
        let permissions = PermissionManager::new(opts.permission_mode);

        Ok(Self {
            client,
            runtime,
            agent_id,
            permissions,
        })
    }

    // -- Agent info

    pub fn agent_id(&self) -> &str {
        &self.agent_id
    }

    // -- Prompting

    /// Send a prompt and return the final assistant text.
    pub async fn prompt(&self, text: &str) -> Result<String> {
        let messages = self
            .client
            .stream_message(&self.agent_id, text, |_msg| {})
            .await
            .map_err(|e| crate::Error::custom(format!("stream: {e}")))?;

        let text: String = messages
            .iter()
            .filter_map(|m| m.assistant_text())
            .collect::<Vec<_>>()
            .join("");

        Ok(text)
    }

    /// Send a prompt with a streaming callback that receives text deltas.
    pub async fn prompt_stream(
        &self,
        text: &str,
        on_delta: impl Fn(&str) + Send,
    ) -> Result<String> {
        let messages = self
            .client
            .stream_message(&self.agent_id, text, |msg| {
                if let Some(t) = msg.assistant_text() {
                    on_delta(t);
                }
            })
            .await
            .map_err(|e| crate::Error::custom(format!("stream: {e}")))?;

        let full: String = messages
            .iter()
            .filter_map(|m| m.assistant_text())
            .collect::<Vec<_>>()
            .join("");
        Ok(full)
    }

    // -- Memory

    /// Retrieve the value of a memory block.
    pub async fn get_memory(&self, label: &str) -> Result<Option<String>> {
        let blocks = self
            .client
            .get_memory(&self.agent_id)
            .await
            .map_err(|e| crate::Error::custom(format!("get_memory: {e}")))?;
        Ok(blocks
            .into_iter()
            .find(|b| b.label == label)
            .map(|b| b.value))
    }

    /// Set a memory block.
    pub async fn set_memory(&self, label: &str, value: &str) -> Result<()> {
        self.client
            .upsert_memory(&self.agent_id, label, value, None)
            .await
            .map_err(|e| crate::Error::custom(format!("set_memory: {e}")))
    }

    /// List all memory blocks for this agent.
    pub async fn list_memory(&self) -> Result<Vec<MemoryBlock>> {
        self.client
            .get_memory(&self.agent_id)
            .await
            .map_err(|e| crate::Error::custom(format!("list_memory: {e}")))
    }

    // -- Skills

    /// List all available skills for the current working directory.
    pub fn list_skills(&self) -> Vec<Skill> {
        cade_core::skills::discover_all_skills(&self.runtime.cwd, Some(&self.agent_id), None)
    }
}

// endregion: --- AgentSession
