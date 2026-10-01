/// Plugin registry: discovers, loads, and dispatches plugin tools.
use std::collections::HashMap;
use std::path::{Path, PathBuf};

use serde_json::Value;
use tokio::process::Command;

use crate::manifest::PluginManifest;

/// Manifest-backed plugin inventory entry exposed through `PluginEngine`.
#[derive(Debug, Clone)]
pub struct PluginInventoryEntry {
    pub id: String,
    pub name: String,
    pub version: String,
    pub scope: String,
    pub tools_count: usize,
    pub skills_count: usize,
    pub mcp_servers_count: usize,
}

// region:    --- Types

/// A resolved plugin tool ready for dispatch.
#[derive(Debug, Clone)]
pub struct ResolvedPluginTool {
    pub name: String,
    pub schema: Value,
    pub handler: Option<PathBuf>, // executable script
    pub plugin_name: String,
    /// Canonical package root; also disambiguates equal display names.
    pub plugin_root: PathBuf,
}

/// The plugin registry holds all discovered plugins and their tools.
pub struct PluginRegistry {
    plugins: Vec<LoadedPlugin>,
    /// Canonical tool name → resolved tool (for fast dispatch)
    tool_map: HashMap<String, ResolvedPluginTool>,
}

struct LoadedPlugin {
    manifest: PluginManifest,
    root: PathBuf,
}

// endregion: --- Types

// region:    --- PluginRegistry

impl PluginRegistry {
    // -- Constructor

    /// Create an empty registry.
    pub fn empty() -> Self {
        Self {
            plugins: Vec::new(),
            tool_map: HashMap::new(),
        }
    }

    /// Discover and load all plugins from the given search directories.
    pub fn discover(search_dirs: &[PathBuf]) -> Self {
        let mut registry = Self::empty();
        for dir in search_dirs {
            if !dir.exists() {
                continue;
            }
            let Ok(entries) = std::fs::read_dir(dir) else {
                continue;
            };
            let mut paths: Vec<_> = entries.flatten().map(|entry| entry.path()).collect();
            paths.sort();
            for path in paths {
                if path
                    .file_name()
                    .is_some_and(|name| name.to_string_lossy().starts_with('.'))
                {
                    continue;
                }
                // Search directories are ordered project first, then global.
                if registry
                    .plugins
                    .iter()
                    .any(|plugin| plugin.root.file_name() == path.file_name())
                {
                    continue;
                }
                if path.is_dir()
                    && let Err(e) = registry.load_plugin(&path)
                {
                    tracing::warn!("Failed to load plugin at {}: {e}", path.display());
                }
            }
        }
        registry
    }

    fn load_plugin(&mut self, root: &Path) -> crate::Result<()> {
        let root = root.canonicalize()?;
        let manifest = PluginManifest::load(&root)?;
        for definition in &manifest.tools {
            match resolve_tool(&root, &manifest.name, definition) {
                Ok(Some(tool)) => {
                    self.tool_map.entry(tool.name.clone()).or_insert(tool);
                }
                Ok(None) => {} // A declaration without a handler is not a ready capability.
                Err(error) => {
                    tracing::warn!(%error, plugin = %manifest.name, "plugin tool is not ready")
                }
            }
        }

        self.plugins.push(LoadedPlugin { manifest, root });
        Ok(())
    }

    // -- Accessors

    pub fn is_empty(&self) -> bool {
        self.plugins.is_empty()
    }

    /// Manifest-backed plugin inventory sorted by stable plugin identifier.
    pub fn inventory(&self, project_dir: &Path) -> Vec<PluginInventoryEntry> {
        let tools = self.list_resolved_tools();
        let project_dir = project_dir
            .canonicalize()
            .unwrap_or_else(|_| project_dir.to_path_buf());
        let mut entries = self
            .plugins
            .iter()
            .map(|plugin| PluginInventoryEntry {
                id: plugin
                    .root
                    .file_name()
                    .and_then(|name| name.to_str())
                    .unwrap_or("unknown")
                    .to_string(),
                name: plugin.manifest.name.clone(),
                version: plugin
                    .manifest
                    .version
                    .clone()
                    .unwrap_or_else(|| "0.0.0".to_string()),
                scope: if plugin.root.starts_with(&project_dir) {
                    "project".to_string()
                } else {
                    "global".to_string()
                },
                tools_count: tools
                    .iter()
                    .filter(|tool| tool.plugin_root == plugin.root)
                    .count(),
                skills_count: plugin.manifest.skills.len(),
                mcp_servers_count: plugin.manifest.mcp_servers.len(),
            })
            .collect::<Vec<_>>();
        entries.sort_by(|left, right| left.id.cmp(&right.id));
        entries
    }

    /// All tool JSON schemas contributed by loaded plugins.
    pub fn all_tool_schemas(&self) -> Vec<Value> {
        self.list_resolved_tools()
            .into_iter()
            .map(|tool| tool.schema)
            .collect()
    }

    /// All resolved plugin tools ready for execution.
    pub fn list_resolved_tools(&self) -> Vec<ResolvedPluginTool> {
        let mut tools: Vec<_> = self
            .tool_map
            .values()
            .filter(|tool| tool_is_ready(tool))
            .cloned()
            .collect();
        tools.sort_by(|left, right| left.name.cmp(&right.name));
        tools
    }

    /// Check if a tool name belongs to a plugin.
    pub fn has_tool(&self, name: &str) -> bool {
        self.tool_map.get(name).is_some_and(tool_is_ready)
    }

    /// Retrieve the executable handler path for a plugin tool.
    pub fn find_tool_handler(&self, name: &str) -> Option<PathBuf> {
        let tool = self.tool_map.get(name)?;
        tool_is_ready(tool).then(|| tool.handler.clone()).flatten()
    }

