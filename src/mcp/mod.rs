//! MCP (Model Context Protocol) client integration.
//!
//! Spawns configured MCP servers as child processes (stdio transport),
//! discovers their tools, and routes tool calls from the LLM to the
//! appropriate server.
//!
//! Tool names are prefixed with `{server_key}__` to avoid collisions:
//!   `git__status`, `developer__bash`, etc.
//!
//! ## Reconnect behaviour
//!
//! If `call_tool` fails (process crash, broken pipe, etc.) the manager
//! automatically attempts to reconnect the affected server up to
//! `MAX_RECONNECT_ATTEMPTS` times with `RECONNECT_DELAY_SECS` between
//! each attempt.  After all attempts are exhausted the server is marked
//! `disabled`; its tools remain visible in the schema list (so the LLM
//! doesn't need to forget about them) but every call returns an error
//! explaining the situation. A `tracing::warn!` is emitted for each
//! reconnect attempt and a `tracing::error!` when a server is disabled.

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
use tokio::sync::RwLock;
use tracing::{error, info, warn};

use crate::settings::McpServerConfig;

// ── Reconnect constants ───────────────────────────────────────────────────────

const MAX_RECONNECT_ATTEMPTS: u32 = 3;
const RECONNECT_DELAY_SECS:   u64 = 2;

// ── Types ─────────────────────────────────────────────────────────────────────

/// Public summary of a running MCP server (for `/mcp` command display).
#[derive(Debug, Clone)]
pub struct McpStatus {
    pub key:      String,
    pub command:  String,
    pub tools:    Vec<String>, // prefixed names
    pub disabled: bool,
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
    /// Original config — needed to reconnect the child process.
    config:  McpServerConfig,
    /// Consecutive failed reconnect attempts since last success.
    reconnect_attempts: u32,
    /// If true, all reconnect attempts have been exhausted; calls fail immediately.
    disabled: bool,
    /// The live peer — kept alive as long as this struct exists.
    _service: RunningService<RoleClient, ()>,
    peer:     rmcp::Peer<RoleClient>,
}

// ── McpManager ────────────────────────────────────────────────────────────────

/// Manages all active MCP server connections.
///
/// Constructed once at startup; passed as `Arc<McpManager>` to the REPL.
/// All methods take `&self` (thread-safe via interior `RwLock`).
pub struct McpManager {
    /// Interior-mutable server list so `call_tool(&self)` can reconnect.
    servers: RwLock<Vec<McpServer>>,
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

