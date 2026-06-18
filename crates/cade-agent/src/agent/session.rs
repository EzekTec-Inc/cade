use crate::Result;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

/// Persisted conversation session metadata.
///
/// Tracks the active agent, conversation, and background run state so the
/// CLI can resume across restarts without losing context.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct Session {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub agent_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub conversation_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub agent_name: Option<String>,
    /// Last background run ID — used to resume an interrupted stream
    #[serde(skip_serializing_if = "Option::is_none")]
    pub run_id: Option<String>,
    /// Last received seq_id for the active run
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_seq_id: Option<i64>,
}

/// Manages load/save of a [`Session`] to a JSON file on disk.
pub struct SessionStore {
    path: PathBuf,
    pub session: Session,
}

/// Ensure `entry` appears as a line in the given `.gitignore` file.
/// Creates the file if it doesn't exist; appends the entry if missing.
fn ensure_gitignore_entry(gitignore: &Path, entry: &str) -> std::io::Result<()> {
    if gitignore.exists() {
        let content = std::fs::read_to_string(gitignore)?;
        if content.lines().any(|l| l.trim() == entry) {
            return Ok(());
        }
        // Append with a leading newline if the file doesn't end with one
        let prefix = if content.ends_with('\n') { "" } else { "\n" };
        std::fs::write(gitignore, format!("{content}{prefix}{entry}\n"))?;
    } else {
        std::fs::write(gitignore, format!("{entry}\n"))?;
    }
    Ok(())
}

impl SessionStore {
    /// The canonical session file name (new location).
    const FILENAME: &'static str = "session.json";
    /// Legacy file that used to hold session fields (pre-migration).
    const LEGACY_FILENAME: &'static str = "settings.local.json";

    pub fn load(cwd: &Path) -> Self {
        let cade_dir = cwd.join(".cade");
        let path = cade_dir.join(Self::FILENAME);

        // Try the new file first
        let session = if path.exists() {
            std::fs::read_to_string(&path)
                .ok()
                .and_then(|s| serde_json::from_str(&s).ok())
                .unwrap_or_default()
        } else {
            // Backward-compat migration: read session fields from the legacy file
            let legacy = cade_dir.join(Self::LEGACY_FILENAME);
            if legacy.exists() {
                std::fs::read_to_string(&legacy)
                    .ok()
                    .and_then(|s| serde_json::from_str(&s).ok())
                    .unwrap_or_default()
            } else {
                Session::default()
            }
        };
        Self { path, session }
    }

    pub fn save(&self) -> Result<()> {
        if let Some(parent) = self.path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        // Ensure session.json is gitignored
        let gitignore = match self.path.parent() {
            Some(p) => p.join(".gitignore"),
            None => return Ok(()), // no parent dir — skip gitignore step
        };
        ensure_gitignore_entry(&gitignore, Self::FILENAME)?;
        let content = serde_json::to_string_pretty(&self.session)?;
        std::fs::write(&self.path, content)?;
        Ok(())
    }

    pub fn set_agent(&mut self, agent_id: String, agent_name: Option<String>) -> Result<()> {
        self.session.agent_id = Some(agent_id);
        self.session.agent_name = agent_name;
        self.session.conversation_id = None; // reset conversation when agent changes
        self.save()
    }

    pub fn set_conversation(&mut self, conversation_id: Option<String>) -> Result<()> {
        self.session.conversation_id = conversation_id;
        self.save()
    }

