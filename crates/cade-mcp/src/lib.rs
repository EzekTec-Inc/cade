//! MCP (Model Context Protocol) client integration & gateway.
//!
//! Spawns configured MCP servers as child processes (stdio transport) or connects
//! over remote HTTP/SSE, discovers tools, and routes tool calls.
//!
//! Tool names are prefixed with `{server_key}__` to avoid collisions:
//!   `git__status`, `developer__bash`, etc.

// region:    --- Modules

mod error;
pub mod schema;
pub mod transport;
pub mod watcher;

pub use error::{Error, Result};
pub use schema::{McpToolSchema, ToolSchemaNormalizer};
pub use transport::{HttpTransportAdapter, SingletonProcessGuard, StdioTransportAdapter};

use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use tokio::sync::RwLock;
use tracing::{error, info, warn};
use serde_json::Value;

use rmcp::{
    RoleClient,
    model::{CallToolRequestParams, RawContent},
    service::RunningService,
};

use cade_core::settings::McpServerConfig;

// endregion: --- Modules

// region:    --- Constants

const MAX_RECONNECT_ATTEMPTS: u32 = 3;
const RECONNECT_DELAY_SECS: u64 = 2;
/// Maximum time (in seconds) to wait for a single MCP server to spawn,
/// complete the JSON-RPC handshake, and report its tool list.
const MCP_SERVER_TIMEOUT_SECS: u64 = 45;

// endregion: --- Constants

// region:    --- Types

/// Result of a single MCP server startup attempt — used by the progress reporter.
#[derive(Debug, Clone)]
pub enum McpStartResult {
    /// Server connected and reported its tools.
    Ok { key: String, tool_count: usize },
    /// Server failed to start (spawn error, handshake failure, etc.).
    Failed { key: String, error: String },
    /// Server exceeded the per-server startup timeout.
    Timeout { key: String, timeout_secs: u64 },
}

impl McpStartResult {
    pub fn key(&self) -> &str {
        match self {
            Self::Ok { key, .. } => key,
            Self::Failed { key, .. } => key,
            Self::Timeout { key, .. } => key,
        }
    }
}

/// Public summary of a running MCP server (for status & /mcp command display).
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct McpStatus {
    pub key: String,
    pub command: String,
    pub tools: Vec<String>, // prefixed names
    pub disabled: bool,
}

/// Trait for routing MCP operations to a remote CADE server.
#[async_trait::async_trait]
pub trait RemoteMcpClient: Send + Sync {
    async fn call_mcp_tool(
        &self,
        name: &str,
        arguments: &Value,
    ) -> Result<(String, bool, Option<String>)>;

    async fn list_mcp_statuses(&self) -> Result<Vec<McpStatus>>;
}

/// Summary returned by `McpManager::reload()`.
#[derive(Debug, Default, Clone, serde::Serialize, serde::Deserialize)]
pub struct ReloadSummary {
    pub started: Vec<String>,
    pub stopped: Vec<String>,
    pub kept: Vec<String>,
    pub failed: Vec<String>,
}

struct McpServer {
    key: String,
    command: String,
    tools: Vec<McpToolSchema>,
    config: McpServerConfig,
    reconnect_attempts: u32,
    disabled: bool,
    _service: RunningService<RoleClient, ()>,
    peer: rmcp::Peer<RoleClient>,
    _singleton_guard: Option<SingletonProcessGuard>,
}

// endregion: --- Types

// region:    --- McpGateway / McpManager

/// Central gateway managing active MCP server connections.
pub struct McpManager {
    servers: RwLock<Vec<McpServer>>,
    pub schemas_dirty: Arc<AtomicBool>,
    pub remote_client: Option<Arc<dyn RemoteMcpClient>>,
}

/// Type alias for deep module naming.
pub type McpGateway = McpManager;

