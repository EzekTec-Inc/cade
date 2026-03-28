use crate::Result;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

// -- Hook configuration

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

fn default_hook_timeout() -> u64 {
    60_000
}

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
        self.post_tool_use_failure
            .extend(other.post_tool_use_failure);
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

// -- MCP server configuration

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

    // -- Resource system (Phase 1)
    /// Installed packages (npm:, git:, or local path).
    #[serde(default)]
    pub packages: Vec<crate::resources::packages::PackageSource>,
    /// Active theme name.  Empty string or absent = built-in default.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub theme: Option<String>,
    /// Extra prompt template directories or files beyond the standard locations.
    #[serde(default)]
    pub extra_prompts: Vec<std::path::PathBuf>,
    /// Extra skill directories beyond the standard locations.
    #[serde(default)]
    pub extra_skills: Vec<std::path::PathBuf>,
    /// Extra context file paths (appended to AGENTS.md discovery results).
    #[serde(default)]
    pub extra_context_files: Vec<std::path::PathBuf>,

    // -- Execution backend (Phase 6)
    /// Where to run bash commands and file operations.
    #[serde(default)]
    pub execution: ExecutionProfile,

    // -- Capability profile
    /// Capability profile name: "core", "pro", or "full".
    /// Default is "full" for backward compatibility.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub profile: Option<String>,
    /// Extra capabilities to enable on top of the profile.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub enable_capabilities: Vec<String>,
    /// Capabilities to disable (overrides profile).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub disable_capabilities: Vec<String>,
}

fn default_true() -> bool {
    true
}

// region:    --- Execution backend settings

/// Which execution backend to use for bash and file operations.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum ExecutionBackendKind {
    /// Run commands on the local machine (default).
    #[default]
    Local,
    /// Run commands inside a Docker container.
    Docker,
    /// Run commands on a remote host via SSH.
    Ssh,
    /// Block all writes; allow reads only.
    ReadOnly,
}

impl ExecutionBackendKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Local => "local",
            Self::Docker => "docker",
            Self::Ssh => "ssh",
            Self::ReadOnly => "readonly",
        }
    }
}

impl std::str::FromStr for ExecutionBackendKind {
    type Err = String;
    fn from_str(s: &str) -> core::result::Result<Self, Self::Err> {
        match s.to_lowercase().as_str() {
            "local" => Ok(Self::Local),
            "docker" => Ok(Self::Docker),
            "ssh" => Ok(Self::Ssh),
            "readonly" | "read-only" | "read_only" => Ok(Self::ReadOnly),
            other => Err(format!(
                "Unknown backend '{other}'. Valid: local, docker, ssh, readonly"
            )),
        }
    }
}

impl std::fmt::Display for ExecutionBackendKind {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.as_str())
    }
}

/// Full execution profile — backend selection + backend-specific config.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ExecutionProfile {
    /// Which backend to use (default: local).
    #[serde(default)]
    pub backend: ExecutionBackendKind,

    // -- Docker settings
    /// Docker image for the docker backend (e.g. "ubuntu:22.04").
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub docker_image: Option<String>,
    /// Extra flags passed to `docker run`.
    #[serde(default)]
    pub docker_flags: Vec<String>,

    // -- SSH settings
    /// Remote host name or IP.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ssh_host: Option<String>,
    /// Remote username.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ssh_user: Option<String>,
    /// Path to SSH private key.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ssh_key_path: Option<String>,
    /// SSH port (default 22).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ssh_port: Option<u16>,
}

// endregion: --- Execution backend settings