    pub fn set_run(&mut self, run_id: Option<String>, last_seq_id: Option<i64>) -> Result<()> {
        self.session.run_id = run_id;
        self.session.last_seq_id = last_seq_id;
        self.save()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn session_store_uses_session_json_not_settings_local() {
        let tmp = TempDir::new().unwrap();
        let store = SessionStore::load(tmp.path());
        assert!(
            store.path.ends_with(".cade/session.json"),
            "expected session.json, got {:?}",
            store.path
        );
    }

    #[test]
    fn save_does_not_touch_settings_local_json() {
        let tmp = TempDir::new().unwrap();
        let cade_dir = tmp.path().join(".cade");
        std::fs::create_dir_all(&cade_dir).unwrap();
        // Pre-create settings.local.json with known content
        let settings_path = cade_dir.join("settings.local.json");
        std::fs::write(&settings_path, r#"{"last_agent":"agent-keep-me"}"#).unwrap();

        let mut store = SessionStore::load(tmp.path());
        store
            .set_agent("agent-new".to_string(), Some("Test".to_string()))
            .unwrap();

        // settings.local.json must be untouched
        let settings_content = std::fs::read_to_string(&settings_path).unwrap();
        assert!(
            settings_content.contains("agent-keep-me"),
            "settings.local.json was overwritten: {settings_content}"
        );
        // session.json must exist with the agent
        let session_path = cade_dir.join("session.json");
        let session_content = std::fs::read_to_string(&session_path).unwrap();
        assert!(session_content.contains("agent-new"));
    }

    #[test]
    fn migrates_agent_id_from_old_settings_local() {
        let tmp = TempDir::new().unwrap();
        let cade_dir = tmp.path().join(".cade");
        std::fs::create_dir_all(&cade_dir).unwrap();
        // Simulate old format: agent_id lives in settings.local.json
        std::fs::write(
            cade_dir.join("settings.local.json"),
            r#"{"agent_id":"agent-old","agent_name":"OldAgent","conversation_id":"conv-1"}"#,
        )
        .unwrap();
        // session.json does NOT exist yet
        assert!(!cade_dir.join("session.json").exists());

        let store = SessionStore::load(tmp.path());
        assert_eq!(store.session.agent_id.as_deref(), Some("agent-old"));
        assert_eq!(store.session.agent_name.as_deref(), Some("OldAgent"));
        assert_eq!(store.session.conversation_id.as_deref(), Some("conv-1"));
    }

    #[test]
    fn session_json_included_in_gitignore() {
        let tmp = TempDir::new().unwrap();
        let mut store = SessionStore::load(tmp.path());
        store.set_agent("agent-x".to_string(), None).unwrap();
        let gitignore = tmp.path().join(".cade").join(".gitignore");
        let content = std::fs::read_to_string(&gitignore).unwrap();
        assert!(
            content.contains("session.json"),
            ".gitignore missing session.json: {content}"
        );
    }

    /// Integration test: SessionStore and SettingsManager coexist without data loss.
    /// SessionStore writes to session.json, SettingsManager writes to settings.local.json.
    /// Neither store should clobber the other's file.
    #[test]
    fn dual_store_coexistence_no_data_loss() {
        use cade_core::settings::SettingsManager;

        let tmp = TempDir::new().unwrap();
        let cade_dir = tmp.path().join(".cade");
        std::fs::create_dir_all(&cade_dir).unwrap();

        // 1. SettingsManager writes local settings (last_agent, pinned_agents)
        let mut sm = SettingsManager::new(tmp.path()).unwrap();
        sm.set_last_agent("agent-settings-1").unwrap();
        sm.pin_agent("agent-settings-1", "SettingsAgent").unwrap();

        // 2. SessionStore writes session data
        let mut ss = SessionStore::load(tmp.path());
        ss.set_agent(
            "agent-session-1".to_string(),
            Some("SessionAgent".to_string()),
        )
        .unwrap();
        ss.set_conversation(Some("conv-1".to_string())).unwrap();
        ss.set_run(Some("run-1".to_string()), Some(99)).unwrap();

        // 3. Verify settings.local.json still has SettingsManager data
        let settings_raw = std::fs::read_to_string(cade_dir.join("settings.local.json")).unwrap();
        assert!(
            settings_raw.contains("agent-settings-1"),
            "settings.local.json lost last_agent after SessionStore write: {settings_raw}"
        );
        assert!(
            settings_raw.contains("SettingsAgent"),
            "settings.local.json lost pinned_agents after SessionStore write: {settings_raw}"
        );
        // settings.local.json must NOT contain session fields
        assert!(
            !settings_raw.contains("agent-session-1"),
            "settings.local.json was contaminated with session data: {settings_raw}"
        );

        // 4. Verify session.json has SessionStore data
        let session_raw = std::fs::read_to_string(cade_dir.join("session.json")).unwrap();
        assert!(
            session_raw.contains("agent-session-1"),
            "session.json lost agent_id: {session_raw}"
        );
        assert!(
            session_raw.contains("conv-1"),
            "session.json lost conversation_id: {session_raw}"
        );
        assert!(
            session_raw.contains("run-1"),
            "session.json lost run_id: {session_raw}"
        );
        // session.json must NOT contain settings fields
        assert!(
            !session_raw.contains("agent-settings-1"),
            "session.json was contaminated with settings data: {session_raw}"
        );

        // 5. SettingsManager writes again — session.json must survive
        sm.set_last_agent("agent-settings-2").unwrap();
        let session_after = std::fs::read_to_string(cade_dir.join("session.json")).unwrap();
        assert!(
            session_after.contains("agent-session-1"),
            "session.json lost data after SettingsManager re-save: {session_after}"
        );

        // 6. SessionStore writes again — settings.local.json must survive
        ss.set_conversation(Some("conv-2".to_string())).unwrap();
        let settings_after = std::fs::read_to_string(cade_dir.join("settings.local.json")).unwrap();
        assert!(
            settings_after.contains("agent-settings-2"),
            "settings.local.json lost data after SessionStore re-save: {settings_after}"
        );

        // 7. Reload both stores and verify data integrity
        let sm2 = SettingsManager::new(tmp.path()).unwrap();
        assert_eq!(sm2.last_agent(), Some("agent-settings-2"));
        assert_eq!(sm2.pinned_agents().len(), 1);
        assert_eq!(sm2.pinned_agents()[0].name, "SettingsAgent");

        let ss2 = SessionStore::load(tmp.path());
        assert_eq!(ss2.session.agent_id.as_deref(), Some("agent-session-1"));
        assert_eq!(ss2.session.agent_name.as_deref(), Some("SessionAgent"));
        assert_eq!(ss2.session.conversation_id.as_deref(), Some("conv-2"));
        assert_eq!(ss2.session.run_id.as_deref(), Some("run-1"));
        assert_eq!(ss2.session.last_seq_id, Some(99));
    }

    #[test]
    fn roundtrip_save_and_load() {
        let tmp = TempDir::new().unwrap();
        let mut store = SessionStore::load(tmp.path());
        store
            .set_agent("agent-rt".to_string(), Some("RoundTrip".to_string()))
            .unwrap();
        store.set_conversation(Some("conv-rt".to_string())).unwrap();
        store.set_run(Some("run-rt".to_string()), Some(42)).unwrap();

        let store2 = SessionStore::load(tmp.path());
        assert_eq!(store2.session.agent_id.as_deref(), Some("agent-rt"));
        assert_eq!(store2.session.agent_name.as_deref(), Some("RoundTrip"));
        assert_eq!(store2.session.conversation_id.as_deref(), Some("conv-rt"));
        assert_eq!(store2.session.run_id.as_deref(), Some("run-rt"));
        assert_eq!(store2.session.last_seq_id, Some(42));
    }
}