fn existing_identity<'a>(server: &Option<&'a McpServer>) -> Option<&'a str> {
    let s = (*server)?;
    let cmd = s.command.as_str();
    if let Some(url) = cmd.strip_prefix("[http] ") {
        Some(url)
    } else {
        Some(cmd)
    }
}

impl McpManager {
    /// Spawn all enabled MCP servers, handshake, and fetch their tool lists.
    pub async fn start(
        configs: &HashMap<String, McpServerConfig>,
        mut on_progress: Option<&mut (dyn FnMut(McpStartResult) + Send)>,
    ) -> (Self, Vec<McpStartResult>) {
        let mut servers = Vec::new();
        let mut results = Vec::new();

        let mut entries: Vec<(&String, &McpServerConfig)> = configs.iter().collect();
        entries.sort_by_key(|(k, _)| k.as_str());

        let timeout_dur = std::time::Duration::from_secs(MCP_SERVER_TIMEOUT_SECS);

        let mut join_set = tokio::task::JoinSet::new();
        for (key, config) in entries {
            let k = key.clone();
            let c = config.clone();
            join_set.spawn(async move {
                let res = tokio::time::timeout(timeout_dur, Self::connect_server(&k, &c)).await;
                (k, res)
            });
        }

        while let Some(Ok((key, result))) = join_set.join_next().await {
            let res = match result {
                Ok(Ok(server)) => {
                    let count = server.tools.len();
                    info!("MCP server '{}' ready — {} tool(s)", key, count);
                    let r = McpStartResult::Ok {
                        key: key.clone(),
                        tool_count: count,
                    };
                    servers.push(server);
                    r
                }
                Ok(Err(e)) => {
                    let msg = e.to_string();
                    warn!("MCP server '{}' failed to start: {msg}", key);
                    McpStartResult::Failed {
                        key: key.clone(),
                        error: msg,
                    }
                }
                Err(_elapsed) => {
                    warn!(
                        "MCP server '{}' timed out after {}s — skipping",
                        key, MCP_SERVER_TIMEOUT_SECS
                    );
                    McpStartResult::Timeout {
                        key: key.clone(),
                        timeout_secs: MCP_SERVER_TIMEOUT_SECS,
                    }
                }
            };
            results.push(res.clone());
            if let Some(ref mut cb) = on_progress {
                cb(res);
            }
        }

        let mgr = McpManager {
            servers: RwLock::new(servers),
            schemas_dirty: Arc::new(AtomicBool::new(false)),
            remote_client: None,
        };
        (mgr, results)
    }

    /// Construct an McpManager that delegates tool execution to a remote CADE server.
    pub fn from_remote(remote: Arc<dyn RemoteMcpClient>) -> Self {
        McpManager {
            servers: RwLock::new(vec![]),
            schemas_dirty: Arc::new(AtomicBool::new(false)),
            remote_client: Some(remote),
        }
    }

    /// No-op (empty) manager.
    pub fn empty() -> Self {
        McpManager {
            servers: RwLock::new(vec![]),
            schemas_dirty: Arc::new(AtomicBool::new(false)),
            remote_client: None,
        }
    }

    /// Merge servers from a completed background boot into this manager.
    pub async fn merge_from(&self, other: McpManager) {
        let new_servers = other.servers.into_inner();
        let mut current = self.servers.write().await;
        current.extend(new_servers);
        self.schemas_dirty.store(true, Ordering::SeqCst);
    }

    /// Dynamically start and add a single MCP server on-demand.
    pub async fn start_and_add_server(&self, key: &str, config: &McpServerConfig) -> Result<()> {
        let server = Self::connect_server(key, config).await?;
        let mut servers = self.servers.write().await;
        servers.retain(|s| s.key != key);
        servers.push(server);
        self.schemas_dirty.store(true, Ordering::SeqCst);
        Ok(())
    }