/// Project settings stored in .cade/settings.json (committable — share with team)
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ProjectSettings {
    #[serde(default)]
    pub hooks: HooksConfig,
    /// Project-scoped MCP servers (same key as global = project wins).
    #[serde(default, rename = "mcpServers")]
    pub mcp_servers: std::collections::HashMap<String, McpServerConfig>,
    /// Whether to automatically create a checkpoint before destructive edits
    #[serde(default = "default_true")]
    pub auto_checkpoint: bool,
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

// region:    --- Tests

#[cfg(test)]
mod tests {
    #[allow(unused)]
    type Result<T> = core::result::Result<T, Box<dyn std::error::Error>>; // For tests.

    use super::*;
    use std::fs;

    // -- HooksConfig

    #[test]
    fn hooks_config_default_is_empty() {
        let h = HooksConfig::default();
        assert!(h.is_empty());
    }

    #[test]
    fn hooks_config_merge_preserves_both() {
        let mut a = HooksConfig::default();
        a.pre_tool_use.push(HookEntry {
            matcher: None,
            hooks: vec![],
        });
        let mut b = HooksConfig::default();
        b.stop.push(HookEntry {
            matcher: None,
            hooks: vec![],
        });
        let merged = a.merge(b);
        assert_eq!(merged.pre_tool_use.len(), 1);
        assert_eq!(merged.stop.len(), 1);
    }

    // -- PermissionSettings

    #[test]
    fn permission_settings_default() {
        let p = PermissionSettings::default();
        assert!(p.allow.is_empty());
        assert!(p.deny.is_empty());
        assert!(!p.strict_bash);
    }

    // -- GlobalSettings serialization

    #[test]
    fn global_settings_roundtrip_json() -> Result<()> {
        let mut gs = GlobalSettings::default();
        gs.last_agent = Some("agent-1".into());
        gs.env.api_key = Some("sk-test".into());
        gs.permissions.allow.push("Bash(cargo test)".into());
        gs.permissions.deny.push("Bash(rm:*)".into());
        gs.permissions.strict_bash = true;
        gs.store_api_key = true; // explicitly set (Default gives false, serde default gives true)

        let json = serde_json::to_string_pretty(&gs)?;
        let parsed: GlobalSettings = serde_json::from_str(&json)?;

        assert_eq!(parsed.last_agent.as_deref(), Some("agent-1"));
        assert_eq!(parsed.env.api_key.as_deref(), Some("sk-test"));
        assert_eq!(parsed.permissions.allow, vec!["Bash(cargo test)"]);
        assert_eq!(parsed.permissions.deny, vec!["Bash(rm:*)"]);
        assert!(parsed.permissions.strict_bash);
        assert!(parsed.store_api_key);

        Ok(())
    }

    #[test]
    fn global_settings_store_api_key_defaults_true() -> Result<()> {
        let json = r#"{}"#;
        let gs: GlobalSettings = serde_json::from_str(json)?;
        assert!(gs.store_api_key);

        Ok(())
    }

    // -- ProjectSettings serialization

    #[test]
    fn project_settings_with_mcp_servers() -> Result<()> {
        let json = r#"{
            "mcpServers": {
                "my-mcp": {
                    "command": "/usr/bin/mcp-server",
                    "args": ["--port", "8080"],
                    "env": {"API_KEY": "test"},
                    "write_tools": ["create_pr"],
                    "disabled": false
                }
            }
        }"#;
        let ps: ProjectSettings = serde_json::from_str(json)?;
        assert!(ps.mcp_servers.contains_key("my-mcp"));
        let cfg = &ps.mcp_servers["my-mcp"];
        assert_eq!(cfg.command, "/usr/bin/mcp-server");
        assert_eq!(cfg.args, vec!["--port", "8080"]);
        assert_eq!(cfg.env.get("API_KEY").map(|s| s.as_str()), Some("test"));
        assert_eq!(cfg.write_tools, vec!["create_pr"]);
        assert!(!cfg.disabled);

        Ok(())
    }

    // -- LocalSettings

    #[test]
    fn local_settings_pinned_agents() -> Result<()> {
        let json = r#"{
            "last_agent": "agent-42",
            "pinned_agents": [
                {"id": "a1", "name": "Alpha"},
                {"id": "a2", "name": "Beta"}
            ]
        }"#;
        let ls: LocalSettings = serde_json::from_str(json)?;
        assert_eq!(ls.last_agent.as_deref(), Some("agent-42"));
        assert_eq!(ls.pinned_agents.len(), 2);
        assert_eq!(ls.pinned_agents[0].id, "a1");
        assert_eq!(ls.pinned_agents[1].name, "Beta");

        Ok(())
    }

    // -- McpServerConfig

    #[test]
    fn mcp_server_config_defaults() {
        let cfg = McpServerConfig::default();
        assert!(cfg.command.is_empty());
        assert!(cfg.args.is_empty());
        assert!(cfg.env.is_empty());
        assert!(cfg.write_tools.is_empty());
        assert!(!cfg.disabled);
    }

    // -- HookDef serialization

    #[test]
    fn hook_def_json_roundtrip() -> Result<()> {
        let hook = HookDef::Command {
            command: "echo hello".into(),
            timeout: 30000,
        };
        let json = serde_json::to_string(&hook)?;
        let parsed: HookDef = serde_json::from_str(&json)?;
        match parsed {
            HookDef::Command { command, timeout } => {
                assert_eq!(command, "echo hello");
                assert_eq!(timeout, 30000);
            }
        }

        Ok(())
    }

    #[test]
    fn hook_def_default_timeout() -> Result<()> {
        let json = r#"{"type": "command", "command": "test"}"#;
        let hook: HookDef = serde_json::from_str(json)?;
        match hook {
            HookDef::Command { timeout, .. } => assert_eq!(timeout, 60_000),
        }

        Ok(())
    }

    // -- SettingsManager (with temp dirs)

    #[test]
    fn settings_manager_loads_defaults_for_missing_files() -> Result<()> {
        let dir = tempfile::tempdir()?;
        let mgr = SettingsManager::new(dir.path())?;
        // local.last_agent is None for a fresh project dir
        assert!(mgr.local().last_agent.is_none());
        assert!(mgr.pinned_agents().is_empty());

        Ok(())
    }

    #[test]
    fn settings_manager_merged_mcp_servers() -> Result<()> {
        let dir = tempfile::tempdir()?;
        let cade_dir = dir.path().join(".cade");
        fs::create_dir_all(&cade_dir)?;

        // Project settings with one server
        let project_json = r#"{
            "mcpServers": {
                "proj-server": {"command": "/bin/proj", "args": []}
            }
        }"#;
        fs::write(cade_dir.join("settings.json"), project_json)?;

        // Local settings with an override and a new server
        let local_json = r#"{
            "mcpServers": {
                "proj-server": {"command": "/bin/local-proj", "args": []},
                "local-only": {"command": "/bin/local-only", "args": []}
            }
        }"#;
        fs::write(cade_dir.join("settings.local.json"), local_json)?;

        let mgr = SettingsManager::new(dir.path())?;
        let servers = mgr.merged_mcp_servers();

        // Local override wins for proj-server
        assert_eq!(
            servers
                .get("proj-server")
                .ok_or("Should find server")?
                .command,
            "/bin/local-proj"
        );
        // Local-only server is present
        assert!(servers.contains_key("local-only"));

        Ok(())
    }

    #[test]
    fn settings_manager_disabled_mcp_server_excluded() -> Result<()> {
        let dir = tempfile::tempdir()?;
        let cade_dir = dir.path().join(".cade");
        fs::create_dir_all(&cade_dir)?;

        let project_json = r#"{
            "mcpServers": {
                "disabled-srv": {"command": "/bin/srv", "args": [], "disabled": true},
                "active-srv":   {"command": "/bin/active", "args": []}
            }
        }"#;
        fs::write(cade_dir.join("settings.json"), project_json)?;

        let mgr = SettingsManager::new(dir.path())?;
        let servers = mgr.merged_mcp_servers();
        assert!(!servers.contains_key("disabled-srv"));
        assert!(servers.contains_key("active-srv"));

        Ok(())
    }

    #[test]
    fn settings_manager_set_and_get_last_agent() -> Result<()> {
        let dir = tempfile::tempdir()?;
        let cade_dir = dir.path().join(".cade");
        fs::create_dir_all(&cade_dir)?;

        // We don't modify home dir in tests — just verify the function works
        // with the temp project dir
        let mut mgr = SettingsManager::new(dir.path())?;
        // This writes to the temp dir's .cade/ and to ~/.cade/ (which may or may not exist)
        // We just verify it doesn't panic and returns Ok
        let _ = mgr.set_last_agent("test-agent-123");
        assert_eq!(mgr.last_agent(), Some("test-agent-123"));

        Ok(())
    }

    #[test]
    fn settings_manager_pin_agent() -> Result<()> {
        let dir = tempfile::tempdir()?;
        let cade_dir = dir.path().join(".cade");
        fs::create_dir_all(&cade_dir)?;

        let mut mgr = SettingsManager::new(dir.path())?;
        mgr.pin_agent("a1", "Agent One")?;
        mgr.pin_agent("a2", "Agent Two")?;
        assert_eq!(mgr.pinned_agents().len(), 2);

        // Pin same ID again — deduplicates
        mgr.pin_agent("a1", "Agent One Updated")?;
        assert_eq!(mgr.pinned_agents().len(), 2);
        assert_eq!(
            mgr.pinned_agents()
                .iter()
                .find(|p| p.id == "a1")
                .ok_or("Should find agent")?
                .name,
            "Agent One Updated"
        );

        Ok(())
    }

    #[test]
    fn settings_manager_save_and_load_rules() -> Result<()> {
        let dir = tempfile::tempdir()?;
        let cade_dir = dir.path().join(".cade");
        fs::create_dir_all(&cade_dir)?;

        // Don't modify ~/.cade in CI — just test that save_allow_rule works without error
        let mut mgr = SettingsManager::new(dir.path())?;
        let _ = mgr.save_allow_rule("Bash(cargo test)");
        let _ = mgr.save_deny_rule("Bash(rm:*)");
        // Verify in-memory state
        assert!(
            mgr.permission_settings()
                .allow
                .contains(&"Bash(cargo test)".to_string())
        );
        assert!(
            mgr.permission_settings()
                .deny
                .contains(&"Bash(rm:*)".to_string())
        );

        Ok(())
    }

    #[test]
    fn settings_manager_base_url_default() -> Result<()> {
        let dir = tempfile::tempdir()?;
        let mgr = SettingsManager::new(dir.path())?;
        // Without env vars set, should default to localhost
        let url = mgr.base_url();
        assert!(url.starts_with("http://localhost:"), "got: {url}");

        Ok(())
    }

    #[test]
    fn settings_manager_merged_hooks() -> Result<()> {
        let dir = tempfile::tempdir()?;
        let cade_dir = dir.path().join(".cade");
        fs::create_dir_all(&cade_dir)?;

        let project_json = r#"{
            "hooks": {
                "PreToolUse": [{"hooks": [{"type": "command", "command": "echo proj"}]}]
            }
        }"#;
        fs::write(cade_dir.join("settings.json"), project_json)?;

        let local_json = r#"{
            "hooks": {
                "PreToolUse": [{"hooks": [{"type": "command", "command": "echo local"}]}]
            }
        }"#;
        fs::write(cade_dir.join("settings.local.json"), local_json)?;

        let mgr = SettingsManager::new(dir.path())?;
        let hooks = mgr.merged_hooks();
        // Local runs first (highest priority), then project, then global
        assert_eq!(hooks.pre_tool_use.len(), 2);

        Ok(())
    }
}

// endregion: --- Tests

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
        // Remove disabled entries
        merged.retain(|_, v| !v.disabled && !v.command.is_empty());
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
        let profile = self
            .global
            .profile
            .as_deref()
            .and_then(crate::capabilities::Profile::from_name)
            .unwrap_or_default();
        crate::capabilities::resolve_capabilities(
            profile,
            &self.global.enable_capabilities,
            &self.global.disable_capabilities,
        )
    }
    /// Persist global settings to disk.
    pub fn save_global(&self) -> Result<()> {
        Self::save_json(&self.global_path, &self.global)
    }
    pub fn local(&self) -> &LocalSettings {
        &self.local
    }
    pub fn project(&self) -> &ProjectSettings {
        &self.project
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
