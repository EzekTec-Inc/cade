use super::hooks::*;
use super::models::*;
use crate::Result;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

pub struct SettingsManager {
    global_path: PathBuf,
    project_path: PathBuf,
    local_path: PathBuf,
    global: GlobalSettings,
    project: ProjectSettings,
    local: LocalSettings,
}

impl SettingsManager {
    pub fn new(cwd: &Path) -> Result<Self> {
        let home = dirs::home_dir().ok_or("cannot resolve home dir")?;
        let global_path = home.join(".cade").join("settings.json");
        let project_path = cwd.join(".cade").join("settings.json");
        let local_path = cwd.join(".cade").join("settings.local.json");

        let global: GlobalSettings = Self::load_json(&global_path).unwrap_or_default();
        let project: ProjectSettings = Self::load_json(&project_path).unwrap_or_default();
        let local: LocalSettings = Self::load_json(&local_path).unwrap_or_default();

        Ok(Self {
            global_path,
            project_path,
            local_path,
            global,
            project,
            local,
        })
    }

    /// Reload settings from disk (useful for hot-reloading).
    pub fn reload(&mut self) -> Result<()> {
        self.global = Self::load_json(&self.global_path).unwrap_or_default();
        self.project = Self::load_json(&self.project_path).unwrap_or_default();
        self.local = Self::load_json(&self.local_path).unwrap_or_default();
        Ok(())
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
        // Remove disabled entries and entries with no transport configured
        merged.retain(|_, v| !v.disabled && (!v.command.is_empty() || v.url.is_some()));
        merged
    }

    /// Merged hooks config: local first (highest priority), then project, then global.
    pub fn merged_hooks(&self) -> HooksConfig {
        // Clone each source; local runs first per CADE spec
        let local = self.local.hooks.clone();
        let project = self.project.hooks.clone();
        let global = self.global.hooks.clone();
        local.merge(project).merge(global)
    }

    /// Path to the project settings file (.cade/settings.json — committable)
    pub fn project_path(&self) -> &Path {
        &self.project_path
    }
    /// Path to the local settings file (.cade/settings.local.json — gitignored)
    pub fn local_path(&self) -> &Path {
        &self.local_path
    }
    /// Path to the global settings file (~/.cade/settings.json)
    pub fn global_path(&self) -> &Path {
        &self.global_path
    }

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

    /// Resolve API key: CADE_API_KEY env var > global settings file > bootstrap token.
    /// SEC-B2: If `store_api_key` is false in settings, the settings-file fallback is
    /// skipped, but the bootstrap token at `~/.cade/api-token` is still consulted so the
    /// CLI can talk to its auto-spawned server.  The token is created on demand.
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
            .or_else(|| {
                let path = crate::bootstrap_token::default_token_path()?;
                // Prefer read-only when possible; fall back to create-on-demand so
                // the CLI never races the server for the token file.
                crate::bootstrap_token::read_existing_token(&path)
                    .or_else(|| crate::bootstrap_token::load_or_create_token(&path).ok())
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
        self.local
            .last_agent
            .as_deref()
            .or(self.global.last_agent.as_deref())
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
        self.local.pinned_agents.push(PinnedAgent {
            id: id.to_string(),
            name: name.to_string(),
        });
        Self::save_json(&self.local_path, &self.local)
    }

    pub fn pinned_agents(&self) -> &[PinnedAgent] {
        &self.local.pinned_agents
    }

    /// Resolve reasoning effort: command line args (if provided) > local settings > global settings.
    pub fn reasoning_effort(&self) -> Option<String> {
        self.local
            .reasoning_effort
            .clone()
            .or_else(|| self.global.reasoning_effort.clone())
    }

    /// Set reasoning effort and save to local settings.
    pub fn set_reasoning_effort(&mut self, effort: Option<String>) -> Result<()> {
        self.local.reasoning_effort = effort;
        self.save_local()
    }

    pub fn global(&self) -> &GlobalSettings {
        &self.global
    }
    pub fn global_settings_mut(&mut self) -> &mut GlobalSettings {
        &mut self.global
    }

    /// Returns the active execution profile.
    pub fn execution_profile(&self) -> &ExecutionProfile {
        &self.global.execution
    }

    /// Resolve the effective capability set from settings.
    pub fn resolve_capabilities(&self) -> crate::capabilities::CapabilitySet {
        crate::capabilities::resolve_capabilities(
            &self.global.enable_capabilities,
            &self.global.disable_capabilities,
        )
    }
    /// Persist global settings to disk.
    pub fn save_global(&self) -> Result<()> {
        Self::save_json(&self.global_path, &self.global)
    }

    /// Persist local settings to disk.
    pub fn save_local(&self) -> Result<()> {
        Self::save_json(&self.local_path, &self.local)
    }
    pub fn local(&self) -> &LocalSettings {
        &self.local
    }
    pub fn project(&self) -> &ProjectSettings {
        &self.project
    }

    /// Whether subagent live streaming should be silenced.
    pub fn silent_subagents(&self) -> bool {
        self.project
            .silent_subagents
            .unwrap_or(self.global.silent_subagents.unwrap_or(false))
    }

    /// Retrieve the optional maximum context budget limit (in chars).
    pub fn max_context_budget(&self) -> Option<usize> {
        self.project
            .max_context_budget
            .or(self.global.max_context_budget)
    }

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