    /// Reload MCP servers from a new config map.
    pub async fn reload(
        &self,
        new_configs: &HashMap<String, McpServerConfig>,
        mut on_progress: Option<&mut (dyn FnMut(McpStartResult) + Send)>,
    ) -> ReloadSummary {
        let mut summary = ReloadSummary::default();

        let mut entries: Vec<(&String, &McpServerConfig)> = new_configs.iter().collect();
        entries.sort_by_key(|(k, _)| k.as_str());

        let timeout_dur = std::time::Duration::from_secs(MCP_SERVER_TIMEOUT_SECS);

        let mut to_restart = Vec::new();
        let mut preserved = Vec::new();

        {
            let mut current = self.servers.write().await;
            for (key, cfg) in &entries {
                let target_identity = cfg.url.as_deref().unwrap_or(&cfg.command);
                let existing = current.iter().find(|s| &s.key == *key);

                if existing_identity(&existing) == Some(target_identity) {
                    preserved.push((*key).clone());
                } else {
                    to_restart.push(((*key).clone(), (*cfg).clone()));
                }
            }

            let new_keys: HashSet<&str> = new_configs.keys().map(|s| s.as_str()).collect();
            let mut kept_servers = Vec::new();

            for srv in current.drain(..) {
                if !new_keys.contains(srv.key.as_str()) {
                    summary.stopped.push(srv.key.clone());
                } else if preserved.contains(&srv.key) {
                    summary.kept.push(srv.key.clone());
                    kept_servers.push(srv);
                } else {
                    summary.stopped.push(srv.key.clone());
                }
            }
            *current = kept_servers;
        }

        let mut join_set = tokio::task::JoinSet::new();
        for (key, config) in to_restart {
            let k = key.clone();
            let c = config.clone();
            join_set.spawn(async move {
                let res = tokio::time::timeout(timeout_dur, Self::connect_server(&k, &c)).await;
                (k, res)
            });
        }

        while let Some(Ok((key, result))) = join_set.join_next().await {
            match result {
                Ok(Ok(new_server)) => {
                    let count = new_server.tools.len();
                    info!("MCP server '{key}' (re)started — {count} tool(s)");
                    summary.started.push(key.clone());
                    let mut current = self.servers.write().await;
                    current.push(new_server);
                    if let Some(ref mut cb) = on_progress {
                        cb(McpStartResult::Ok {
                            key,
                            tool_count: count,
                        });
                    }
                }
                Ok(Err(e)) => {
                    let msg = e.to_string();
                    warn!("MCP server '{key}' failed to start during reload: {msg}");
                    summary.failed.push(key.clone());
                    if let Some(ref mut cb) = on_progress {
                        cb(McpStartResult::Failed {
                            key,
                            error: msg,
                        });
                    }
                }
                Err(_) => {
                    warn!("MCP server '{key}' timed out during reload ({MCP_SERVER_TIMEOUT_SECS}s)");
                    summary.failed.push(key.clone());
                    if let Some(ref mut cb) = on_progress {
                        cb(McpStartResult::Timeout {
                            key,
                            timeout_secs: MCP_SERVER_TIMEOUT_SECS,
                        });
                    }
                }
            }
        }

        self.schemas_dirty.store(true, Ordering::SeqCst);
        summary
    }

    /// Returns true if no servers are configured or connected.
    pub async fn is_empty(&self) -> bool {
        self.servers.read().await.is_empty() && self.remote_client.is_none()
    }

    /// Return all cached tool schemas across all servers in OpenAI Value format.
    pub async fn all_tool_schemas(&self) -> Vec<Value> {
        let servers = self.servers.read().await;
        servers
            .iter()
            .flat_map(|s| s.tools.iter().map(|t| t.schema.clone()))
            .collect()
    }

    /// Return all cached typed tool schemas across all servers.
    pub async fn all_typed_tool_schemas(&self) -> Vec<McpToolSchema> {
        let servers = self.servers.read().await;
        servers.iter().flat_map(|s| s.tools.clone()).collect()
    }

