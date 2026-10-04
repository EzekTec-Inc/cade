//! Unified PluginEngine Seam (Candidate 3).
//!
//! Encapsulates plugin discovery, installation from tarball/marketplace,
//! manifest validation, and tool dispatch behind a single deep interface.

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::path::{Path, PathBuf};

use crate::marketplace::{install_plugin_with_checksum, validate_plugin_id};
use crate::registry::{PluginRegistry, ResolvedPluginTool};
use crate::{Error, Result};

// region:    --- Types

/// High-level report returned after installing or loading a plugin.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct PluginReport {
    pub id: String,
    pub name: String,
    pub version: String,
    pub scope: String,
    pub status: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub diagnostic: Option<String>,
    pub tools_count: usize,
    pub skills_count: usize,
    pub mcp_servers_count: usize,
}

/// Unified interface for managing plugin lifecycles, marketplace installs, and tool execution.
#[async_trait]
pub trait PluginEngine: Send + Sync {
    /// Discover and load all plugins from the search directories.
    fn load_all(&self) -> Result<Vec<PluginReport>>;

    /// Explicitly reload all plugins, refreshing internal registries and returning updated inventory.
    fn reload(&self) -> Result<Vec<PluginReport>> {
        self.load_all()
    }

    /// Install a plugin package from a remote URL or tarball into target directory.
    async fn install(&self, url: &str, plugin_id: &str) -> Result<PluginReport>;

    /// Verify archive integrity before activation. Adapters must not silently ignore a checksum.
    async fn install_with_checksum(
        &self,
        url: &str,
        plugin_id: &str,
        sha256: Option<&str>,
    ) -> Result<PluginReport> {
        if sha256.is_some() {
            return Err(Error::custom(
                "Checksum installation is not supported by this adapter",
            ));
        }
        self.install(url, plugin_id).await
    }

    /// Remove a project-local plugin by stable identifier and refresh the resolved registry.
    fn uninstall(&self, plugin_id: &str) -> Result<PluginReport>;

    /// List all resolved plugin tools ready for agent execution.
    fn list_tools(&self) -> Vec<ResolvedPluginTool>;

    /// Dispatch a plugin tool execution.
    async fn dispatch(&self, tool_name: &str, args: &Value) -> Result<String>;

    /// Search remote marketplace catalog for matching plugin packages.
    async fn search_marketplace(
        &self,
        registry_url: &str,
        query: &str,
    ) -> Result<Vec<crate::marketplace::RegistryPluginInfo>> {
        let index = crate::marketplace::fetch_catalog(registry_url).await?;
        Ok(crate::marketplace::search_catalog(&index, query)
            .into_iter()
            .cloned()
            .collect())
    }

    /// Install a plugin directly from the marketplace by identifier with automatic checksum verification.
    async fn install_from_marketplace(
        &self,
        registry_url: &str,
        plugin_id: &str,
    ) -> Result<PluginReport> {
        let index = crate::marketplace::fetch_catalog(registry_url).await?;
        let info = index
            .plugins
            .iter()
            .find(|p| p.id == plugin_id)
            .ok_or_else(|| Error::custom(format!("Plugin '{plugin_id}' not found in registry")))?;
        self.install_with_checksum(&info.url, plugin_id, info.sha256.as_deref())
            .await
    }
}

// endregion: --- Types

// region:    --- Native Plugin Engine

/// Native production implementation of the PluginEngine.
pub struct NativePluginEngine {
    search_dirs: Vec<PathBuf>,
    primary_install_dir: PathBuf,
    working_directory: Option<PathBuf>,
}

impl NativePluginEngine {
    pub fn new(mut search_dirs: Vec<PathBuf>, primary_install_dir: PathBuf) -> Self {
        if !search_dirs.contains(&primary_install_dir) {
            search_dirs.insert(0, primary_install_dir.clone());
        }
        Self {
            search_dirs,
            primary_install_dir,
            working_directory: None,
        }
    }

    /// Bind native child processes to the accepted workspace without changing process cwd.
    pub fn with_working_directory(mut self, cwd: PathBuf) -> Self {
        self.working_directory = Some(cwd);
        self
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
        let project_dir = self
            .search_dirs
            .first()
            .map(PathBuf::as_path)
            .unwrap_or(self.primary_install_dir.as_path());
        let reports = fresh_registry
            .inventory(project_dir)
            .into_iter()
            .map(|plugin| PluginReport {
                id: plugin.id,
                name: plugin.name,
                version: plugin.version,
                scope: plugin.scope,
                status: "active".to_string(),
                diagnostic: None,
                tools_count: plugin.tools_count,
                skills_count: plugin.skills_count,
                mcp_servers_count: plugin.mcp_servers_count,
            })
            .collect();
        Ok(reports)
    }

    async fn install(&self, url: &str, plugin_id: &str) -> Result<PluginReport> {
        self.install_with_checksum(url, plugin_id, None).await
    }

    async fn install_with_checksum(
        &self,
        url: &str,
        plugin_id: &str,
        sha256: Option<&str>,
    ) -> Result<PluginReport> {
        install_plugin_with_checksum(url, plugin_id, &self.primary_install_dir, sha256).await?;
        self.load_all()?
            .into_iter()
            .find(|report| report.id == plugin_id)
            .ok_or_else(|| Error::custom("Installed plugin could not be activated"))
    }

