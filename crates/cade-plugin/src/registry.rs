/// Plugin registry: discovers, loads, and dispatches plugin tools.
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use serde_json::Value;
use tokio::process::Command;


use crate::manifest::PluginManifest;

// region:    --- Types

/// A resolved plugin tool ready for dispatch.
#[derive(Debug, Clone)]
pub struct ResolvedPluginTool {
    pub name:        String,
    pub schema:      Value,
    pub handler:     Option<PathBuf>,  // executable script
    pub plugin_name: String,
}

/// The plugin registry holds all discovered plugins and their tools.
pub struct PluginRegistry {
    plugins: Vec<LoadedPlugin>,
    /// Canonical tool name → resolved tool (for fast dispatch)
    tool_map: HashMap<String, Arc<ResolvedPluginTool>>,
}

struct LoadedPlugin {
    manifest: PluginManifest,
    root:     PathBuf,
    #[allow(dead_code)]
    tools:    Vec<Arc<ResolvedPluginTool>>,
}

// endregion: --- Types

// region:    --- PluginRegistry

impl PluginRegistry {
    // -- Constructor

    /// Create an empty registry.
    pub fn empty() -> Self {
        Self { plugins: Vec::new(), tool_map: HashMap::new() }
    }

    /// Discover and load all plugins from the given search directories.
    pub fn discover(search_dirs: &[PathBuf]) -> Self {
        let mut registry = Self::empty();
        for dir in search_dirs {
            if !dir.exists() { continue; }
            let Ok(entries) = std::fs::read_dir(dir) else { continue };
            for entry in entries.flatten() {
                let path = entry.path();
                if path.is_dir()
                    && let Err(e) = registry.load_plugin(&path) {
                        tracing::warn!("Failed to load plugin at {}: {e}", path.display());
                    }
            }
        }
        registry
    }

    fn load_plugin(&mut self, root: &Path) -> crate::Result<()> {
        let manifest = PluginManifest::load(root)?;
        let mut tools = Vec::new();

        // Load tool schemas from tools/ directory
        let tools_dir = root.join("tools");
        if tools_dir.exists() {
            let Ok(entries) = std::fs::read_dir(&tools_dir) else { return Ok(()); };
            for entry in entries.flatten() {
                let p = entry.path();
                if p.extension().and_then(|e| e.to_str()) != Some("json") { continue; }
                let Ok(content) = std::fs::read_to_string(&p) else { continue };
                let Ok(schema) = serde_json::from_str::<Value>(&content) else { continue };
                let Some(name) = schema["name"].as_str().map(String::from) else { continue };

                // Look for a matching handler script
                let handler = manifest.tools.iter()
                    .find(|t| {
                        t.schema.file_stem().and_then(|n| n.to_str())
                            .map(|stem| stem == name)
                            .unwrap_or(false)
                    })
                    .and_then(|t| t.handler.as_ref())
                    .map(|h| root.join(h));

                let tool = Arc::new(ResolvedPluginTool {
                    name: name.clone(),
                    schema,
                    handler,
                    plugin_name: manifest.name.clone(),
                });
                self.tool_map.insert(name, Arc::clone(&tool));
                tools.push(tool);
            }
        }

        self.plugins.push(LoadedPlugin {
            manifest,
            root: root.to_path_buf(),
            tools,
        });
        Ok(())
    }

    // -- Accessors

    pub fn is_empty(&self) -> bool { self.plugins.is_empty() }

    /// All tool JSON schemas contributed by loaded plugins.
    pub fn all_tool_schemas(&self) -> Vec<Value> {
        self.tool_map.values().map(|t| t.schema.clone()).collect()
    }

    /// Check if a tool name belongs to a plugin.
    pub fn has_tool(&self, name: &str) -> bool {
        self.tool_map.contains_key(name)
    }

    /// All skills directories from loaded plugins.
    pub fn all_skill_dirs(&self) -> Vec<PathBuf> {
        self.plugins.iter()
            .flat_map(|p| p.manifest.skills.iter().map(|s| {
                if s.is_absolute() { s.clone() } else { p.root.join(s) }
            }))
            .collect()
    }

    /// All prompt template directories/files from loaded plugins.
    pub fn all_prompt_paths(&self) -> Vec<PathBuf> {
        self.plugins.iter()
            .flat_map(|p| p.manifest.prompts.iter().map(|s| {
                if s.is_absolute() { s.clone() } else { p.root.join(s) }
            }))
            .collect()
    }

    /// All theme directories/files from loaded plugins.
    pub fn all_theme_paths(&self) -> Vec<PathBuf> {
        self.plugins.iter()
            .flat_map(|p| p.manifest.themes.iter().map(|s| {
                if s.is_absolute() { s.clone() } else { p.root.join(s) }
            }))
            .collect()
    }

    // -- Dispatch

    /// Execute a plugin tool by name.  Returns None if the tool is unknown.
    pub async fn dispatch(&self, tool_name: &str, args: &Value) -> Option<(String, bool)> {
        let tool = self.tool_map.get(tool_name)?;
        let handler = tool.handler.as_ref()?;

        let args_str = serde_json::to_string(args).unwrap_or_default();
        let result = execute_plugin_handler(handler, &args_str).await;
        Some(result)
    }
}

// endregion: --- PluginRegistry

// region:    --- Support

async fn execute_plugin_handler(script: &Path, stdin_data: &str) -> (String, bool) {
    use tokio::io::AsyncWriteExt;

    let mut child = match Command::new(script)
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
    {
        Ok(c)  => c,
        Err(e) => return (format!("Failed to spawn plugin handler: {e}"), true),
    };

    if let Some(mut stdin) = child.stdin.take() {
        let _ = stdin.write_all(stdin_data.as_bytes()).await;
    }

    match child.wait_with_output().await {
        Err(e) => (format!("Plugin handler error: {e}"), true),
        Ok(out) => {
            let stdout = String::from_utf8_lossy(&out.stdout).to_string();
            let stderr = String::from_utf8_lossy(&out.stderr).to_string();
            let is_error = !out.status.success();
            let output = if stderr.is_empty() { stdout } else { format!("{stdout}\n{stderr}") };
            (output, is_error)
        }
    }
}

// endregion: --- Support
