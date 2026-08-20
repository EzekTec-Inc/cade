//! Unified PluginEngine Seam (Candidate 3).
//!
//! Encapsulates plugin discovery, installation from tarball/marketplace,
//! manifest validation, and tool dispatch behind a single deep interface.

use async_trait::async_trait;
use parking_lot::RwLock;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::path::{Path, PathBuf};

use crate::marketplace::install_plugin;
use crate::registry::{PluginRegistry, ResolvedPluginTool};
use crate::{Error, Result};

// region:    --- Types

/// High-level report returned after installing or loading a plugin.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct PluginReport {
    pub id: String,
    pub name: String,
    pub version: String,
    pub tools_count: usize,
    pub skills_count: usize,
    pub mcp_servers_count: usize,
}

/// Unified interface for managing plugin lifecycles, marketplace installs, and tool execution.
#[async_trait]
pub trait PluginEngine: Send + Sync {
    /// Discover and load all plugins from the search directories.
    fn load_all(&self) -> Result<Vec<PluginReport>>;

    /// Install a plugin package from a remote URL or tarball into target directory.
    async fn install(&self, url: &str, plugin_id: &str) -> Result<PluginReport>;

    /// List all resolved plugin tools ready for agent execution.
    fn list_tools(&self) -> Vec<ResolvedPluginTool>;

    /// Dispatch a plugin tool execution.
    async fn dispatch(&self, tool_name: &str, args: &Value) -> Result<String>;
}

// endregion: --- Types

// region:    --- Native Plugin Engine

/// Native production implementation of the PluginEngine.
pub struct NativePluginEngine {
    search_dirs: Vec<PathBuf>,
    primary_install_dir: PathBuf,
    registry: RwLock<PluginRegistry>,
}

impl NativePluginEngine {
    pub fn new(search_dirs: Vec<PathBuf>, primary_install_dir: PathBuf) -> Self {
        let registry = PluginRegistry::discover(&search_dirs);
        Self {
            search_dirs,
            primary_install_dir,
            registry: RwLock::new(registry),
        }
    }

    pub fn from_default_dirs(cwd: &Path) -> Self {
        let mut dirs_list = Vec::new();

        // 1. Project local plugins
        dirs_list.push(cwd.join(".cade").join("plugins"));

        // 2. Global user plugins
        if let Some(home) = dirs::home_dir() {
            dirs_list.push(home.join(".cade").join("plugins"));
        }

        let primary_install_dir = dirs_list
            .first()
            .cloned()
            .unwrap_or_else(|| cwd.join(".cade").join("plugins"));

        Self::new(dirs_list, primary_install_dir)
    }
}

#[async_trait]
impl PluginEngine for NativePluginEngine {
    fn load_all(&self) -> Result<Vec<PluginReport>> {
        let fresh_registry = PluginRegistry::discover(&self.search_dirs);
        let schemas = fresh_registry.all_tool_schemas();
        let reports = vec![PluginReport {
            id: "all-plugins".to_string(),
            name: "Active Plugins".to_string(),
            version: "1.0.0".to_string(),
            tools_count: schemas.len(),
            skills_count: 0,
            mcp_servers_count: 0,
        }];
        *self.registry.write() = fresh_registry;
        Ok(reports)
    }

    async fn install(&self, url: &str, plugin_id: &str) -> Result<PluginReport> {
        let manifest = install_plugin(url, plugin_id, &self.primary_install_dir).await?;

        // Reload registry after installation
        let fresh_registry = PluginRegistry::discover(&self.search_dirs);
        *self.registry.write() = fresh_registry;

        Ok(PluginReport {
            id: plugin_id.to_string(),
            name: manifest.name,
            version: manifest.version.unwrap_or_else(|| "1.0.0".to_string()),
            tools_count: manifest.tools.len(),
            skills_count: manifest.skills.len(),
            mcp_servers_count: manifest.mcp_servers.len(),
        })
    }

    fn list_tools(&self) -> Vec<ResolvedPluginTool> {
        self.registry.read().list_resolved_tools()
    }

    async fn dispatch(&self, tool_name: &str, args: &Value) -> Result<String> {
        let handler = {
            let reg = self.registry.read();
            if !reg.has_tool(tool_name) {
                return Err(Error::custom(format!("Unknown plugin tool: {tool_name}")));
            }
            reg.find_tool_handler(tool_name)
                .ok_or_else(|| Error::custom(format!("Plugin tool '{tool_name}' has no executable handler")))?
        };

        let args_str = serde_json::to_string(args).unwrap_or_default();
        let (out, is_error) = crate::registry::execute_plugin_handler(&handler, &args_str).await;
        if is_error {
            Err(Error::custom(out))
        } else {
            Ok(out)
        }
    }
}

// endregion: --- Native Plugin Engine

// region:    --- Mock Plugin Engine

/// Mock adapter implementing PluginEngine for zero-I/O unit testing.
pub struct MockPluginEngine {
    pub canned_tools: Vec<ResolvedPluginTool>,
    pub canned_report: PluginReport,
}

impl Default for MockPluginEngine {
    fn default() -> Self {
        Self {
            canned_tools: vec![ResolvedPluginTool {
                name: "plugin__test_tool".to_string(),
                schema: serde_json::json!({
                    "name": "plugin__test_tool",
                    "description": "Mock plugin tool"
                }),
                handler: None,
                plugin_name: "test-plugin".to_string(),
            }],
            canned_report: PluginReport {
                id: "test-plugin".to_string(),
                name: "Test Plugin".to_string(),
                version: "1.0.0".to_string(),
                tools_count: 1,
                skills_count: 0,
                mcp_servers_count: 0,
            },
        }
    }
}

#[async_trait]
impl PluginEngine for MockPluginEngine {
    fn load_all(&self) -> Result<Vec<PluginReport>> {
        Ok(vec![self.canned_report.clone()])
    }

    async fn install(&self, _url: &str, _plugin_id: &str) -> Result<PluginReport> {
        Ok(self.canned_report.clone())
    }

    fn list_tools(&self) -> Vec<ResolvedPluginTool> {
        self.canned_tools.clone()
    }

    async fn dispatch(&self, tool_name: &str, _args: &Value) -> Result<String> {
        if self.canned_tools.iter().any(|t| t.name == tool_name) {
            Ok("Mock plugin output".to_string())
        } else {
            Err(Error::custom(format!("Unknown plugin tool: {tool_name}")))
        }
    }
}

// endregion: --- Mock Plugin Engine

// region:    --- Tests

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_mock_plugin_engine_seam() -> Result<()> {
        let mock = MockPluginEngine::default();

        let plugins = mock.load_all()?;
        assert_eq!(plugins.len(), 1);
        assert_eq!(plugins[0].id, "test-plugin");

        let tools = mock.list_tools();
        assert_eq!(tools.len(), 1);
        assert_eq!(tools[0].name, "plugin__test_tool");

        let out = mock.dispatch("plugin__test_tool", &serde_json::json!({})).await?;
        assert_eq!(out, "Mock plugin output");

        let err = mock.dispatch("nonexistent", &serde_json::json!({})).await;
        assert!(err.is_err());

        Ok(())
    }
}

// endregion: --- Tests