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

// region:    --- Modules

mod error;

pub use error::{Error, Result};

pub mod watcher;
use rmcp::{
    RoleClient, ServiceExt,
    model::{CallToolRequestParams, RawContent},
    service::RunningService,
    transport::TokioChildProcess,
};
use serde_json::{Value, json};
use std::collections::HashMap;
use tokio::process::Command;
use tokio::sync::RwLock;
use tracing::{error, info, warn};

use cade_core::settings::McpServerConfig;

// endregion: --- Modules

// -- Reconnect constants

const MAX_RECONNECT_ATTEMPTS: u32 = 3;
const RECONNECT_DELAY_SECS: u64 = 2;
/// Maximum time (in seconds) to wait for a single MCP server to spawn,
/// complete the JSON-RPC handshake, and report its tool list.
/// Servers exceeding this deadline are skipped with a warning.
const MCP_SERVER_TIMEOUT_SECS: u64 = 15;

// -- Types

/// Result of a single MCP server startup attempt — used by the progress
/// reporter in `main.rs` to show per-server status.
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

/// Public summary of a running MCP server (for `/mcp` command display).
#[derive(Debug, Clone, serde::Serialize)]
pub struct McpStatus {
    pub key: String,
    pub command: String,
    pub tools: Vec<String>, // prefixed names
    pub disabled: bool,
}

/// A cached tool schema in OpenAI-compatible format.
#[derive(Debug, Clone)]
pub struct McpToolSchema {
    pub server_key: String,
    pub prefixed_name: String,
    pub original_name: String,
    pub schema: Value, // OpenAI-compatible: { name, description, parameters }
    /// If true, calling this tool requires user permission.
    pub is_write: bool,
}

// -- McpServer

struct McpServer {
    key: String,
    command: String,
    tools: Vec<McpToolSchema>,
    /// Original config — needed to reconnect the child process.
    config: McpServerConfig,
    /// Consecutive failed reconnect attempts since last success.
    reconnect_attempts: u32,
    /// If true, all reconnect attempts have been exhausted; calls fail immediately.
    disabled: bool,
    /// The live peer — kept alive as long as this struct exists.
    _service: RunningService<RoleClient, ()>,
    peer: rmcp::Peer<RoleClient>,
}

// -- McpManager

/// Manages all active MCP server connections.
///
/// Constructed once at startup; passed as `Arc<McpManager>` to the REPL.
/// All methods take `&self` (thread-safe via interior `RwLock`).
pub struct McpManager {
    /// Interior-mutable server list so `call_tool(&self)` can reconnect.
    servers: RwLock<Vec<McpServer>>,
    /// Set to `true` when tool schemas change after a successful reconnect.
    /// The REPL polls this flag each tick and re-registers tools when set.
    pub schemas_dirty: std::sync::Arc<std::sync::atomic::AtomicBool>,
}

/// Summary returned by `McpManager::reload()` for display in the REPL.
#[derive(Debug, Default)]
pub struct ReloadSummary {
    pub started: Vec<String>,
    pub stopped: Vec<String>,
    pub kept: Vec<String>,
    pub failed: Vec<String>,
}