    fn uninstall(&self, plugin_id: &str) -> Result<PluginReport> {
        validate_plugin_id(plugin_id)?;
        let plugin_dir = self.primary_install_dir.join(plugin_id);
        if !plugin_dir.is_dir() {
            return Err(Error::custom(format!("Unknown plugin: {plugin_id}")));
        }

        if std::fs::symlink_metadata(&plugin_dir)?
            .file_type()
            .is_symlink()
        {
            return Err(Error::custom(
                "Cannot uninstall a symlinked plugin directory",
            ));
        }
        // Broken packages must remain removable.
        let manifest = crate::manifest::PluginManifest::load(&plugin_dir).unwrap_or_else(|_| {
            crate::manifest::PluginManifest {
                name: plugin_id.to_owned(),
                ..Default::default()
            }
        });
        let plugin_root = plugin_dir.canonicalize()?;
        let tools_count = self
            .list_tools()
            .iter()
            .filter(|tool| tool.plugin_root == plugin_root)
            .count();
        std::fs::remove_dir_all(&plugin_dir)?;

        Ok(PluginReport {
            id: plugin_id.to_string(),
            name: manifest.name,
            version: manifest.version.unwrap_or_else(|| "0.0.0".to_string()),
            scope: "project".to_string(),
            status: "removed".to_string(),
            diagnostic: None,
            tools_count,
            skills_count: manifest.skills.len(),
            mcp_servers_count: manifest.mcp_servers.len(),
        })
    }

    fn list_tools(&self) -> Vec<ResolvedPluginTool> {
        // A long-lived engine observes installs/removals made through any API instance.
        PluginRegistry::discover(&self.search_dirs).list_resolved_tools()
    }

    async fn dispatch(&self, tool_name: &str, args: &Value) -> Result<String> {
        let handler = {
            let reg = PluginRegistry::discover(&self.search_dirs);
            if !reg.has_tool(tool_name) {
                return Err(Error::custom(format!("Unknown plugin tool: {tool_name}")));
            }
            reg.find_tool_handler(tool_name).ok_or_else(|| {
                Error::custom(format!(
                    "Plugin tool '{tool_name}' has no executable handler"
                ))
            })?
        };

        let args_str = serde_json::to_string(args).unwrap_or_default();
        let (out, is_error) = crate::registry::execute_plugin_handler(
            &handler,
            &args_str,
            self.working_directory.as_deref(),
        )
        .await;
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
    removed: std::sync::atomic::AtomicBool,
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
                plugin_root: PathBuf::from("test-plugin"),
            }],
            canned_report: PluginReport {
                id: "test-plugin".to_string(),
                name: "Test Plugin".to_string(),
                version: "1.0.0".to_string(),
                scope: "project".to_string(),
                status: "active".to_string(),
                diagnostic: None,
                tools_count: 1,
                skills_count: 0,
                mcp_servers_count: 0,
            },
            removed: std::sync::atomic::AtomicBool::new(false),
        }
    }
}

#[async_trait]
impl PluginEngine for MockPluginEngine {
    fn load_all(&self) -> Result<Vec<PluginReport>> {
        Ok(if self.removed.load(std::sync::atomic::Ordering::SeqCst) {
            vec![]
        } else {
            vec![self.canned_report.clone()]
        })
    }

    async fn install(&self, _url: &str, _plugin_id: &str) -> Result<PluginReport> {
        self.removed
            .store(false, std::sync::atomic::Ordering::SeqCst);
        Ok(self.canned_report.clone())
    }

    fn uninstall(&self, plugin_id: &str) -> Result<PluginReport> {
        if plugin_id == self.canned_report.id
            && !self.removed.swap(true, std::sync::atomic::Ordering::SeqCst)
        {
            let mut report = self.canned_report.clone();
            report.status = "removed".to_string();
            Ok(report)
        } else {
            Err(Error::custom(format!("Unknown plugin: {plugin_id}")))
        }
    }

    fn list_tools(&self) -> Vec<ResolvedPluginTool> {
        if self.removed.load(std::sync::atomic::Ordering::SeqCst) {
            vec![]
        } else {
            self.canned_tools.clone()
        }
    }

    async fn dispatch(&self, tool_name: &str, _args: &Value) -> Result<String> {
        if self.list_tools().iter().any(|t| t.name == tool_name) {
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

        let out = mock
            .dispatch("plugin__test_tool", &serde_json::json!({}))
            .await?;
        assert_eq!(out, "Mock plugin output");

        let err = mock.dispatch("nonexistent", &serde_json::json!({})).await;
        assert!(err.is_err());

        let removed = mock.uninstall("test-plugin")?;
        assert_eq!(removed.status, "removed");
        assert!(mock.list_tools().is_empty());
        assert!(
            mock.dispatch("plugin__test_tool", &serde_json::json!({}))
                .await
                .is_err()
        );
        assert!(mock.uninstall("missing-plugin").is_err());

        Ok(())
    }
}

// endregion: --- Tests
