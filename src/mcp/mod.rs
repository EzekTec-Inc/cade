//! MCP (Model Context Protocol) client integration.
//!
//! Spawns configured MCP servers as child processes (stdio transport),
//! discovers their tools, and routes tool calls from the LLM to the
//! appropriate server.
//!
//! Tool names are prefixed with `{server_key}__` to avoid collisions:
//!   `git__status`, `developer__bash`, etc.

use anyhow::{Context, Result};
use rmcp::{
    RoleClient, ServiceExt,
    model::{CallToolRequestParam, RawContent},
    service::RunningService,
    transport::TokioChildProcess,
};
use serde_json::{Value, json};
use std::collections::HashMap;
use tokio::process::Command;
use tracing::{info, warn};

use crate::settings::McpServerConfig;

// ── Types ─────────────────────────────────────────────────────────────────────

/// Public summary of a running MCP server (for `/mcp` command display).
#[derive(Debug, Clone)]
pub struct McpStatus {
    pub key:      String,
    pub command:  String,
    pub tools:    Vec<String>, // prefixed names
}

/// A cached tool schema in OpenAI-compatible format.
#[derive(Debug, Clone)]
pub struct McpToolSchema {
    pub server_key:   String,
    pub prefixed_name: String,
    pub original_name: String,
    pub schema:        Value, // OpenAI-compatible: { name, description, parameters }
    /// If true, calling this tool requires user permission.
    pub is_write:      bool,
}

// ── McpServer ─────────────────────────────────────────────────────────────────

struct McpServer {
    key:     String,
    command: String,
    tools:   Vec<McpToolSchema>,
    /// The live peer — kept alive as long as this struct exists.
    _service: RunningService<RoleClient, ()>,
    peer:     rmcp::Peer<RoleClient>,
}

// ── McpManager ────────────────────────────────────────────────────────────────

/// Manages all active MCP server connections.
///
/// Constructed once at startup; passed as `Arc<McpManager>` to the REPL.
/// All methods take `&self` (thread-safe).
pub struct McpManager {
    servers: Vec<McpServer>,
}

impl McpManager {
    /// Spawn all enabled MCP servers, handshake, and fetch their tool lists.
    /// Servers that fail to start are skipped with a warning.
    pub async fn start(configs: &HashMap<String, McpServerConfig>) -> Self {
        let mut servers = Vec::new();

        // Sort for deterministic startup order
        let mut entries: Vec<(&String, &McpServerConfig)> = configs.iter().collect();
        entries.sort_by_key(|(k, _)| k.as_str());

        for (key, config) in entries {
            match Self::connect_server(key, config).await {
                Ok(server) => {
                    info!(
                        "MCP server '{}' ready — {} tool(s)",
                        key,
                        server.tools.len()
                    );
                    servers.push(server);
                }
                Err(e) => {
                    warn!("MCP server '{}' failed to start: {e}", key);
                }
            }
        }

        McpManager { servers }
    }

    /// No-op (empty) manager — used when no servers are configured.
    pub fn empty() -> Self {
        McpManager { servers: vec![] }
    }

    /// Returns true if any servers are configured.
    pub fn is_empty(&self) -> bool {
        self.servers.is_empty()
    }

    /// All tool schemas across all servers (OpenAI-compatible).
    pub fn all_tool_schemas(&self) -> Vec<Value> {
        self.servers
            .iter()
            .flat_map(|s| s.tools.iter().map(|t| t.schema.clone()))
            .collect()
    }

    /// Returns true if this manager owns the given prefixed tool name.
    pub fn owns_tool(&self, prefixed_name: &str) -> bool {
        self.find_tool(prefixed_name).is_some()
    }

    /// Call a prefixed MCP tool. Returns `None` if no server owns it.
    pub async fn call_tool(
        &self,
        prefixed_name: &str,
        args: &Value,
    ) -> Option<Result<(String, bool)>> {
        let (server_idx, original_name) = self.find_tool(prefixed_name)?;
        let server = &self.servers[server_idx];

        let arguments = args.as_object().cloned();
        let result = server
            .peer
            .call_tool(CallToolRequestParam {
                name: original_name.into(),
                arguments,
            })
            .await;

        Some(match result {
            Ok(ctr) => {
                let is_error = ctr.is_error.unwrap_or(false);
                let text = extract_content_text(&ctr.content);
                Ok((text, is_error))
            }
            Err(e) => Err(anyhow::anyhow!("MCP call failed: {e}")),
        })
    }

    /// Whether a tool requires user permission (mutable tools).
    pub fn is_write_tool(&self, prefixed_name: &str) -> bool {
        self.find_tool_schema(prefixed_name)
            .map(|t| t.is_write)
            .unwrap_or(true) // default to write (safe)
    }

