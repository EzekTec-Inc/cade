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

use serde_json::Value;
use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use tokio::sync::{Mutex, RwLock, watch};
use tracing::{error, info, warn};

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

fn default_ready_status() -> String {
    "ready".to_string()
}

/// Public summary of a running MCP server (for status & /mcp command display).
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct McpStatus {
    pub key: String,
    pub command: String,
    pub tools: Vec<String>, // prefixed names
    /// Mutability for discovered tools. Missing entries are treated as writes by callers.
    #[serde(default)]
    pub tool_mutability: HashMap<String, bool>,
    /// Opaque connection identity, used to bind authorization to dispatch.
    #[serde(default)]
    pub generation: Option<String>,
    pub disabled: bool,
    #[serde(default = "default_ready_status")]
    pub status: String,
    #[serde(default)]
    pub error: Option<String>,
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

    async fn call_mcp_tool_bound(
        &self,
        _name: &str,
        _arguments: &Value,
        _generation: &str,
    ) -> Result<(String, bool, Option<String>)> {
        Err(Error::custom(
            "Remote MCP client does not support bound authorization",
        ))
    }
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
    generation: Arc<String>,
    key: String,
    command: String,
    tools: Vec<McpToolSchema>,
    config: McpServerConfig,
    _service: Option<RunningService<RoleClient, ()>>,
    peer: rmcp::Peer<RoleClient>,
    _singleton_guard: Option<SingletonProcessGuard>,
}

impl McpServer {
    fn is_ready(&self) -> bool {
        self._service.is_some() && !self.peer.is_transport_closed()
    }

    fn disconnected_snapshot(&self) -> Self {
        Self {
            generation: self.generation.clone(),
            key: self.key.clone(),
            command: self.command.clone(),
            tools: self.tools.clone(),
            config: self.config.clone(),
            peer: self.peer.clone(),
            _service: None,
            _singleton_guard: None,
        }
    }

    async fn shutdown(self) {
        // Stop and join the transport before releasing singleton ownership.
        // Dropping a guard asynchronously used to race the replacement spawn.
        // Cleanup retains the guard even if its reload/reconnect caller is cancelled.
        let _ = tokio::spawn(async move {
            let Self {
                _service,
                _singleton_guard,
                ..
            } = self;
            if let Some(service) = _service {
                let _ = service.cancel().await;
            }
            drop(_singleton_guard);
        })
        .await;
    }
}

// endregion: --- Types

// region:    --- McpGateway / McpManager

/// Diagnostic status and error recording for an MCP server.
#[derive(Debug, Clone)]
pub struct McpDiagnostic {
    pub status: String,
    pub command: String,
    pub error: Option<String>,
}

/// Central gateway managing active MCP server connections.
pub struct McpManager {
    servers: Arc<RwLock<Vec<McpServer>>>,
    // Serialize lifecycle mutations, never ordinary tool calls or catalog reads.
    lifecycle: Arc<Mutex<()>>,
    recovery_tasks: std::sync::Mutex<Vec<tokio::task::AbortHandle>>,
    catalog_changed: watch::Sender<u64>,
    pub schemas_dirty: Arc<AtomicBool>,
    pub remote_client: Option<Arc<dyn RemoteMcpClient>>,
    pub diagnostics: Arc<RwLock<HashMap<String, McpDiagnostic>>>,
}

/// Type alias for deep module naming.
pub type McpGateway = McpManager;

impl Drop for McpManager {
    fn drop(&mut self) {
        for task in self
            .recovery_tasks
            .get_mut()
            .unwrap_or_else(|e| e.into_inner())
            .drain(..)
        {
            task.abort();
        }
    }
}

pub(crate) fn server_command_display(config: &McpServerConfig) -> String {
    if let Some(url) = &config.url {
        format!("[http] {url}")
    } else {
        config.command.clone()
    }
}

fn same_configuration(left: &McpServerConfig, right: &McpServerConfig) -> bool {
    // Compare the complete serialized configuration, including future fields.
    // A command/URL identity alone misses auth, args, environment and policy changes.
    matches!((serde_json::to_value(left), serde_json::to_value(right)),
        (Ok(left), Ok(right)) if left == right)
}