    /// Return all cached tool schemas for a specific server.
    pub async fn schemas_for_server(&self, server_key: &str) -> Vec<McpToolSchema> {
        let servers = self.servers.read().await;
        servers
            .iter()
            .find(|s| s.key == server_key)
            .map(|s| s.tools.clone())
            .unwrap_or_default()
    }

    /// Return a public status summary for every managed server.
    pub async fn status(&self) -> Vec<McpStatus> {
        let servers = self.servers.read().await;
        if !servers.is_empty() {
            return servers
                .iter()
                .map(|s| McpStatus {
                    key: s.key.clone(),
                    command: s.command.clone(),
                    tools: s.tools.iter().map(|t| t.prefixed_name.clone()).collect(),
                    disabled: s.disabled,
                })
                .collect();
        }
        if let Some(remote) = &self.remote_client {
            return remote.list_mcp_statuses().await.unwrap_or_default();
        }
        vec![]
    }

    fn is_rpc_protocol_error(msg: &str) -> bool {
        msg.contains("Mcp error:") || msg.contains("jsonrpc error")
    }

    /// Check if this manager has a connected server that owns the specified tool.
    pub async fn owns_tool(&self, prefixed_name: &str) -> bool {
        self.find_tool_idx(prefixed_name).await.is_some()
    }

    /// Check if a tool owned by this manager is marked as a mutating/write tool.
    pub async fn is_write_tool(&self, prefixed_name: &str) -> bool {
        if let Some((_, is_write)) = self.find_tool_idx(prefixed_name).await {
            is_write
        } else {
            false
        }
    }

    async fn find_tool_idx(&self, prefixed_name: &str) -> Option<(usize, bool)> {
        let servers = self.servers.read().await;
        for (i, server) in servers.iter().enumerate() {
            if let Some(tool) = server.tools.iter().find(|t| t.prefixed_name == prefixed_name) {
                return Some((i, tool.is_write));
            }
        }
        None
    }

