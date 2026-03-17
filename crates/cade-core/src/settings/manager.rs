use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

// ── Hook configuration ────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "lowercase")]
pub enum HookDef {
    /// Run a shell command. Exit 0=allow, 1=log+continue, 2=block+stderr→agent.
    Command {
        command: String,
        #[serde(default = "default_hook_timeout")]
        timeout: u64, // milliseconds
    },
}

fn default_hook_timeout() -> u64 { 60_000 }

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct HookEntry {
    /// Regex matcher for tool name (tool-related hooks only).
    /// None / "" / "*" → match all tools.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub matcher: Option<String>,
    pub hooks: Vec<HookDef>,
}

/// All hooks grouped by event type.
/// Field names match CADE's settings.json key names exactly.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct HooksConfig {
    #[serde(default, rename = "PreToolUse")]
    pub pre_tool_use: Vec<HookEntry>,
    #[serde(default, rename = "PostToolUse")]
    pub post_tool_use: Vec<HookEntry>,
    #[serde(default, rename = "PostToolUseFailure")]
    pub post_tool_use_failure: Vec<HookEntry>,
    #[serde(default, rename = "PermissionRequest")]
    pub permission_request: Vec<HookEntry>,
    #[serde(default, rename = "UserPromptSubmit")]
    pub user_prompt_submit: Vec<HookEntry>,
    #[serde(default, rename = "Stop")]
    pub stop: Vec<HookEntry>,
    #[serde(default, rename = "SubagentStop")]
    pub subagent_stop: Vec<HookEntry>,
    #[serde(default, rename = "SessionStart")]
    pub session_start: Vec<HookEntry>,
    #[serde(default, rename = "SessionEnd")]
    pub session_end: Vec<HookEntry>,
    #[serde(default, rename = "Notification")]
    pub notification: Vec<HookEntry>,
}

impl HooksConfig {
    /// Merge two configs: `self` runs first (higher priority).
    pub fn merge(mut self, other: HooksConfig) -> HooksConfig {
        self.pre_tool_use.extend(other.pre_tool_use);
        self.post_tool_use.extend(other.post_tool_use);
        self.post_tool_use_failure.extend(other.post_tool_use_failure);
        self.permission_request.extend(other.permission_request);
        self.user_prompt_submit.extend(other.user_prompt_submit);
        self.stop.extend(other.stop);
        self.subagent_stop.extend(other.subagent_stop);
        self.session_start.extend(other.session_start);
        self.session_end.extend(other.session_end);
        self.notification.extend(other.notification);
        self
    }

    pub fn is_empty(&self) -> bool {
        self.pre_tool_use.is_empty()
            && self.post_tool_use.is_empty()
            && self.post_tool_use_failure.is_empty()
            && self.permission_request.is_empty()
            && self.user_prompt_submit.is_empty()
            && self.stop.is_empty()
            && self.subagent_stop.is_empty()
            && self.session_start.is_empty()
            && self.session_end.is_empty()
            && self.notification.is_empty()
    }
}

/// Permission allow/deny rules persisted in settings.json
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct PermissionSettings {
    /// Patterns that are always allowed: e.g. ["Bash(cargo test)", "Read(src/**)"]
    #[serde(default)]
    pub allow: Vec<String>,
    /// Patterns that are always denied: e.g. ["Bash(rm -rf:*)"]
    #[serde(default)]
    pub deny: Vec<String>,
    /// SEC-B1: When true, bash / shell / run_command / execute_command are
    /// never auto-approved regardless of allow rules or permission mode.
    /// Every bash invocation will require explicit user confirmation.
    #[serde(default)]
    pub strict_bash: bool,
}

// ── MCP server configuration ──────────────────────────────────────────────────

/// Configuration for a single MCP server (matches Claude Desktop / VS Code format).
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct McpServerConfig {
    /// Absolute path (or executable name on PATH) to the server binary.
    pub command: String,
    /// CLI arguments passed to the server process.
    #[serde(default)]
    pub args: Vec<String>,
    /// Extra environment variables injected into the server process.
    #[serde(default)]
    pub env: std::collections::HashMap<String, String>,
    /// Tool names that mutate state (require permission prompt).
    /// If not set, ALL tools from this server require permission.
    #[serde(default)]
    pub write_tools: Vec<String>,
    /// If true, skip this server on startup (disabled without removing the entry).
    #[serde(default)]
    pub disabled: bool,
}

