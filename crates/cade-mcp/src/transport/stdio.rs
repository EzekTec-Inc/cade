//! Stdio Transport Adapter.
//!
//! Spawns local child processes via stdio pipe with sandboxed environments,
//! stderr redirection, and singleton process guards.

// region:    --- Imports

use std::collections::HashSet;
use std::sync::OnceLock;
use tokio::process::Command;
use tokio::sync::Mutex;
use rmcp::transport::TokioChildProcess;
use rmcp::{RoleClient, ServiceExt, service::RunningService};

use cade_core::settings::McpServerConfig;
use crate::{Error, Result};

// endregion: --- Imports

// region:    --- Singleton Guard

static ACTIVE_SINGLETON_PROCESSES: OnceLock<Mutex<HashSet<String>>> = OnceLock::new();

fn get_active_singleton_processes() -> &'static Mutex<HashSet<String>> {
    ACTIVE_SINGLETON_PROCESSES.get_or_init(|| Mutex::new(HashSet::new()))
}

/// RAII guard that tracks and enforces singleton execution for designated MCP servers.
#[derive(Debug)]
pub struct SingletonProcessGuard {
    signature: Option<String>,
}

impl SingletonProcessGuard {
    pub async fn acquire(key: &str, config: &McpServerConfig) -> Result<Self> {
        let is_singleton = config.singleton.unwrap_or(false)
            || key == "serena"
            || config.command.ends_with("serena")
            || config.command.contains("serena");

        if !is_singleton {
            return Ok(Self { signature: None });
        }

        let sig = format!("{}:{}", key, config.command);
        let mut set = get_active_singleton_processes().lock().await;
        if set.contains(&sig) {
            return Err(Error::custom(format!(
                "Singleton process guard: MCP server '{key}' ({}) is already executing as an active singleton process. Refusing duplicate spawn.",
                config.command
            )));
        }
        set.insert(sig.clone());
        Ok(Self { signature: Some(sig) })
    }

    pub async fn release(&mut self) {
        if let Some(sig) = self.signature.take() {
            let mut set = get_active_singleton_processes().lock().await;
            set.remove(&sig);
        }
    }
}

impl Drop for SingletonProcessGuard {
    fn drop(&mut self) {
        if let Some(sig) = self.signature.take()
            && let Ok(handle) = tokio::runtime::Handle::try_current()
        {
            handle.spawn(async move {
                let mut set = get_active_singleton_processes().lock().await;
                set.remove(&sig);
            });
        }
    }
}

// endregion: --- Singleton Guard

// region:    --- Stdio Transport Adapter

pub struct StdioTransportAdapter;

impl StdioTransportAdapter {
    /// Build the standard command for spawning a local stdio MCP child process.
    pub fn build_command(config: &McpServerConfig) -> Command {
        let mut cmd = Command::new(&config.command);

        let is_sandboxed = config.sandboxed.unwrap_or(true);
        if is_sandboxed {
            cmd.env_clear();
            const SAFE_ENV_VARS: &[&str] = &[
                "PATH", "HOME", "LANG", "TZ", "TERM", "USER", "LOGNAME", "SHELL",
            ];
            for var in SAFE_ENV_VARS {
                if let Ok(val) = std::env::var(var) {
                    cmd.env(var, val);
                }
            }
        }

        cade_core::agent_env::apply_agent_env(&mut cmd);
        cmd.args(&config.args);
        for (k, v) in &config.env {
            cmd.env(k, v);
        }

        cmd
    }

    /// Spawn a child process and complete initial JSON-RPC service connection.
    pub async fn connect(
        key: &str,
        config: &McpServerConfig,
    ) -> Result<(RunningService<RoleClient, ()>, rmcp::Peer<RoleClient>, SingletonProcessGuard)> {
        let singleton_guard = SingletonProcessGuard::acquire(key, config).await?;

        if let Ok(mut log_file) = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open("/tmp/mcp_server_err.log")
        {
            use std::io::Write;
            let now = chrono::Utc::now().to_rfc3339();
            let _ = writeln!(
                log_file,
                "[{now}] [mcp-spawn] Spawning MCP server '{key}' ({})",
                config.command
            );
        }

        let cmd = Self::build_command(config);

        let stderr_io = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open("/tmp/mcp_server_err.log")
            .map(std::process::Stdio::from)
            .unwrap_or_else(|_| std::process::Stdio::null());

        let (transport, _stderr) = TokioChildProcess::builder(cmd)
            .stderr(stderr_io)
            .spawn()
            .map_err(|e| Error::custom(format!("spawn MCP server '{key}' ({}): {e}", config.command)))?;

        let service = ()
            .serve(transport)
            .await
            .map_err(|e| Error::custom(format!("handshake with MCP server '{key}': {e}")))?;

        let peer = service.peer().clone();
        Ok((service, peer, singleton_guard))
    }
}

// endregion: --- Stdio Transport Adapter
