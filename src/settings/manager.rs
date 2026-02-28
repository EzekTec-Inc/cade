use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

/// Global settings stored in ~/.cade/settings.json
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct GlobalSettings {
    #[serde(default)]
    pub env: EnvSettings,
    #[serde(default)]
    pub last_agent: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct EnvSettings {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub letta_api_key: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub letta_base_url: Option<String>,
}

/// Local project settings stored in .cade/settings.local.json (gitignored)
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct LocalSettings {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_agent: Option<String>,
    #[serde(default)]
    pub pinned_agents: Vec<String>,
}

pub struct SettingsManager {
    global_path: PathBuf,
    local_path: PathBuf,
    global: GlobalSettings,
    local: LocalSettings,
}

impl SettingsManager {
    pub fn new(cwd: &Path) -> Result<Self> {
        let home = dirs::home_dir().context("cannot resolve home dir")?;
        let global_path = home.join(".cade").join("settings.json");
        let local_path = cwd.join(".cade").join("settings.local.json");

        let global = Self::load_json(&global_path).unwrap_or_default();
        let local = Self::load_json(&local_path).unwrap_or_default();

        Ok(Self { global_path, local_path, global, local })
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
        std::fs::write(path, content)?;
        Ok(())
    }

    /// Resolve API key: env var > global settings
    pub fn api_key(&self) -> Option<String> {
        std::env::var("LETTA_API_KEY")
            .ok()
            .or_else(|| self.global.env.letta_api_key.clone())
    }

    /// Resolve base URL: env var > global settings > default cloud
    pub fn base_url(&self) -> String {
        std::env::var("LETTA_BASE_URL")
            .ok()
            .or_else(|| self.global.env.letta_base_url.clone())
            .unwrap_or_else(|| "https://api.letta.com".to_string())
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

    pub fn global(&self) -> &GlobalSettings { &self.global }
    pub fn local(&self) -> &LocalSettings { &self.local }
}