    /// Summary of all active servers (for `/mcp` command).
    pub fn status(&self) -> Vec<McpStatus> {
        self.servers
            .iter()
            .map(|s| McpStatus {
                key:     s.key.clone(),
                command: s.command.clone(),
                tools:   s.tools.iter().map(|t| t.prefixed_name.clone()).collect(),
            })
            .collect()
    }

    // ── Internal helpers ──────────────────────────────────────────────────────

    async fn connect_server(key: &str, config: &McpServerConfig) -> Result<McpServer> {
        let mut cmd = Command::new(&config.command);
        cmd.args(&config.args);
        for (k, v) in &config.env {
            cmd.env(k, v);
        }
        // Suppress server stderr from polluting CADE's terminal
        cmd.stderr(std::process::Stdio::null());

        let transport = TokioChildProcess::new(cmd)
            .with_context(|| format!("spawn MCP server '{key}' ({})", config.command))?;

        let service = ()
            .serve(transport)
            .await
            .with_context(|| format!("handshake with MCP server '{key}'"))?;

        let peer = service.peer().clone();

        // Fetch all tools (paginated)
        let raw_tools = peer
            .list_all_tools()
            .await
            .with_context(|| format!("list_tools from '{key}'"))?;

        let write_set: std::collections::HashSet<&str> =
            config.write_tools.iter().map(|s| s.as_str()).collect();

        let tools: Vec<McpToolSchema> = raw_tools
            .into_iter()
            .map(|tool| {
                let original = tool.name.to_string();
                let prefixed = format!("{key}__{original}");
                let description = tool
                    .description
                    .as_deref()
                    .unwrap_or("")
                    .to_string();

                // Convert MCP input_schema (JsonObject) to OpenAI parameters Value
                let parameters = Value::Object((*tool.input_schema).clone());

                // Infer write tool:
                // 1. Explicit config.write_tools list (if non-empty → whitelist mode)
                // 2. If list is empty → default: all tools are write (conservative)
                // 3. Check ToolAnnotations.readOnlyHint if available
                let is_write = if !config.write_tools.is_empty() {
                    // whitelist mode: only listed tools are write
                    write_set.contains(original.as_str())
                } else if let Some(ann) = &tool.annotations {
                    // use MCP hint: readOnlyHint = true → not a write tool
                    !ann.read_only_hint.unwrap_or(false)
                } else {
                    true // conservative default
                };

                let schema = json!({
                    "name":        prefixed,
                    "description": description,
                    "parameters":  parameters,
                });

                McpToolSchema {
                    server_key:    key.to_string(),
                    prefixed_name: prefixed,
                    original_name: original,
                    schema,
                    is_write,
                }
            })
            .collect();

        Ok(McpServer {
            key:      key.to_string(),
            command:  config.command.clone(),
            tools,
            _service: service,
            peer,
        })
    }

    /// Find (server_index, original_tool_name) for a prefixed tool name.
    fn find_tool(&self, prefixed_name: &str) -> Option<(usize, String)> {
        for (i, server) in self.servers.iter().enumerate() {
            if let Some(t) = server.tools.iter().find(|t| t.prefixed_name == prefixed_name) {
                return Some((i, t.original_name.clone()));
            }
        }
        None
    }

    fn find_tool_schema(&self, prefixed_name: &str) -> Option<&McpToolSchema> {
        self.servers
            .iter()
            .flat_map(|s| &s.tools)
            .find(|t| t.prefixed_name == prefixed_name)
    }
}

// ── Content extraction ────────────────────────────────────────────────────────

fn extract_content_text(content: &[rmcp::model::Content]) -> String {
    // Some MCP servers emit two content items per result:
    //   • one with audience=[Assistant]  (for the LLM)
    //   • one with audience=[User]       (for the human UI)
    // Joining both would duplicate the output. We keep only:
    //   • items whose audience includes Assistant, OR
    //   • items with no audience annotation (generic / unspecified)
    // This mirrors how compliant MCP clients filter content.
    let assistant_items: Vec<&rmcp::model::Content> = content
        .iter()
        .filter(|c| {
            match c.audience() {
                None         => true, // no audience = include for everyone
                Some(roles)  => roles.contains(&rmcp::model::Role::Assistant),
            }
        })
        .collect();

    // If filtering left nothing (shouldn't happen, but be safe), fall back to all items
    let items = if assistant_items.is_empty() { content.iter().collect() } else { assistant_items };

    items
        .iter()
        .map(|c| match &c.raw {
            RawContent::Text(t)     => t.text.clone(),
            RawContent::Image(_)    => "[image]".to_string(),
            RawContent::Audio(_)    => "[audio]".to_string(),
            RawContent::Resource(r) => match &r.resource {
                rmcp::model::ResourceContents::TextResourceContents { text, .. } => text.clone(),
                _ => "[binary resource]".to_string(),
            },
        })
        .collect::<Vec<_>>()
        .join("\n")
}