impl McpManager {
    /// Spawn all enabled MCP servers, handshake, and fetch their tool lists.
    pub async fn start(
        configs: &HashMap<String, McpServerConfig>,
        mut on_progress: Option<&mut (dyn FnMut(McpStartResult) + Send)>,
    ) -> (Self, Vec<McpStartResult>) {
        let mut servers = Vec::new();
        let mut results = Vec::new();

        let mut entries: Vec<(&String, &McpServerConfig)> =
            configs.iter().filter(|(_, cfg)| !cfg.disabled).collect();
        entries.sort_by_key(|(k, _)| k.as_str());

        let timeout_dur = std::time::Duration::from_secs(MCP_SERVER_TIMEOUT_SECS);

        let mut join_set = tokio::task::JoinSet::new();
        for (key, config) in entries {
            let k = key.clone();
            let c = config.clone();
            join_set.spawn(async move {
                let res = tokio::time::timeout(timeout_dur, Self::connect_server(&k, &c)).await;
                (k, c, res)
            });
        }

        let mut diagnostics = HashMap::new();

        while let Some(Ok((key, config, result))) = join_set.join_next().await {
            let cmd_display = server_command_display(&config);
            let res = match result {
                Ok(Ok(server)) => {
                    let count = server.tools.len();
                    info!("MCP server '{}' ready — {} tool(s)", key, count);
                    let r = McpStartResult::Ok {
                        key: key.clone(),
                        tool_count: count,
                    };
                    diagnostics.insert(
                        key.clone(),
                        McpDiagnostic {
                            status: "ready".into(),
                            command: server.command.clone(),
                            error: None,
                        },
                    );
                    servers.push(server);
                    r
                }
                Ok(Err(e)) => {
                    let msg = e.to_string();
                    warn!("MCP server '{}' failed to start: {msg}", key);
                    diagnostics.insert(
                        key.clone(),
                        McpDiagnostic {
                            status: "failed".into(),
                            command: cmd_display,
                            error: Some(msg.clone()),
                        },
                    );
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
                    diagnostics.insert(
                        key.clone(),
                        McpDiagnostic {
                            status: "timeout".into(),
                            command: cmd_display,
                            error: Some(format!("Timed out after {MCP_SERVER_TIMEOUT_SECS}s")),
                        },
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
            servers: Arc::new(RwLock::new(servers)),
            lifecycle: Arc::new(Mutex::new(())),
            recovery_tasks: std::sync::Mutex::new(Vec::new()),
            catalog_changed: watch::channel(0).0,
            schemas_dirty: Arc::new(AtomicBool::new(false)),
            remote_client: None,
            diagnostics: Arc::new(RwLock::new(diagnostics)),
        };
        (mgr, results)
    }

    /// Construct an McpManager that delegates tool execution to a remote CADE server.
    pub fn from_remote(remote: Arc<dyn RemoteMcpClient>) -> Self {
        McpManager {
            servers: Arc::new(RwLock::new(vec![])),
            lifecycle: Arc::new(Mutex::new(())),
            recovery_tasks: std::sync::Mutex::new(Vec::new()),
            catalog_changed: watch::channel(0).0,
            schemas_dirty: Arc::new(AtomicBool::new(false)),
            remote_client: Some(remote),
            diagnostics: Arc::new(RwLock::new(HashMap::new())),
        }
    }

    /// No-op (empty) manager.
    pub fn empty() -> Self {
        McpManager {
            servers: Arc::new(RwLock::new(vec![])),
            lifecycle: Arc::new(Mutex::new(())),
            recovery_tasks: std::sync::Mutex::new(Vec::new()),
            catalog_changed: watch::channel(0).0,
            schemas_dirty: Arc::new(AtomicBool::new(false)),
            remote_client: None,
            diagnostics: Arc::new(RwLock::new(HashMap::new())),
        }
    }

    /// Merge servers from a completed background boot into this manager.
    pub async fn merge_from(&self, other: McpManager) {
        let _lifecycle = self.lifecycle.lock().await;
        let new_servers: Vec<_> = other.servers.write().await.drain(..).collect();
        let mut current = self.servers.write().await;
        let new_keys: HashSet<_> = new_servers.iter().map(|s| s.key.clone()).collect();
        let mut retired = Vec::new();
        let mut kept = Vec::new();
        for server in current.drain(..) {
            if new_keys.contains(&server.key) {
                retired.push(server);
            } else {
                kept.push(server);
            }
        }
        *current = kept;
        current.extend(new_servers);
        drop(current);
        for server in retired {
            server.shutdown().await;
        }

        let new_diags = other.diagnostics.read().await.clone();
        let mut cur_diags = self.diagnostics.write().await;
        cur_diags.extend(new_diags);

        self.publish_catalog_change();
    }

    /// Each subscriber observes all lifecycle changes independently of the legacy
    /// CLI dirty flag. Slow subscribers coalesce changes and read the latest catalog.
    pub fn subscribe_catalog_changes(&self) -> watch::Receiver<u64> {
        self.catalog_changed.subscribe()
    }

    /// Durable mirrors wait for the current lifecycle mutation to settle, so a
    /// temporary withdrawal during reload cannot delete stable tool identities.
    /// Model context uses active_catalog directly and sees withdrawal immediately.
    pub async fn settled_catalog(
        &self,
    ) -> Vec<cade_core::capabilities::mesh::TaggedCapabilitySchema> {
        use cade_core::capabilities::mesh::{CapabilityExecutionContext, CapabilityMesh};
        let _lifecycle = self.lifecycle.lock().await;
        self.active_catalog(&CapabilityExecutionContext::new("catalog-mirror"))
            .await
    }

    fn publish_catalog_change(&self) {
        self.schemas_dirty.store(true, Ordering::SeqCst);
        self.catalog_changed
            .send_modify(|revision| *revision = revision.wrapping_add(1));
    }

    /// Dynamically start and add a single MCP server on-demand.
    pub async fn start_and_add_server(&self, key: &str, config: &McpServerConfig) -> Result<()> {
        let _lifecycle = self.lifecycle.lock().await;
        let old = {
            let mut servers = self.servers.write().await;
            self.diagnostics.write().await.insert(
                key.to_string(),
                McpDiagnostic {
                    status: if config.disabled {
                        "disabled"
                    } else {
                        "starting"
                    }
                    .into(),
                    command: server_command_display(config),
                    error: None,
                },
            );
            servers
                .iter()
                .position(|s| s.key == key)
                .map(|index| servers.remove(index))
        };
        self.publish_catalog_change();
        if let Some(old) = old {
            old.shutdown().await;
        }
        if config.disabled {
            self.diagnostics.write().await.remove(key);
            return Ok(());
        }
        let server = match Self::connect_server(key, config).await {
            Ok(server) => {
                let mut diags = self.diagnostics.write().await;
                diags.insert(
                    key.to_string(),
                    McpDiagnostic {
                        status: "ready".into(),
                        command: server.command.clone(),
                        error: None,
                    },
                );
                server
            }
            Err(e) => {
                let mut diags = self.diagnostics.write().await;
                diags.insert(
                    key.to_string(),
                    McpDiagnostic {
                        status: "failed".into(),
                        command: server_command_display(config),
                        error: Some(e.to_string()),
                    },
                );
                return Err(e);
            }
        };
        let mut servers = self.servers.write().await;
        servers.push(server);
        self.publish_catalog_change();
        Ok(())
    }

    /// Reload MCP servers from a new config map.
    pub async fn reload(
        &self,
        new_configs: &HashMap<String, McpServerConfig>,
        mut on_progress: Option<&mut (dyn FnMut(McpStartResult) + Send)>,
    ) -> ReloadSummary {
        let _lifecycle = self.lifecycle.lock().await;
        let mut summary = ReloadSummary::default();

        let mut entries: Vec<(&String, &McpServerConfig)> = new_configs
            .iter()
            .filter(|(_, cfg)| !cfg.disabled)
            .collect();
        entries.sort_by_key(|(k, _)| k.as_str());

        let timeout_dur = std::time::Duration::from_secs(MCP_SERVER_TIMEOUT_SECS);

        let mut to_restart = Vec::new();
        let mut preserved = Vec::new();
        let mut retired = Vec::new();

        {
            let mut current = self.servers.write().await;
            for (key, cfg) in &entries {
                let existing = current.iter().find(|s| &s.key == *key);

                if existing.is_some_and(|s| s.is_ready() && same_configuration(&s.config, cfg)) {
                    preserved.push((*key).clone());
                    if let Some(ref mut cb) = on_progress {
                        cb(McpStartResult::Ok {
                            key: (*key).clone(),
                            tool_count: existing.map(|s| s.tools.len()).unwrap_or_default(),
                        });
                    }
                } else {
                    to_restart.push(((*key).clone(), (*cfg).clone()));
                }
            }

            let mut kept_servers = Vec::new();
            {
                let mut diags = self.diagnostics.write().await;
                diags.retain(|key, _| new_configs.get(key).is_some_and(|cfg| !cfg.disabled));
                for (key, config) in &to_restart {
                    diags.insert(
                        key.clone(),
                        McpDiagnostic {
                            status: "starting".into(),
                            command: server_command_display(config),
                            error: None,
                        },
                    );
                }
            }

            for srv in current.drain(..) {
                if preserved.contains(&srv.key) {
                    summary.kept.push(srv.key.clone());
                    kept_servers.push(srv);
                } else {
                    summary.stopped.push(srv.key.clone());
                    retired.push(srv);
                }
            }
            *current = kept_servers;
        }
        self.publish_catalog_change();
        for server in retired {
            server.shutdown().await;
        }

        let mut join_set = tokio::task::JoinSet::new();
        for (key, config) in to_restart {
            let k = key.clone();
            let c = config.clone();
            join_set.spawn(async move {
                let res = tokio::time::timeout(timeout_dur, Self::connect_server(&k, &c)).await;
                (k, c, res)
            });
        }

        while let Some(Ok((key, config, result))) = join_set.join_next().await {
            let cmd_display = server_command_display(&config);
            match result {
                Ok(Ok(new_server)) => {
                    let count = new_server.tools.len();
                    info!("MCP server '{key}' (re)started — {count} tool(s)");
                    summary.started.push(key.clone());
                    let mut diags = self.diagnostics.write().await;
                    diags.insert(
                        key.clone(),
                        McpDiagnostic {
                            status: "ready".into(),
                            command: new_server.command.clone(),
                            error: None,
                        },
                    );
                    drop(diags);
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
                    let mut diags = self.diagnostics.write().await;
                    diags.insert(
                        key.clone(),
                        McpDiagnostic {
                            status: "failed".into(),
                            command: cmd_display,
                            error: Some(msg.clone()),
                        },
                    );
                    drop(diags);
                    if let Some(ref mut cb) = on_progress {
                        cb(McpStartResult::Failed { key, error: msg });
                    }
                }
                Err(_) => {
                    warn!(
                        "MCP server '{key}' timed out during reload ({MCP_SERVER_TIMEOUT_SECS}s)"
                    );
                    summary.failed.push(key.clone());
                    let mut diags = self.diagnostics.write().await;
                    diags.insert(
                        key.clone(),
                        McpDiagnostic {
                            status: "timeout".into(),
                            command: cmd_display,
                            error: Some(format!("Timed out after {MCP_SERVER_TIMEOUT_SECS}s")),
                        },
                    );
                    drop(diags);
                    if let Some(ref mut cb) = on_progress {
                        cb(McpStartResult::Timeout {
                            key,
                            timeout_secs: MCP_SERVER_TIMEOUT_SECS,
                        });
                    }
                }
            }
        }

        self.publish_catalog_change();
        summary
    }

    /// Returns true if no servers are configured or connected.
    pub async fn is_empty(&self) -> bool {
        self.servers.read().await.is_empty() && self.remote_client.is_none()
    }

    /// Return all cached tool schemas across all servers in OpenAI Value format.
    pub async fn all_tool_schemas(&self) -> Vec<Value> {
        self.all_typed_tool_schemas()
            .await
            .into_iter()
            .map(|tool| tool.schema)
            .collect()
    }

    /// Return all cached typed tool schemas across all servers.
    pub async fn all_typed_tool_schemas(&self) -> Vec<McpToolSchema> {
        let servers = self.servers.read().await;
        let mut tools: Vec<_> = servers
            .iter()
            .filter(|s| s.is_ready())
            .flat_map(|s| s.tools.clone())
            .collect();
        tools.sort_by(|left, right| left.prefixed_name.cmp(&right.prefixed_name));
        tools
    }

    /// Return all cached tool schemas for a specific server.
    pub async fn schemas_for_server(&self, server_key: &str) -> Vec<McpToolSchema> {
        let servers = self.servers.read().await;
        servers
            .iter()
            .find(|s| s.key == server_key && s.is_ready())
            .map(|s| s.tools.clone())
            .unwrap_or_default()
    }

    /// Return a public status summary for every managed server, including diagnostic health and errors.
    pub async fn status(&self) -> Vec<McpStatus> {
        let servers = self.servers.read().await;
        let diagnostics = self.diagnostics.read().await;
        let mut list = Vec::new();
        let mut seen = std::collections::HashSet::new();

        for s in servers.iter() {
            seen.insert(s.key.clone());
            list.push(McpStatus {
                generation: s.is_ready().then(|| s.generation.as_ref().clone()),
                key: s.key.clone(),
                command: s.command.clone(),
                tools: s
                    .tools
                    .iter()
                    .filter(|_| s.is_ready())
                    .map(|t| t.prefixed_name.clone())
                    .collect(),
                tool_mutability: s
                    .tools
                    .iter()
                    .filter(|_| s.is_ready())
                    .map(|t| (t.prefixed_name.clone(), t.is_write))
                    .collect(),
                disabled: false,
                status: if !s.is_ready() {
                    diagnostics
                        .get(&s.key)
                        .filter(|d| d.status != "ready")
                        .map(|d| d.status.clone())
                        .unwrap_or_else(|| "disconnected".into())
                } else {
                    "ready".to_string()
                },
                error: diagnostics.get(&s.key).and_then(|d| d.error.clone()),
            });
        }

        for (k, diag) in diagnostics.iter() {
            if !seen.contains(k) {
                list.push(McpStatus {
                    generation: None,
                    key: k.clone(),
                    command: diag.command.clone(),
                    tools: vec![],
                    tool_mutability: HashMap::new(),
                    disabled: false,
                    status: diag.status.clone(),
                    error: diag.error.clone(),
                });
            }
        }

        if !list.is_empty() {
            return list;
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
        self.ready_tool_mutability(prefixed_name).await.is_some()
    }

    /// Check mutability using connected metadata or the server-hosted catalog.
    /// Unknown external tools fail closed: they are treated as writes until metadata is available.
    pub async fn is_write_tool(&self, prefixed_name: &str) -> bool {
        if let Some(is_write) = self.ready_tool_mutability(prefixed_name).await {
            return is_write;
        }
        if let Some(remote) = &self.remote_client
            && let Ok(statuses) = remote.list_mcp_statuses().await
            && let Some(is_write) = statuses
                .iter()
                .filter(|s| !s.disabled && s.status == "ready")
                .find_map(|s| s.tool_mutability.get(prefixed_name))
        {
            return *is_write;
        }
        true
    }

    async fn ready_tool_mutability(&self, prefixed_name: &str) -> Option<bool> {
        let servers = self.servers.read().await;
        for server in servers.iter() {
            if !server.is_ready() {
                continue;
            }
            if let Some(tool) = server
                .tools
                .iter()
                .find(|t| t.prefixed_name == prefixed_name)
            {
                return Some(tool.is_write);
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
        self.call_tool_inner(prefixed_name, args, None).await
    }

    /// Snapshot identity and mutability together, before permission evaluation.
    pub async fn tool_binding(&self, name: &str) -> Option<(String, bool)> {
        let known = {
            let servers = self.servers.read().await;
            servers.iter().find_map(|server| {
                let tool = server
                    .tools
                    .iter()
                    .find(|tool| tool.prefixed_name == name)?;
                Some((
                    server.key.clone(),
                    server.generation.clone(),
                    server.is_ready(),
                    tool.is_write,
                ))
            })
        };
        if let Some((key, generation, ready, is_write)) = known {
            if ready {
                return Some((generation.as_ref().clone(), is_write));
            }
            // Idle disconnects never reach invoke(), so binding resolution
            // must initiate recovery too. The owned worker only reconnects;
            // it has no intent to replay if this authorization caller drops.
            let _ = self
                .recover(
                    key.clone(),
                    generation,
                    name.to_owned(),
                    "Transport closed before authorization".into(),
                )
                .await;
            // Another recovery or reload may have won the lifecycle lock.
            // Resolve the current ready implementation afresh in either case;
            // authorization has not happened yet. Never return stale policy.
            let servers = self.servers.read().await;
            let server = servers
                .iter()
                .find(|server| server.key == key && server.is_ready())?;
            if let Some(tool) = server.tools.iter().find(|tool| tool.prefixed_name == name) {
                return Some((server.generation.as_ref().clone(), tool.is_write));
            }
            return None;
        }
        if let Some(remote) = &self.remote_client {
            for status in remote.list_mcp_statuses().await.ok()? {
                if !status.disabled
                    && status.status == "ready"
                    && status.tools.iter().any(|tool| tool == name)
                {
                    return status.generation.map(|generation| {
                        (
                            generation,
                            status.tool_mutability.get(name).copied().unwrap_or(true),
                        )
                    });
                }
            }
        }
        None
    }

    pub async fn call_tool_bound(
        &self,
        name: &str,
        args: &Value,
        generation: &str,
    ) -> Option<Result<(String, bool, Option<String>)>> {
        self.call_tool_inner(name, args, Some(generation)).await
    }

    async fn call_tool_inner(
        &self,
        prefixed_name: &str,
        args: &Value,
        expected: Option<&str>,
    ) -> Option<Result<(String, bool, Option<String>)>> {
        // Capture identity and generation under one read lock. Never retain a
        // vector index across an await: reload can remove/reorder servers.
        let target = {
            let servers = self.servers.read().await;
            servers.iter().find_map(|server| {
                server
                    .tools
                    .iter()
                    .find(|tool| tool.prefixed_name == prefixed_name)
                    .map(|tool| {
                        (
                            server.key.clone(),
                            server.generation.clone(),
                            tool.original_name.clone(),
                            server.peer.clone(),
                            tool.is_write,
                        )
                    })
            })
        };
        let Some((key, generation, original, peer, was_write)) = target else {
            return if let Some(remote) = &self.remote_client {
                Some(match expected {
                    Some(generation) => {
                        remote
                            .call_mcp_tool_bound(prefixed_name, args, generation)
                            .await
                    }
                    None => remote.call_mcp_tool(prefixed_name, args).await,
                })
            } else {
                None
            };
        };
        // The checked peer is the peer invoked below. Reload cannot substitute
        // another implementation between this check and the transport request.
        if expected.is_some_and(|expected| expected != generation.as_str()) {
            return Some(Err(Error::custom(
                "MCP implementation changed; fresh authorization required",
            )));
        }
        let error_msg = match Self::invoke(&peer, &original, args).await {
            Ok(result) => return Some(Ok(result)),
            Err(error) => error.to_string(),
        };
        if Self::is_rpc_protocol_error(&error_msg) {
            return Some(Err(Error::custom(error_msg)));
        }

        // Spawn before the next suspension point. Recovery belongs to the manager,
        // not this Run; the worker never receives arguments and cannot replay work.
        let recovery = self.recover(key, generation, prefixed_name.to_owned(), error_msg);
        let recovered = recovery
            .await
            .map_err(|error| Error::custom(format!("MCP recovery stopped: {error}")))
            .and_then(|result| result);
        if was_write {
            return Some(Err(Error::custom(format!(
                "MCP tool '{prefixed_name}' outcome is uncertain: the transport closed after dispatch. The operation may have completed; it was not replayed. Verify its effects before retrying.{}",
                recovered
                    .err()
                    .map(|error| format!(" Recovery failed: {error}"))
                    .unwrap_or_default()
            ))));
        }
        if expected.is_some() {
            return Some(Err(Error::custom(
                "MCP connection changed during recovery; fresh authorization required",
            )));
        }
        let (peer, tool) = match recovered {
            Ok(result) => result,
            Err(error) => return Some(Err(error)),
        };
        let result = match tool {
            Some(tool) if !tool.is_write => Self::invoke(&peer, &tool.original_name, args).await,
            Some(_) => Err(Error::custom(format!(
                "MCP tool '{prefixed_name}' permissions changed after reconnect; retry for authorization"
            ))),
            None => Err(Error::custom(format!(
                "Tool '{prefixed_name}' no longer exposed after reconnect"
            ))),
        };
        if peer.is_transport_closed() {
            self.publish_catalog_change();
        }
        Some(result)
    }

    fn recover(
        &self,
        key: String,
        generation: Arc<String>,
        name: String,
        error: String,
    ) -> tokio::task::JoinHandle<Result<(rmcp::Peer<RoleClient>, Option<McpToolSchema>)>> {
        let servers = self.servers.clone();
        let lifecycle = self.lifecycle.clone();
        let diagnostics = self.diagnostics.clone();
        let dirty = self.schemas_dirty.clone();
        let changed = self.catalog_changed.clone();
        let task = tokio::spawn(async move {
            let publish = || {
                dirty.store(true, Ordering::SeqCst);
                changed.send_modify(|revision| *revision = revision.wrapping_add(1));
            };
            let _lifecycle = lifecycle.lock().await;
            let old = {
                let mut servers = servers.write().await;
                let Some(server) = servers.iter_mut().find(|server| {
                    server.key == key && Arc::ptr_eq(&server.generation, &generation)
                }) else {
                    return Err(Error::custom(format!(
                        "MCP server '{key}' changed during execution; reauthorization required"
                    )));
                };
                // Retain configuration and identity in the owner even if recovery
                // fails. The disconnected slot cannot publish executable schemas.
                let disconnected = server.disconnected_snapshot();
                let old = std::mem::replace(server, disconnected);
                diagnostics.write().await.insert(
                    key.clone(),
                    McpDiagnostic {
                        status: "reconnecting".into(),
                        command: old.command.clone(),
                        error: Some(error.clone()),
                    },
                );
                old
            };
            let config = old.config.clone();
            publish();
            old.shutdown().await;
            let mut last_error = error;
            for attempt in 1..=MAX_RECONNECT_ATTEMPTS {
                tokio::time::sleep(std::time::Duration::from_secs(RECONNECT_DELAY_SECS)).await;
                match Self::connect_server(&key, &config).await {
                    Ok(server) => {
                        let tool = server
                            .tools
                            .iter()
                            .find(|tool| tool.prefixed_name == name)
                            .cloned();
                        let peer = server.peer.clone();
                        let mut servers = servers.write().await;
                        servers.retain(|old| old.key != key);
                        diagnostics.write().await.insert(
                            key.clone(),
                            McpDiagnostic {
                                status: "ready".into(),
                                command: server.command.clone(),
                                error: None,
                            },
                        );
                        servers.push(server);
                        publish();
                        return Ok((peer, tool));
                    }
                    Err(error) => {
                        last_error = error.to_string();
                        warn!("Reconnect attempt {attempt} for '{key}' failed: {last_error}");
                    }
                }
            }
            error!("MCP server '{key}' unavailable after {MAX_RECONNECT_ATTEMPTS} reconnects");
            diagnostics.write().await.insert(
                key.clone(),
                McpDiagnostic {
                    status: "failed".into(),
                    command: server_command_display(&config),
                    error: Some(last_error.clone()),
                },
            );
            publish();
            Err(Error::custom(format!(
                "MCP server '{key}' disconnected after {MAX_RECONNECT_ATTEMPTS} reconnect attempts: {last_error}"
            )))
        });
        let mut tasks = self
            .recovery_tasks
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        tasks.retain(|task| !task.is_finished());
        tasks.push(task.abort_handle());
        task
    }

    async fn invoke(
        peer: &rmcp::Peer<RoleClient>,
        original: &str,
        args: &Value,
    ) -> Result<(String, bool, Option<String>)> {
        let result = peer
            .call_tool(
                CallToolRequestParams::new(original.to_string())
                    .with_arguments(args.as_object().cloned().unwrap_or_default()),
            )
            .await
            .map_err(|error| Error::custom(error.to_string()))?;
        let uri = result.meta.as_ref().and_then(|meta| {
            serde_json::to_value(meta).ok().and_then(|value| {
                value
                    .get("ui")
                    .and_then(|ui| ui.get("resourceUri"))
                    .and_then(|uri| uri.as_str().map(String::from))
            })
        });
        Ok((
            extract_content_text(&result.content),
            result.is_error.unwrap_or(false),
            uri,
        ))
    }

    async fn connect_server(key: &str, config: &McpServerConfig) -> Result<McpServer> {
        tokio::time::timeout(
            std::time::Duration::from_secs(MCP_SERVER_TIMEOUT_SECS),
            Self::connect_server_inner(key, config),
        )
        .await
        .map_err(|_| Error::custom(format!("MCP server '{key}' connection timed out")))?
    }

    async fn connect_server_inner(key: &str, config: &McpServerConfig) -> Result<McpServer> {
        if let Some(url) = &config.url {
            let (service, peer) = HttpTransportAdapter::connect(key, config, url).await?;
            Self::build_server_from_peer(key, config, peer, service, format!("[http] {url}"), None)
                .await
        } else {
            let (service, peer, singleton_guard) =
                StdioTransportAdapter::connect(key, config).await?;
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
            .map(|tool| {
                ToolSchemaNormalizer::normalize(key, &tool, &config.write_tools, config.core_server)
            })
            .collect();

        Ok(McpServer {
            generation: Arc::new({
                static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
                format!(
                    "{}-{}-{}",
                    std::process::id(),
                    chrono::Utc::now().timestamp_nanos_opt().unwrap_or_default(),
                    NEXT.fetch_add(1, Ordering::Relaxed)
                )
            }),
            key: key.to_string(),
            command: command_display,
            tools,
            config: config.clone(),
            _service: Some(service),
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
                    Err(
                        cade_core::capabilities::mesh::ExecutionError::ExecutionFailed(
                            intent.capability_name,
                            err_str,
                        ),
                    )
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
                if t.schema
                    .get("x-cade")
                    .and_then(|metadata| metadata.get("core_server"))
                    .and_then(serde_json::Value::as_bool)
                    .unwrap_or(false)
                {
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

/// Deep module for extracting and normalizing textual content from arbitrary MCP tool results.
///
/// Interface:
/// - Takes a slice of `Annotated<RawContent>`
/// - Returns a clean, concatenated `String`
///
/// Implementation (Depth):
/// - Extracts standard `RawContent::Text(t)`.
/// - Extracts embedded text resources `RawContent::Resource(r)` (`TextResourceContents`).
/// - Extracts resource links `RawContent::ResourceLink(l)` (`uri` / `name`).
/// - Strips sampling fallback error headers (e.g. `[Sampling fell back to raw results...]`).
pub fn extract_content_text(content: &[rmcp::model::Annotated<RawContent>]) -> String {
    let mut parts = Vec::new();
    for c in content {
        match &c.raw {
            RawContent::Text(t) => {
                parts.push(clean_mcp_text(&t.text));
            }
            RawContent::Resource(r) => match &r.resource {
                rmcp::model::ResourceContents::TextResourceContents { text, .. } => {
                    parts.push(clean_mcp_text(text));
                }
                rmcp::model::ResourceContents::BlobResourceContents { uri, mime_type, .. } => {
                    parts.push(format!(
                        "[Binary Resource: {} ({})]",
                        uri,
                        mime_type.as_deref().unwrap_or("application/octet-stream")
                    ));
                }
            },
            RawContent::ResourceLink(l) => {
                parts.push(format!("[Resource Link: {} ({})]", l.name, l.uri));
            }
            RawContent::Image(img) => {
                parts.push(format!("[Image: {}]", img.mime_type));
            }
            RawContent::Audio(aud) => {
                parts.push(format!("[Audio: {}]", aud.mime_type));
            }
        }
    }
    parts.join("\n")
}

/// Clean and normalize text from MCP tool results, stripping diagnostic fallback wrappers.
pub fn clean_mcp_text(text: &str) -> String {
    let trimmed = text.trim();
    if trimmed.starts_with("[Sampling fell back to raw results") {
        if let Some(pos) = trimmed.find("]\n\n") {
            return trimmed[pos + 3..].trim().to_string();
        } else if let Some(pos) = trimmed.find("]\n") {
            return trimmed[pos + 2..].trim().to_string();
        }
    }
    trimmed.to_string()
}

// endregion: --- Content Extraction

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_mcp_manager_status_includes_failed_and_timeout_diagnostics() {
        let mgr = McpManager::empty();
        {
            let mut diags = mgr.diagnostics.write().await;
            diags.insert(
                "broken_server".into(),
                McpDiagnostic {
                    status: "failed".into(),
                    command: "node broken.js".into(),
                    error: Some("connection refused".into()),
                },
            );
            diags.insert(
                "slow_server".into(),
                McpDiagnostic {
                    status: "timeout".into(),
                    command: "sleep 20".into(),
                    error: Some("Timed out after 10s".into()),
                },
            );
        }

        let statuses = mgr.status().await;
        assert_eq!(statuses.len(), 2);

        let broken = statuses.iter().find(|s| s.key == "broken_server").unwrap();
        assert_eq!(broken.status, "failed");
        assert_eq!(broken.command, "node broken.js");
        assert_eq!(broken.error.as_deref(), Some("connection refused"));

        let slow = statuses.iter().find(|s| s.key == "slow_server").unwrap();
        assert_eq!(slow.status, "timeout");
        assert_eq!(slow.command, "sleep 20");
        assert_eq!(slow.error.as_deref(), Some("Timed out after 10s"));
    }

    #[test]
    fn test_extract_content_text_with_resources_and_sampling_cleanup() {
        // -- Setup & Fixtures
        use rmcp::model::{
            Annotated, RawContent, RawEmbeddedResource, RawResource, ResourceContents,
        };

        let items = vec![
            Annotated {
                raw: RawContent::text(
                    "[Sampling fell back to raw results due to error: MethodNotFound]\n\nResult row 1\nResult row 2",
                ),
                annotations: None,
            },
            Annotated {
                raw: RawContent::Resource(RawEmbeddedResource {
                    meta: None,
                    resource: ResourceContents::text(
                        "Attached document contents",
                        "file:///doc.txt",
                    ),
                }),
                annotations: None,
            },
            Annotated {
                raw: RawContent::ResourceLink(RawResource {
                    uri: "https://example.com/api".to_string(),
                    name: "API Spec".to_string(),
                    description: None,
                    mime_type: None,
                    meta: None,
                    size: None,
                    icons: None,
                    title: None,
                }),
                annotations: None,
            },
        ];

        // -- Exec
        let text = extract_content_text(&items);

        // -- Check
        assert!(
            !text.contains("Sampling fell back"),
            "Sampling error headers must be stripped"
        );
        assert!(text.contains("Result row 1\nResult row 2"));
        assert!(text.contains("Attached document contents"));
        assert!(text.contains("[Resource Link: API Spec (https://example.com/api)]"));
    }
}