    /// Call a prefixed MCP tool with automatic reconnect on transport failure.
    pub async fn call_tool(
        &self,
        prefixed_name: &str,
        args: &Value,
    ) -> Option<Result<(String, bool, Option<String>)>> {
        let server_idx = match self.find_tool_idx(prefixed_name).await {
            Some((idx, _)) => idx,
            None => {
                if let Some(remote) = &self.remote_client {
                    return Some(remote.call_mcp_tool(prefixed_name, args).await);
                }
                return None;
            }
        };

        let (is_disabled, server_key, original_name, peer) = {
            let servers = self.servers.read().await;
            let server = &servers[server_idx];
            let orig = server
                .tools
                .iter()
                .find(|t| t.prefixed_name == prefixed_name)
                .map(|t| t.original_name.clone())
                .unwrap_or_default();
            (
                server.disabled,
                server.key.clone(),
                orig,
                server.peer.clone(),
            )
        };

        if is_disabled {
            return Some(Err(Error::custom(format!(
                "MCP server '{server_key}' is disabled after {MAX_RECONNECT_ATTEMPTS} failed reconnect attempts"
            ))));
        }

        let call_result = peer
            .call_tool(
                CallToolRequestParams::new(original_name)
                    .with_arguments(args.as_object().cloned().unwrap_or_default()),
            )
            .await;

        let call_err = match call_result {
            Ok(ctr) => {
                let is_error = ctr.is_error.unwrap_or(false);
                let text = extract_content_text(&ctr.content);
                let ui_resource_uri = ctr.meta.as_ref().and_then(|meta| {
                    serde_json::to_value(meta).ok().and_then(|val| {
                        val.get("ui")
                            .and_then(|ui| ui.get("resourceUri"))
                            .and_then(|uri| uri.as_str().map(String::from))
                    })
                });
                return Some(Ok((text, is_error, ui_resource_uri)));
            }
            Err(e) => e,
        };

        let error_msg = call_err.to_string();

        if Self::is_rpc_protocol_error(&error_msg) {
            return Some(Err(Error::custom(error_msg)));
        }

        warn!(
            "MCP server call failed for '{prefixed_name}': {error_msg} — attempting reconnect"
        );

        for attempt in 1..=MAX_RECONNECT_ATTEMPTS {
            warn!(
                "MCP reconnect attempt {attempt}/{MAX_RECONNECT_ATTEMPTS} for server at index {server_idx}…"
            );
            tokio::time::sleep(tokio::time::Duration::from_secs(RECONNECT_DELAY_SECS)).await;

            let old_tool_names: HashSet<String> = {
                let s = self.servers.read().await;
                s.get(server_idx)
                    .map(|srv| srv.tools.iter().map(|t| t.prefixed_name.clone()).collect())
                    .unwrap_or_default()
            };

            let (key, config) = {
                let servers = self.servers.read().await;
                let s = &servers[server_idx];
                (s.key.clone(), s.config.clone())
            };

            match Self::connect_server(&key, &config).await {
                Ok(new_server) => {
                    info!("MCP server '{key}' reconnected successfully");

                    let original_name = new_server
                        .tools
                        .iter()
                        .find(|t| t.prefixed_name == prefixed_name)
                        .map(|t| t.original_name.clone());

                    let call_result = if let Some(orig) = original_name {
                        new_server
                            .peer
                            .call_tool(
                                CallToolRequestParams::new(orig)
                                    .with_arguments(args.as_object().cloned().unwrap_or_default()),
                            )
                            .await
                            .map_err(|e| Error::custom(e.to_string()))
                    } else {
                        Err(Error::custom(format!(
                            "Tool '{prefixed_name}' no longer exposed by reconnected server '{key}'"
                        )))
                    };

                    let new_tool_names: HashSet<String> = new_server
                        .tools
                        .iter()
                        .map(|t| t.prefixed_name.clone())
                        .collect();
                    let tools_changed = old_tool_names != new_tool_names;

                    {
                        let mut servers = self.servers.write().await;
                        if let Some(srv) = servers.get_mut(server_idx) {
                            *srv = new_server;
                        }
                    }

                    if tools_changed {
                        self.schemas_dirty.store(true, Ordering::SeqCst);
                    }

                    return match call_result {
                        Ok(ctr) => {
                            let is_error = ctr.is_error.unwrap_or(false);
                            let text = extract_content_text(&ctr.content);
                            let ui_resource_uri = ctr.meta.as_ref().and_then(|meta| {
                                serde_json::to_value(meta).ok().and_then(|val| {
                                    val.get("ui")
                                        .and_then(|ui| ui.get("resourceUri"))
                                        .and_then(|uri| uri.as_str().map(String::from))
                                })
                            });
                            Some(Ok((text, is_error, ui_resource_uri)))
                        }
                        Err(e) => Some(Err(Error::custom(e.to_string()))),
                    };
                }
                Err(e) => {
                    warn!("Reconnect attempt {attempt} for '{key}' failed: {e}");
                }
            }
        }

        error!(
            "MCP server '{server_key}' marked DISABLED after {MAX_RECONNECT_ATTEMPTS} failed reconnects"
        );
        {
            let mut servers = self.servers.write().await;
            if let Some(srv) = servers.get_mut(server_idx) {
                srv.disabled = true;
                srv.reconnect_attempts = MAX_RECONNECT_ATTEMPTS;
            }
        }

        Some(Err(Error::custom(format!(
            "MCP server '{server_key}' failed and could not be reconnected after {MAX_RECONNECT_ATTEMPTS} attempts: {error_msg}"
        ))))
    }

