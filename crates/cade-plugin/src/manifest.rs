/// Plugin manifest: declares what a plugin package provides.
///
/// Loaded from `cade-plugin.json` (preferred) or `package.json` in the
/// package root.  Paths are relative to the manifest file's directory.
use std::path::PathBuf;

use serde::{Deserialize, Serialize};

// region:    --- Types

/// A JSON-schema-based tool defined by a plugin.
/// The schema must follow the OpenAI function-calling format
/// (`name`, `description`, `parameters`).
///
/// When `handler` is set, CADE executes that script and passes tool
/// arguments as JSON on stdin.  When absent, the tool falls through to MCP.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PluginToolDef {
    /// Path to the JSON schema file, relative to the manifest directory.
    pub schema: PathBuf,
    /// Optional executable script to run when this tool is called.
    /// Receives JSON arguments on stdin; writes result to stdout.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub handler: Option<PathBuf>,
}

/// Shell-command hook declared in a plugin.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PluginHookDef {
    /// Event type this hook fires on (mirrors `HooksConfig` field names).
    pub event: String,
    /// Optional tool-name matcher (regex or "*").
    #[serde(skip_serializing_if = "Option::is_none")]
    pub matcher: Option<String>,
    /// Shell command to run.
    pub command: String,
    /// Timeout in milliseconds (default: 60 000).
    #[serde(default = "default_timeout")]
    pub timeout_ms: u64,
}

fn default_timeout() -> u64 { 60_000 }

/// Top-level plugin manifest.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct PluginManifest {
    pub name:    String,
    #[serde(default)]
    pub version: Option<String>,
    #[serde(default)]
    pub description: Option<String>,

    // Resources this plugin contributes
    #[serde(default)]
    pub tools:     Vec<PluginToolDef>,
    #[serde(default)]
    pub hooks:     Vec<PluginHookDef>,
    #[serde(default)]
    pub skills:    Vec<PathBuf>,
    #[serde(default)]
    pub prompts:   Vec<PathBuf>,
    #[serde(default)]
    pub themes:    Vec<PathBuf>,
    #[serde(default)]
    pub subagents: Vec<PathBuf>,
}

// endregion: --- Types

// region:    --- Parsing

impl PluginManifest {
    /// Load a manifest from `cade-plugin.json` or `package.json` in `root`.
    pub fn load(root: &std::path::Path) -> crate::Result<Self> {
        for name in &["cade-plugin.json", "package.json"] {
            let path = root.join(name);
            if path.exists() {
                let content = std::fs::read_to_string(&path)?;
                let manifest: Self = serde_json::from_str(&content)?;
                return Ok(manifest);
            }
        }
        // Auto-discover
        Ok(Self::auto_discover(root))
    }

    #[allow(clippy::field_reassign_with_default)]
    fn auto_discover(root: &std::path::Path) -> Self {
        let mut m = Self::default();
        m.name = root
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("unknown")
            .to_string();
        for (dir, field) in &[
            ("tools",    "tools"),
            ("skills",   "skills"),
            ("prompts",  "prompts"),
            ("themes",   "themes"),
            ("subagents","subagents"),
        ] {
            let p = root.join(dir);
            if p.is_dir() {
                match *field {
                    "skills"    => m.skills    = vec![p],
                    "prompts"   => m.prompts   = vec![p],
                    "themes"    => m.themes    = vec![p],
                    "subagents" => m.subagents = vec![p],
                    _ => {} // tools handled separately via JSON schemas
                }
            }
        }
        m
    }
}

// endregion: --- Parsing