        McpManager { servers: RwLock::new(servers) }
    }

    /// No-op (empty) manager — used when no servers are configured.
    pub fn empty() -> Self {
        McpManager { servers: RwLock::new(vec![]) }
    }

    /// Returns true if any servers are configured.
    pub async fn is_empty(&self) -> bool {
        self.servers.read().await.is_empty()
    }

    /// All tool schemas across all servers (OpenAI-compatible).
    pub async fn all_tool_schemas(&self) -> Vec<Value> {
        self.servers
            .read()
            .await
            .iter()
            .flat_map(|s| s.tools.iter().map(|t| t.schema.clone()))
            .collect()
    }

    /// Returns true if this manager owns the given prefixed tool name.
    pub async fn owns_tool(&self, prefixed_name: &str) -> bool {
        self.find_tool_idx(prefixed_name).await.is_some()
    }

    /// Returns true when the error looks like a JSON-RPC protocol error
    /// (server received and understood the call but rejected it).
    /// Protocol errors mean the connection is alive — reconnecting won't help.
    /// Only genuine transport failures (broken pipe, process crash) should
    /// trigger reconnect attempts.
    fn is_rpc_protocol_error(msg: &str) -> bool {
        // rmcp formats JSON-RPC errors as "Mcp error: -32XXX: ..."
        // Standard codes: -32700 (parse), -32600..=-32603, server-defined -32000..=-32099.
        msg.contains("Mcp error:") || msg.contains("jsonrpc error")
    }

    /// Call a prefixed MCP tool with automatic reconnect on failure.
    /// Returns `None` if no server owns the tool.
    pub async fn call_tool(
        &self,
        prefixed_name: &str,
        args: &Value,
    ) -> Option<Result<(String, bool)>> {
        let server_idx = self.find_tool_idx(prefixed_name).await?.0;

        // ── Fast path: try the call directly ─────────────────────────────────
        // Extract what we need under the read lock, then drop it before .await
        let (is_disabled, server_key, original_name, peer) = {
            let servers = self.servers.read().await;
            let server  = &servers[server_idx];
            let orig = server.tools
                .iter()
                .find(|t| t.prefixed_name == prefixed_name)
                .map(|t| t.original_name.clone())
                .unwrap_or_default();
            (server.disabled, server.key.clone(), orig, server.peer.clone())
        };

        if is_disabled {
            return Some(Err(anyhow::anyhow!(
                "MCP server '{}' is disabled after {} failed reconnect attempts",
                server_key, MAX_RECONNECT_ATTEMPTS
            )));
        }

        let arguments = args.as_object().cloned();
        let call_result = peer
            .call_tool(CallToolRequestParam {
                name: original_name.into(),
                arguments,
            })
            .await;

        let call_err = match call_result {
            Ok(ctr) => {
                let is_error = ctr.is_error.unwrap_or(false);
                let text = extract_content_text(&ctr.content);
                return Some(Ok((text, is_error)));
            }
            Err(e) => e,
        };

        // ── Slow path: call failed — attempt reconnect ────────────────────────
        let error_msg = call_err.to_string();

        // Protocol errors (-32XXX) mean the server is alive but rejected the call.
        // Reconnecting won't fix a bad argument or unknown method — return immediately.
        if Self::is_rpc_protocol_error(&error_msg) {
            return Some(Err(anyhow::anyhow!("{error_msg}")));
        }

        warn!(
            "MCP server call failed for '{}': {error_msg} — attempting reconnect",
            prefixed_name
        );

        for attempt in 1..=MAX_RECONNECT_ATTEMPTS {
            warn!(
                "MCP reconnect attempt {attempt}/{MAX_RECONNECT_ATTEMPTS} for server at index {server_idx}…"
            );
            tokio::time::sleep(tokio::time::Duration::from_secs(RECONNECT_DELAY_SECS)).await;

            // Re-read config for reconnect
            let (key, config) = {
                let servers = self.servers.read().await;
                let s = &servers[server_idx];
                (s.key.clone(), s.config.clone())
            };

            match Self::connect_server(&key, &config).await {
                Ok(new_server) => {
                    info!("MCP server '{}' reconnected successfully", key);

                    // Retry the original call on the new connection
                    let original_name = new_server.tools
                        .iter()
                        .find(|t| t.prefixed_name == prefixed_name)
                        .map(|t| t.original_name.clone());

                    let call_result = if let Some(orig) = original_name {
                        let arguments = args.as_object().cloned();
                        new_server
                            .peer
                            .call_tool(CallToolRequestParam {
                                name: orig.into(),
                                arguments,
                            })
                            .await
                    } else {
                        // Tool disappeared after reconnect — server API changed
                        let mut servers = self.servers.write().await;
                        servers[server_idx] = new_server;
                        return Some(Err(anyhow::anyhow!(
                            "Tool '{prefixed_name}' not found after MCP server reconnect"
                        )));
                    };

                    // Replace old server entry with the fresh connection
                    {
                        let mut servers = self.servers.write().await;
                        servers[server_idx] = new_server;
                    }

                    return Some(match call_result {
                        Ok(ctr) => {
                            let is_error = ctr.is_error.unwrap_or(false);
                            let text = extract_content_text(&ctr.content);
                            Ok((text, is_error))
                        }
                        Err(e) => Err(anyhow::anyhow!("MCP call failed after reconnect: {e}")),
                    });
                }
                Err(e) => {
                    warn!(
                        "MCP reconnect attempt {attempt}/{MAX_RECONNECT_ATTEMPTS} failed for '{}': {e}",
                        key
                    );
                    // Update reconnect_attempts counter
                    let mut servers = self.servers.write().await;
                    servers[server_idx].reconnect_attempts += 1;
                }
            }
        }

        // All reconnect attempts exhausted — disable the server
        {
            let mut servers = self.servers.write().await;
            let s = &mut servers[server_idx];
            s.disabled = true;
            error!(
                "MCP server '{}' disabled after {MAX_RECONNECT_ATTEMPTS} failed reconnect attempts",
                s.key
            );
        }

        Some(Err(anyhow::anyhow!(
            "MCP server disabled: all {MAX_RECONNECT_ATTEMPTS} reconnect attempts failed \
             (original error: {error_msg})"
        )))
    }

    /// Whether a tool requires user permission (mutable tools).
    pub async fn is_write_tool(&self, prefixed_name: &str) -> bool {
        self.find_tool_schema(prefixed_name).await
            .map(|t| t.is_write)
            .unwrap_or(true) // default to write (safe)
    }

    /// Summary of all active servers (for `/mcp` command).
    pub async fn status(&self) -> Vec<McpStatus> {
        self.servers
            .read()
            .await
            .iter()
            .map(|s| McpStatus {
                key:      s.key.clone(),
                command:  s.command.clone(),
                tools:    s.tools.iter().map(|t| t.prefixed_name.clone()).collect(),
                disabled: s.disabled,
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
                let mut parameters = Value::Object((*tool.input_schema).clone());

                // Bug 1 fix: OpenAI requires "properties" even if empty for "type": "object"
                if let Some(obj) = parameters.as_object_mut() {
                    if obj.get("type").and_then(|t| t.as_str()) == Some("object") && !obj.contains_key("properties") {
                        obj.insert("properties".to_string(), json!({}));
                    }
                }

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
            config:   config.clone(),
            reconnect_attempts: 0,
            disabled: false,
            _service: service,
            peer,
        })
    }

    /// Find (server_index, original_tool_name) for a prefixed tool name.
    async fn find_tool_idx(&self, prefixed_name: &str) -> Option<(usize, String)> {
        let servers = self.servers.read().await;
        for (i, server) in servers.iter().enumerate() {
            if let Some(t) = server.tools.iter().find(|t| t.prefixed_name == prefixed_name) {
                return Some((i, t.original_name.clone()));
            }
        }
        None
    }

    async fn find_tool_schema(&self, prefixed_name: &str) -> Option<McpToolSchema> {
        self.servers
            .read()
            .await
            .iter()
            .flat_map(|s| s.tools.iter())
            .find(|t| t.prefixed_name == prefixed_name)
            .cloned()
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