/// Global settings stored in ~/.cade/settings.json
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct GlobalSettings {
    #[serde(default)]
    pub env: EnvSettings,
    #[serde(default)]
    pub last_agent: Option<String>,
    #[serde(default)]
    pub permissions: PermissionSettings,
    #[serde(default)]
    pub hooks: HooksConfig,
    /// MCP servers available globally (all projects).
    #[serde(default, rename = "mcpServers")]
    pub mcp_servers: std::collections::HashMap<String, McpServerConfig>,
    /// SEC-B2: When false, the CLI ignores any `env.api_key` stored in this
    /// file and relies exclusively on environment variables (`CADE_API_KEY`).
    /// Default true preserves backward compatibility.
    #[serde(default = "default_true")]
    pub store_api_key: bool,
}

fn default_true() -> bool { true }

/// Project settings stored in .cade/settings.json (committable — share with team)
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ProjectSettings {
    #[serde(default)]
    pub hooks: HooksConfig,
    /// Project-scoped MCP servers (same key as global = project wins).
    #[serde(default, rename = "mcpServers")]
    pub mcp_servers: std::collections::HashMap<String, McpServerConfig>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct EnvSettings {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub api_key: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub server_url: Option<String>,
}

/// A pinned agent entry
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PinnedAgent {
    pub id: String,
    pub name: String,
}

/// Local project settings stored in .cade/settings.local.json (gitignored)
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct LocalSettings {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_agent: Option<String>,
    #[serde(default)]
    pub pinned_agents: Vec<PinnedAgent>,
    #[serde(default)]
    pub hooks: HooksConfig,
    /// Local MCP server overrides — gitignored, highest priority.
    /// Overrides project and global settings; use for machine-local servers.
    #[serde(default, rename = "mcpServers")]
    pub mcp_servers: std::collections::HashMap<String, McpServerConfig>,
}

pub struct SettingsManager {
    global_path:  PathBuf,
    project_path: PathBuf,
    local_path:   PathBuf,
    global:  GlobalSettings,
    project: ProjectSettings,
    local:   LocalSettings,
}

impl SettingsManager {
    pub fn new(cwd: &Path) -> Result<Self> {
        let home = dirs::home_dir().context("cannot resolve home dir")?;
        let global_path  = home.join(".cade").join("settings.json");
        let project_path = cwd.join(".cade").join("settings.json");
        let local_path   = cwd.join(".cade").join("settings.local.json");

        let global:  GlobalSettings  = Self::load_json(&global_path).unwrap_or_default();
        let project: ProjectSettings = Self::load_json(&project_path).unwrap_or_default();
        let local:   LocalSettings   = Self::load_json(&local_path).unwrap_or_default();

        Ok(Self { global_path, project_path, local_path, global, project, local })
    }

    /// Merged MCP servers: local > project > global (same key = higher priority wins).
    /// Disabled servers are excluded.
    pub fn merged_mcp_servers(&self) -> std::collections::HashMap<String, McpServerConfig> {
        let mut merged = self.global.mcp_servers.clone();
        // Project overrides global
        for (k, v) in &self.project.mcp_servers {
            merged.insert(k.clone(), v.clone());
        }
        // Local overrides project (highest priority — gitignored)
        for (k, v) in &self.local.mcp_servers {
            merged.insert(k.clone(), v.clone());
        }
        // Remove disabled entries
        merged.retain(|_, v| !v.disabled && !v.command.is_empty());
        merged
    }

    /// Merged hooks config: local first (highest priority), then project, then global.
    pub fn merged_hooks(&self) -> HooksConfig {
        // Clone each source; local runs first per CADE spec
        let local   = self.local.hooks.clone();
        let project = self.project.hooks.clone();
        let global  = self.global.hooks.clone();
        local.merge(project).merge(global)
    }

    /// Path to the project settings file (.cade/settings.json — committable)
    pub fn project_path(&self) -> &Path { &self.project_path }
    /// Path to the local settings file (.cade/settings.local.json — gitignored)
    pub fn local_path(&self) -> &Path { &self.local_path }
    /// Path to the global settings file (~/.cade/settings.json)
    pub fn global_path(&self) -> &Path { &self.global_path }

    fn load_json<T: for<'de> Deserialize<'de> + Default>(path: &Path) -> Result<T> {
        if !path.exists() {
            return Ok(T::default());
        }
        let content = std::fs::read_to_string(path)?;
        Ok(serde_json::from_str(&content)?)
    }

    fn save_json<T: Serialize>(path: &Path, value: &T) -> Result<()> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let content = serde_json::to_string_pretty(value)?;
        
        use std::io::Write;
        let mut opts = std::fs::OpenOptions::new();
        opts.write(true).create(true).truncate(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            opts.mode(0o600);
        }
        let mut file = opts.open(path)?;
        file.write_all(content.as_bytes())?;
        
        Ok(())
    }

    /// Resolve API key: CADE_API_KEY env var > global settings file.
    /// SEC-B2: If `store_api_key` is false in settings, the file-based
    /// fallback is skipped and only environment variables are used.
    pub fn api_key(&self) -> Option<String> {
        std::env::var("CADE_API_KEY")
            .ok()
            .or_else(|| std::env::var("CADE_LEGACY_API_KEY").ok()) // backward-compat
            .or_else(|| {
                if self.global.store_api_key {
                    self.global.env.api_key.clone()
                } else {
                    None
                }
            })
    }

    /// Resolve server URL: CADE_SERVER_URL env var > global settings > localhost
    pub fn base_url(&self) -> String {
        std::env::var("CADE_SERVER_URL")
            .ok()
            .or_else(|| std::env::var("CADE_LEGACY_BASE_URL").ok()) // backward-compat
            .or_else(|| self.global.env.server_url.clone())
            .unwrap_or_else(|| {
                // Respect CADE_SERVER_PORT so client and server stay in sync
                // when the user overrides the port via environment variable.
                let port = std::env::var("CADE_SERVER_PORT")
                    .ok()
                    .and_then(|p| p.parse::<u16>().ok())
                    .unwrap_or(8284);
                format!("http://localhost:{port}")
            })
    }

    /// Get the last used agent for this project directory
    pub fn last_agent(&self) -> Option<&str> {
        self.local.last_agent.as_deref()
            .or_else(|| self.global.last_agent.as_deref())
    }

    /// Save agent ID as last used (both local + global)
    pub fn set_last_agent(&mut self, agent_id: &str) -> Result<()> {
        self.local.last_agent = Some(agent_id.to_string());
        self.global.last_agent = Some(agent_id.to_string());
        Self::save_json(&self.local_path, &self.local)?;
        Self::save_json(&self.global_path, &self.global)?;
        Ok(())
    }

    /// Pin an agent by ID + name (deduplicates by ID).
    pub fn pin_agent(&mut self, id: &str, name: &str) -> Result<()> {
        self.local.pinned_agents.retain(|p| p.id != id);
        self.local.pinned_agents.push(PinnedAgent { id: id.to_string(), name: name.to_string() });
        Self::save_json(&self.local_path, &self.local)
    }

    pub fn pinned_agents(&self) -> &[PinnedAgent] {
        &self.local.pinned_agents
    }

    pub fn global(&self) -> &GlobalSettings { &self.global }
    pub fn local(&self) -> &LocalSettings { &self.local }

    /// Remove the API key from global settings and persist.
    /// Used by `/logout` to clear stored credentials.
    pub fn clear_api_key(&mut self) {
        self.global.env.api_key = None;
        let _ = Self::save_json(&self.global_path, &self.global);
    }

    pub fn permission_settings(&self) -> &PermissionSettings {
        &self.global.permissions
    }

    /// Append a rule to the global allow list and persist.
    pub fn save_allow_rule(&mut self, rule: &str) -> Result<()> {
        if !self.global.permissions.allow.contains(&rule.to_string()) {
            self.global.permissions.allow.push(rule.to_string());
            Self::save_json(&self.global_path, &self.global)?;
        }
        Ok(())
    }

    /// Append a rule to the global deny list and persist.
    pub fn save_deny_rule(&mut self, rule: &str) -> Result<()> {
        if !self.global.permissions.deny.contains(&rule.to_string()) {
            self.global.permissions.deny.push(rule.to_string());
            Self::save_json(&self.global_path, &self.global)?;
        }
        Ok(())
    }
}
