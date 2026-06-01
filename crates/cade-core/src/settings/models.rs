use serde::{Deserialize, Serialize};

use super::hooks::*;
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
    /// When true, allows the LLM to autonomously change the permission mode
    /// via `EnterPlanMode` and `ExitPlanMode` tools. When false, these tools
    /// are hidden from the LLM context.
    #[serde(default)]
    pub allow_agent_mode_changes: bool,
}

// -- MCP server configuration

/// Configuration for a single MCP server (matches Claude Desktop / VS Code format).
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct McpServerConfig {
    /// Absolute path (or executable name on PATH) to the server binary.
    /// Mutually exclusive with `url` — if `url` is set this field is ignored.
    #[serde(default)]
    pub command: String,
    /// CLI arguments passed to the server process.
    #[serde(default)]
    pub args: Vec<String>,
    /// Extra environment variables injected into the server process.
    #[serde(default)]
    pub env: std::collections::HashMap<String, String>,
    /// Remote MCP server URL (HTTP+SSE or Streamable HTTP transport).
    ///
    /// When set, CADE connects over HTTP instead of spawning a child process.
    ///
    /// - Legacy SSE servers (pre-2025-03-26): `http://host/sse`
    /// - Streamable HTTP servers (MCP 2025-03-26): `http://host/mcp`
    ///
    /// Mutually exclusive with `command`.
    #[serde(default)]
    pub url: Option<String>,
    /// Bearer token for authenticated remote MCP servers.
    ///
    /// When set alongside `url`, CADE sends `Authorization: Bearer <token>`
    /// on every HTTP request to the remote server.
    /// Has no effect for stdio (`command`-based) servers.
    #[serde(default)]
    pub auth_token: Option<String>,
    /// Custom HTTP headers sent to remote servers (HTTP/SSE transports).
    /// Environment variables like `${MY_TOKEN}` are supported.
    #[serde(default)]
    pub headers: Option<std::collections::HashMap<String, String>>,
    /// Tool names that mutate state (require permission prompt).
    /// If not set, ALL tools from this server require permission.
    #[serde(default)]
    pub write_tools: Vec<String>,
    /// If true, skip this server on startup (disabled without removing the entry).
    #[serde(default)]
    pub disabled: bool,
    /// If true, tools from this server are considered "core" and never pruned from the LLM context.
    #[serde(default)]
    pub core_server: bool,
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

    /// Maximum context budget (in characters). Caps the default token-based limit.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_context_budget: Option<usize>,
    /// Maximum tokens per turn. Exceeding this triggers a split turn boundary cut.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_tokens_per_turn: Option<usize>,
    /// Extra context file paths (appended to AGENTS.md discovery results).
    #[serde(default)]
    pub extra_context_files: Vec<std::path::PathBuf>,

    // -- Execution backend (Phase 6)
    /// Where to run bash commands and file operations.
    #[serde(default)]
    pub execution: ExecutionProfile,

    // -- Subagents
    /// Whether to silence the live streaming output of subagents.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub silent_subagents: Option<bool>,

    // -- Capability profile
    /// Extra capabilities to enable on top of the default.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub enable_capabilities: Vec<String>,
    /// Capabilities to disable.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub disable_capabilities: Vec<String>,

    /// Default reasoning effort for models that support it (e.g. o1, o3-mini).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reasoning_effort: Option<String>,

    /// Plugin marketplace registry URL. Defaults to the official CADE registry.
    #[serde(default = "default_marketplace_url")]
    pub marketplace_url: String,
}

fn default_true() -> bool {
    true
}

const DEFAULT_MARKETPLACE_URL: &str =
    "https://raw.githubusercontent.com/EzekTec-Inc/cade-registry/main/index.json";

fn default_marketplace_url() -> String {
    DEFAULT_MARKETPLACE_URL.to_string()
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
    /// Run commands in a virtual restricted local sandbox.
    Virtual,
}

impl ExecutionBackendKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Local => "local",
            Self::Docker => "docker",
            Self::Ssh => "ssh",
            Self::ReadOnly => "readonly",
            Self::Virtual => "virtual",
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
            "virtual" => Ok(Self::Virtual),
            other => Err(format!(
                "Unknown backend '{other}'. Valid: local, docker, ssh, readonly, virtual"
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
    /// Whether to silence the live streaming output of subagents.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub silent_subagents: Option<bool>,
    /// Maximum context budget (in characters). Caps the default token-based limit.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_context_budget: Option<usize>,
    /// Maximum tokens per turn. Exceeding this triggers a split turn boundary cut.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_tokens_per_turn: Option<usize>,
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
    /// Local override for reasoning effort.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reasoning_effort: Option<String>,
    /// Maximum tokens per turn. Exceeding this triggers a split turn boundary cut.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_tokens_per_turn: Option<usize>,
}

// region:    --- Tests

// endregion: --- Tests