/// Return the connection identity string of an existing server, if present.
///
/// For remote (HTTP) servers the identity is the URL extracted from the
/// "[http] <url>" display string stored in `McpServer::command`.
/// For stdio servers the identity is the command binary path.
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
    /// Servers that fail to start or exceed the per-server timeout are skipped
    /// with a warning.
    ///
    /// Returns `(manager, results)` where `results` contains per-server
    /// outcomes so the caller can report individual status to the user.
    pub async fn start(
        configs: &HashMap<String, McpServerConfig>,
        mut on_progress: Option<&mut (dyn FnMut(McpStartResult) + Send)>,
    ) -> (Self, Vec<McpStartResult>) {
        let mut servers = Vec::new();
        let mut results = Vec::new();

        // Sort for deterministic startup order
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
            schemas_dirty: std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false)),
        };
        (mgr, results)
    }

    /// No-op (empty) manager — used when no servers are configured.
    pub fn empty() -> Self {
        McpManager {
            servers: RwLock::new(vec![]),
            schemas_dirty: std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false)),
        }
    }

    /// Merge servers from a completed background boot into this (initially empty)
    /// manager.  Used by the deferred-startup path: the REPL starts with an
    /// empty manager, and the background task calls this once all servers are
    /// ready.
    pub async fn merge_from(&self, other: McpManager) {
        let new_servers = other.servers.into_inner();
        let mut current = self.servers.write().await;
        current.extend(new_servers);
        self.schemas_dirty
            .store(true, std::sync::atomic::Ordering::SeqCst);
    }

    /// Reload MCP servers from a new config map.
    ///
    /// - Servers whose key **and** command are unchanged are kept as-is.
    /// - Servers no longer in `new_configs` are dropped (killing the child process).
    /// - New or changed servers are (re-)started with a per-server timeout.
    ///
    /// Returns a `ReloadSummary` suitable for display in the REPL.
    pub async fn reload(&self, new_configs: &HashMap<String, McpServerConfig>) -> ReloadSummary {
        let mut summary = ReloadSummary::default();

        // Sort new configs for deterministic startup order
        let mut entries: Vec<(&String, &McpServerConfig)> = new_configs.iter().collect();
        entries.sort_by_key(|(k, _)| k.as_str());

        let timeout_dur = std::time::Duration::from_secs(MCP_SERVER_TIMEOUT_SECS);

        // Drain the current server list — we'll rebuild it
        let mut old_servers: Vec<McpServer> = {
            let mut servers = self.servers.write().await;
            std::mem::take(&mut *servers)
        };

        // Index old servers by key for O(1) lookup
        let mut old_by_key: HashMap<String, McpServer> =
            old_servers.drain(..).map(|s| (s.key.clone(), s)).collect();

        let mut new_servers: Vec<McpServer> = Vec::new();
        let mut join_set = tokio::task::JoinSet::new();

        for (key, config) in &entries {
            // Keep existing connection if the server identity is unchanged and
            // the server is healthy.
            //
            // Identity for stdio servers  = the `command` binary path.
            // Identity for remote servers = the `url` field (stored in config).
            // `existing.command` holds "[http] <url>" for remote servers, so we
            // compare against `config.url` directly instead of `config.command`.
            let identity_unchanged = if let Some(url) = &config.url {
                existing_identity(&old_by_key.get(*key)) == Some(url.as_str())
            } else {
                old_by_key
                    .get(*key)
                    .map(|e| e.command == config.command)
                    .unwrap_or(false)
            };

            if let Some(existing) = old_by_key.get(*key)
                && identity_unchanged
                && !existing.disabled
            {
                // The `if let Some(...)` immediately above proved `*key`
                // is in the map.  Use unwrap_or_else for belt-and-suspenders
                // safety under panic=unwind — log rather than crash.
                let Some(existing) = old_by_key.remove(*key) else {
                    tracing::error!(
                        "BUG: key '{key}' vanished from old_by_key between get and remove"
                    );
                    continue;
                };
                summary.kept.push(key.to_string());
                new_servers.push(existing);
                continue;
            }
            // Identity changed or server was disabled — drop and reconnect.

            // Start a new connection with per-server timeout
            let k = (*key).clone();
            let c = (*config).clone();
            join_set.spawn(async move {
                let res = tokio::time::timeout(timeout_dur, Self::connect_server(&k, &c)).await;
                (k, res)
            });
        }

        while let Some(Ok((key, result))) = join_set.join_next().await {
            match result {
                Ok(Ok(server)) => {
                    info!(
                        "MCP reload: started server '{key}' — {} tool(s)",
                        server.tools.len()
                    );
                    summary.started.push(key.to_string());
                    new_servers.push(server);
                }
                Ok(Err(e)) => {
                    warn!("MCP reload: server '{key}' failed to start: {e}");
                    summary.failed.push(key.to_string());
                }
                Err(_elapsed) => {
                    warn!(
                        "MCP reload: server '{key}' timed out after {}s — skipping",
                        MCP_SERVER_TIMEOUT_SECS
                    );
                    summary.failed.push(key.to_string());
                }
            }
        }

        // Servers remaining in old_by_key were not in new_configs — they are stopped
        for key in old_by_key.keys() {
            info!("MCP reload: stopping server '{key}'");
            summary.stopped.push(key.clone());
        }
        // Dropping old_by_key drops the McpServer values, killing child processes

        // Install rebuilt server list
        *self.servers.write().await = new_servers;

        summary
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
    ) -> Option<Result<(String, bool, Option<String>)>> {
        let server_idx = self.find_tool_idx(prefixed_name).await?.0;

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
                "MCP server '{}' is disabled after {} failed reconnect attempts",
                server_key, MAX_RECONNECT_ATTEMPTS
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
            return Some(Err(Error::custom(error_msg.to_string())));
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

            let old_tool_names: std::collections::HashSet<String> = {
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
                    info!("MCP server '{}' reconnected successfully", key);

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
                    } else {
                        let mut servers = self.servers.write().await;
                        servers[server_idx] = new_server;
                        self.schemas_dirty
                            .store(true, std::sync::atomic::Ordering::SeqCst);
                        return Some(Err(Error::custom(format!(
                            "Tool '{prefixed_name}' not found after MCP server reconnect",
                        ))));
                    };

                    {
                        let mut servers = self.servers.write().await;
                        servers[server_idx] = new_server;
                        let new_tool_names: std::collections::HashSet<String> = servers[server_idx]
                            .tools
                            .iter()
                            .map(|t| t.prefixed_name.clone())
                            .collect();
                        if old_tool_names != new_tool_names {
                            warn!(
                                "MCP server '{}' tool schemas changed after reconnect — scheduling re-registration",
                                key
                            );
                            self.schemas_dirty
                                .store(true, std::sync::atomic::Ordering::SeqCst);
                        }
                    }

                    return Some(match call_result {
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
                            Ok((text, is_error, ui_resource_uri))
                        }
                        Err(e) => Err(Error::custom(format!(
                            "MCP call failed after reconnect: {e}"
                        ))),
                    });
                }
                Err(e) => {
                    warn!(
                        "MCP reconnect attempt {attempt}/{MAX_RECONNECT_ATTEMPTS} failed for '{}': {e}",
                        key
                    );
                    let mut servers = self.servers.write().await;
                    servers[server_idx].reconnect_attempts += 1;
                }
            }
        }

        {
            let mut servers = self.servers.write().await;
            let s = &mut servers[server_idx];
            s.disabled = true;
            error!(
                "MCP server '{}' disabled after {MAX_RECONNECT_ATTEMPTS} failed reconnect attempts",
                s.key
            );
        }

        Some(Err(Error::custom(format!(
            "MCP server disabled: all {MAX_RECONNECT_ATTEMPTS} reconnect attempts failed \
             (original error: {error_msg})",
        ))))
    }

    /// Whether a tool requires user permission (mutable tools).
    pub async fn is_write_tool(&self, prefixed_name: &str) -> bool {
        self.find_tool_schema(prefixed_name)
            .await
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
                key: s.key.clone(),
                command: s.command.clone(),
                tools: s.tools.iter().map(|t| t.prefixed_name.clone()).collect(),
                disabled: s.disabled,
            })
            .collect()
    }

    // -- Internal helpers

    async fn connect_server(key: &str, config: &McpServerConfig) -> Result<McpServer> {
        // -- Transport selection: URL → HTTP (SSE or Streamable), else → stdio child process
        if let Some(url) = &config.url {
            Self::connect_server_http(key, config, url).await
        } else {
            Self::connect_server_stdio(key, config).await
        }
    }

    /// Connect via HTTP+SSE or Streamable HTTP (remote servers).
    ///
    /// Uses rmcp's unified `StreamableHttpClientTransport` which auto-detects
    /// SSE vs Streamable HTTP on the server side. Auth tokens and custom headers
    /// are injected via `StreamableHttpClientTransportConfig`.
    async fn connect_server_http(
        key: &str,
        config: &McpServerConfig,
        url: &str,
    ) -> Result<McpServer> {
        use http::{HeaderName, HeaderValue};
        use rmcp::transport::streamable_http_client::{
            StreamableHttpClientTransport, StreamableHttpClientTransportConfig,
        };

        let mut transport_config = StreamableHttpClientTransportConfig::with_uri(url);

        // 1. Inject Bearer token
        if let Some(token) = &config.auth_token {
            transport_config = transport_config.auth_header(format!("Bearer {token}"));
        }

        // 2. Inject custom headers with environment variable interpolation
        if let Some(custom_headers) = &config.headers {
            let mut headers = std::collections::HashMap::new();
            for (k, v) in custom_headers {
                let header_name = HeaderName::from_bytes(k.as_bytes()).map_err(|e| {
                    Error::custom(format!("invalid header name '{k}' for '{key}': {e}"))
                })?;

                // Lightweight interpolation for `${VAR}` style
                let mut interpolated = v.to_string();
                while let Some(start) = interpolated.find("${") {
                    if let Some(end) = interpolated[start..].find('}') {
                        let end_idx = start + end;
                        let var_name = &interpolated[start + 2..end_idx];
                        let var_value = std::env::var(var_name).unwrap_or_default();
                        interpolated.replace_range(start..=end_idx, &var_value);
                    } else {
                        break;
                    }
                }

                let value = HeaderValue::from_str(&interpolated).map_err(|e| {
                    Error::custom(format!("invalid header value for '{k}' in '{key}': {e}"))
                })?;
                headers.insert(header_name, value);
            }
            transport_config = transport_config.custom_headers(headers);
        }

        info!("MCP server '{key}': connecting via HTTP → {url}");
        let transport = StreamableHttpClientTransport::from_config(transport_config);
        let service: RunningService<RoleClient, ()> = ()
            .serve(transport)
            .await
            .map_err(|e| Error::custom(format!("HTTP handshake with '{key}' ({url}): {e}")))?;
        let peer = service.peer().clone();

        Self::build_server_from_peer(key, config, peer, service, format!("[http] {url}")).await
    }

    /// Connect via stdio (local child process — original transport).
    async fn connect_server_stdio(key: &str, config: &McpServerConfig) -> Result<McpServer> {
        let mut cmd = Command::new(&config.command);
        cade_core::agent_env::apply_agent_env(&mut cmd);
        cmd.args(&config.args);
        for (k, v) in &config.env {
            cmd.env(k, v);
        }

        // rmcp 1.4's `TokioChildProcess::new()` silently overrides the
        // caller's Stdio settings (it forces stdin/stdout=piped, stderr=inherit
        // via the builder's Default). That re-enables the server's stderr and
        // pollutes CADE's terminal. Use the builder API directly so we can
        // pin stderr=null. `.spawn()` returns (transport, Option<ChildStderr>);
        // the second element is always None because stderr isn't piped.
        let (transport, _stderr) = TokioChildProcess::builder(cmd)
            .stderr(std::process::Stdio::null())
            .spawn()
            .map_err(|e| {
                Error::custom(format!(
                    "spawn MCP server '{key}' ({}): {e}",
                    config.command
                ))
            })?;

        let service = ()
            .serve(transport)
            .await
            .map_err(|e| Error::custom(format!("handshake with MCP server '{key}': {e}")))?;

        let peer = service.peer().clone();
        Self::build_server_from_peer(key, config, peer, service, config.command.clone()).await
    }

    /// Shared post-handshake logic: list tools, build `McpServer`.
    async fn build_server_from_peer(
        key: &str,
        config: &McpServerConfig,
        peer: rmcp::Peer<RoleClient>,
        service: rmcp::service::RunningService<RoleClient, ()>,
        command_display: String,
    ) -> Result<McpServer> {
        // Fetch all tools (paginated)
        let raw_tools = peer
            .list_all_tools()
            .await
            .map_err(|e| Error::custom(format!("list_tools from '{key}': {e}")))?;

        let write_set: std::collections::HashSet<&str> =
            config.write_tools.iter().map(|s| s.as_str()).collect();

        let tools: Vec<McpToolSchema> = raw_tools
            .into_iter()
            .map(|tool| {
                let original = tool.name.to_string();
                let prefixed = format!("{key}__{original}");
                let description = tool.description.as_deref().unwrap_or("").to_string();

                // Convert MCP input_schema (JsonObject) to OpenAI parameters Value
                let mut parameters = Value::Object((*tool.input_schema).clone());

                // Bug 1 fix: OpenAI requires "properties" even if empty for "type": "object"
                if let Some(obj) = parameters.as_object_mut()
                    && obj.get("type").and_then(|t| t.as_str()) == Some("object")
                    && !obj.contains_key("properties")
                {
                    obj.insert("properties".to_string(), json!({}));
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

                let mut schema = json!({
                    "name":        prefixed,
                    "description": description,
                    "parameters":  parameters,
                });

                if config.core_server {
                    schema["_is_core"] = json!(true);
                }

                McpToolSchema {
                    server_key: key.to_string(),
                    prefixed_name: prefixed,
                    original_name: original,
                    schema,
                    is_write,
                }
            })
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
        })
    }

    /// Find (server_index, original_tool_name) for a prefixed tool name.
    async fn find_tool_idx(&self, prefixed_name: &str) -> Option<(usize, String)> {
        let servers = self.servers.read().await;
        for (i, server) in servers.iter().enumerate() {
            if let Some(t) = server
                .tools
                .iter()
                .find(|t| t.prefixed_name == prefixed_name)
            {
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

// -- Content extraction

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
                None => true, // no audience = include for everyone
                Some(roles) => roles.contains(&rmcp::model::Role::Assistant),
            }
        })
        .collect();

    // If filtering left nothing (shouldn't happen, but be safe), fall back to all items
    let items = if assistant_items.is_empty() {
        content.iter().collect()
    } else {
        assistant_items
    };

    items
        .iter()
        .map(|c| match &c.raw {
            RawContent::Text(t) => t.text.clone(),
            RawContent::Image(_) => "[image]".to_string(),
            RawContent::Audio(_) => "[audio]".to_string(),
            RawContent::Resource(r) => match &r.resource {
                rmcp::model::ResourceContents::TextResourceContents { text, .. } => text.clone(),
                _ => "[binary resource]".to_string(),
            },
            _ => "[unsupported content]".to_string(),
        })
        .collect::<Vec<_>>()
        .join("\n")
}