    /// All skills directories from loaded plugins.
    pub fn all_skill_dirs(&self) -> Vec<PathBuf> {
        self.plugins
            .iter()
            .flat_map(|plugin| {
                plugin
                    .manifest
                    .skills
                    .iter()
                    .filter_map(|path| package_path(&plugin.root, path).ok())
            })
            .collect()
    }

    /// All prompt template directories/files from loaded plugins.
    pub fn all_prompt_paths(&self) -> Vec<PathBuf> {
        self.plugins
            .iter()
            .flat_map(|plugin| {
                plugin
                    .manifest
                    .prompts
                    .iter()
                    .filter_map(|path| package_path(&plugin.root, path).ok())
            })
            .collect()
    }

    /// All theme directories/files from loaded plugins.
    pub fn all_theme_paths(&self) -> Vec<PathBuf> {
        self.plugins
            .iter()
            .flat_map(|plugin| {
                plugin
                    .manifest
                    .themes
                    .iter()
                    .filter_map(|path| package_path(&plugin.root, path).ok())
            })
            .collect()
    }

    // -- Dispatch

    /// Execute a plugin tool by name.  Returns None if the tool is unknown.
    pub async fn dispatch(&self, tool_name: &str, args: &Value) -> Option<(String, bool)> {
        let handler = self.find_tool_handler(tool_name)?;

        let args_str = serde_json::to_string(args).unwrap_or_default();
        let result = execute_plugin_handler(&handler, &args_str, None).await;
        Some(result)
    }
}

// endregion: --- PluginRegistry

// region:    --- Support

/// Resolve a manifest path without allowing it to leave its package, including through symlinks.
pub(crate) fn package_path(root: &Path, relative: &Path) -> crate::Result<PathBuf> {
    if relative.is_absolute()
        || relative.components().any(|part| {
            !matches!(
                part,
                std::path::Component::Normal(_) | std::path::Component::CurDir
            )
        })
    {
        return Err(crate::Error::custom(format!(
            "Invalid package path: {}",
            relative.display()
        )));
    }
    let root = root.canonicalize()?;
    let path = root.join(relative).canonicalize()?;
    if !path.starts_with(&root) {
        return Err(crate::Error::custom(format!(
            "Package path escapes plugin: {}",
            relative.display()
        )));
    }
    Ok(path)
}

pub(crate) fn executable(path: &Path) -> bool {
    let Ok(metadata) = path.metadata() else {
        return false;
    };
    if !metadata.is_file() || path.extension().is_some_and(|ext| ext == "wasm") {
        return false;
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        metadata.permissions().mode() & 0o111 != 0
    }
    #[cfg(not(unix))]
    {
        true
    }
}

fn tool_is_ready(tool: &ResolvedPluginTool) -> bool {
    tool.plugin_root.is_dir()
        && tool.handler.as_ref().is_some_and(|path| {
            path.canonicalize().is_ok_and(|resolved| {
                resolved.starts_with(&tool.plugin_root) && executable(&resolved)
            })
        })
}

pub(crate) fn resolve_tool(
    root: &Path,
    plugin_name: &str,
    definition: &crate::manifest::PluginToolDef,
) -> crate::Result<Option<ResolvedPluginTool>> {
    let schema_path = package_path(root, &definition.schema)?;
    let schema: Value = serde_json::from_str(&std::fs::read_to_string(schema_path)?)?;
    let name = schema["name"]
        .as_str()
        .filter(|name| {
            !name.is_empty()
                && name
                    .chars()
                    .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
        })
        .ok_or_else(|| crate::Error::custom("Plugin tool schema requires a valid name"))?
        .to_owned();
    if !schema["parameters"].is_object() || schema["parameters"]["type"] != "object" {
        return Err(crate::Error::custom(format!(
            "Plugin tool '{name}' requires object parameters"
        )));
    }
    let Some(handler) = &definition.handler else {
        return Ok(None);
    };
    let handler = package_path(root, handler)?;
    if !executable(&handler) {
        return Err(crate::Error::custom(format!(
            "Plugin tool '{name}' handler is not an executable native script"
        )));
    }
    Ok(Some(ResolvedPluginTool {
        name,
        schema,
        handler: Some(handler),
        plugin_name: plugin_name.to_owned(),
        plugin_root: root.canonicalize()?,
    }))
}

pub(crate) async fn execute_plugin_handler(
    script: &Path,
    stdin_data: &str,
    cwd: Option<&Path>,
) -> (String, bool) {
    use tokio::io::AsyncWriteExt;

    let mut command = Command::new(script);
    command
        .kill_on_drop(true)
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped());
    if let Some(cwd) = cwd {
        command.current_dir(cwd);
    }
    let mut child = match command.spawn() {
        Ok(c) => c,
        Err(e) => return (format!("Failed to spawn plugin handler: {e}"), true),
    };

    // Drain output while writing stdin, including handlers that emit output
    // before consuming all arguments. Dropping this future kills the child.
    let stdin = child.stdin.take();
    let write = async {
        if let Some(mut stdin) = stdin {
            stdin.write_all(stdin_data.as_bytes()).await?;
        }
        Ok::<_, std::io::Error>(())
    };
    let result = tokio::try_join!(write, child.wait_with_output());
    match result {
        Err(e) => (format!("Plugin handler error: {e}"), true),
        Ok(((), out)) => {
            let stdout = String::from_utf8_lossy(&out.stdout).to_string();
            let stderr = String::from_utf8_lossy(&out.stderr).to_string();
            let is_error = !out.status.success();
            let output = if stderr.is_empty() {
                stdout
            } else {
                format!("{stdout}\n{stderr}")
            };
            (output, is_error)
        }
    }
}

// endregion: --- Support
