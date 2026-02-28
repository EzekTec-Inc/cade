use anyhow::Result;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct Session {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub agent_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub conversation_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub agent_name: Option<String>,
}

pub struct SessionStore {
    path: PathBuf,
    pub session: Session,
}

impl SessionStore {
    pub fn load(cwd: &Path) -> Self {
        let path = cwd.join(".cade").join("settings.local.json");
        let session = if path.exists() {
            std::fs::read_to_string(&path)
                .ok()
                .and_then(|s| serde_json::from_str(&s).ok())
                .unwrap_or_default()
        } else {
            Session::default()
        };
        Self { path, session }
    }

    pub fn save(&self) -> Result<()> {
        if let Some(parent) = self.path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        // Ensure .cade/settings.local.json is gitignored
        let gitignore = self.path.parent().unwrap().join(".gitignore");
        if !gitignore.exists() {
            std::fs::write(&gitignore, "settings.local.json\n")?;
        }
        let content = serde_json::to_string_pretty(&self.session)?;
        std::fs::write(&self.path, content)?;
        Ok(())
    }

    pub fn set_agent(&mut self, agent_id: String, agent_name: Option<String>) -> Result<()> {
        self.session.agent_id = Some(agent_id);
        self.session.agent_name = agent_name;
        self.save()
    }
}