    async fn connect_server(key: &str, config: &McpServerConfig) -> Result<McpServer> {
        if let Some(url) = &config.url {
            let (service, peer) = HttpTransportAdapter::connect(key, config, url).await?;
            Self::build_server_from_peer(key, config, peer, service, format!("[http] {url}"), None).await
        } else {
            let (service, peer, singleton_guard) = StdioTransportAdapter::connect(key, config).await?;
            Self::build_server_from_peer(
                key,
                config,
                peer,
                service,
                config.command.clone(),
                Some(singleton_guard),
            )
            .await
        }
    }

    async fn build_server_from_peer(
        key: &str,
        config: &McpServerConfig,
        peer: rmcp::Peer<RoleClient>,
        service: RunningService<RoleClient, ()>,
        command_display: String,
        singleton_guard: Option<SingletonProcessGuard>,
    ) -> Result<McpServer> {
        let raw_tools = peer
            .list_all_tools()
            .await
            .map_err(|e| Error::custom(format!("list_tools from '{key}': {e}")))?;

        let tools: Vec<McpToolSchema> = raw_tools
            .into_iter()
            .map(|tool| ToolSchemaNormalizer::normalize(key, &tool, &config.write_tools))
            .collect();

        Ok(McpServer {
            key: key.to_string(),
            command: command_display,
            tools,
            config: config.clone(),
            reconnect_attempts: 0,
            disabled: false,
            _service: service,
            peer,
            _singleton_guard: singleton_guard,
        })
    }
}

#[async_trait::async_trait]
impl cade_core::capabilities::mesh::CapabilityMesh for McpManager {
    async fn execute(
        &self,
        intent: cade_core::capabilities::mesh::CapabilityIntent,
        _cx: &mut cade_core::capabilities::mesh::CapabilityExecutionContext,
    ) -> std::result::Result<
        cade_core::capabilities::mesh::CapabilityOutput,
        cade_core::capabilities::mesh::ExecutionError,
    > {
        match self
            .call_tool(&intent.capability_name, &intent.arguments)
            .await
        {
            Some(Ok((output, is_error, ui_resource_uri))) => {
                Ok(cade_core::capabilities::mesh::CapabilityOutput {
                    tool_call_id: intent.tool_call_id,
                    capability_name: intent.capability_name,
                    output,
                    is_error,
                    ui_resource_uri,
                })
            }
            Some(Err(e)) => {
                let err_str = e.to_string();
                if err_str.contains("disconnected") || err_str.contains("closed") {
                    Err(cade_core::capabilities::mesh::ExecutionError::Disconnected(
                        intent.capability_name,
                        err_str,
                    ))
                } else {
                    Err(cade_core::capabilities::mesh::ExecutionError::ExecutionFailed(
                        intent.capability_name,
                        err_str,
                    ))
                }
            }
            None => Err(cade_core::capabilities::mesh::ExecutionError::NotFound(
                intent.capability_name,
            )),
        }
    }

    async fn active_catalog(
        &self,
        _cx: &cade_core::capabilities::mesh::CapabilityExecutionContext,
    ) -> Vec<cade_core::capabilities::mesh::TaggedCapabilitySchema> {
        let schemas = self.all_typed_tool_schemas().await;
        schemas
            .into_iter()
            .map(|t| {
                let mut tags = vec!["cade".to_string(), "mcp".to_string()];
                if t.server_key == "cade-rag-mcp" || t.server_key == "serena" {
                    tags.push("core_mcp".to_string());
                }
                cade_core::capabilities::mesh::TaggedCapabilitySchema {
                    schema: t.schema,
                    tags,
                }
            })
            .collect()
    }
}

// endregion: --- McpGateway / McpManager

// region:    --- Content Extraction

fn extract_content_text(content: &[rmcp::model::Annotated<RawContent>]) -> String {
    content
        .iter()
        .filter_map(|c| match &c.raw {
            RawContent::Text(t) => Some(t.text.as_str()),
            _ => None,
        })
        .collect::<Vec<_>>()
        .join("\n")
}

// endregion: --- Content Extraction
