//! Persistent prompt stash ring (Section J).
//!
//! Saves in-progress, unsubmitted prompt editor text to `~/.cade/prompt_stash.json`
//! so stashes survive session submits, accidental quits, and restarts.

use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

/// Persistent store holding prompt text buffer snapshots.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct PromptStashStore {
    pub stashes: Vec<String>,
    #[serde(skip)]
    path: Option<PathBuf>,
}

impl PromptStashStore {
    /// Load the prompt stash store from ~/.cade/prompt_stash.json or fallback.
    pub fn load_default() -> Self {
        let path = dirs::home_dir()
            .map(|h| h.join(".cade").join("prompt_stash.json"))
            .unwrap_or_else(|| PathBuf::from(".cade/prompt_stash.json"));
        Self::load_from_path(&path)
    }

    /// Load the prompt stash store from a specific file path.
    pub fn load_from_path(path: &Path) -> Self {
        if let Ok(content) = std::fs::read_to_string(path)
            && let Ok(mut store) = serde_json::from_str::<PromptStashStore>(&content)
        {
            store.path = Some(path.to_path_buf());
            return store;
        }
        Self {
            stashes: Vec::new(),
            path: Some(path.to_path_buf()),
        }
    }

    /// Push a new snapshot to the stash and persist to disk.
    pub fn push(&mut self, text: String) {
        if text.trim().is_empty() {
            return;
        }
        self.stashes.push(text);
        self.save();
    }

    /// Pop the most recent stash from the ring and persist to disk.
    pub fn pop(&mut self) -> Option<String> {
        let val = self.stashes.pop();
        self.save();
        val
    }

    /// Save the stash store to disk.
    pub fn save(&self) {
        if let Some(ref path) = self.path {
            if let Some(parent) = path.parent() {
                let _ = std::fs::create_dir_all(parent);
            }
            if let Ok(json) = serde_json::to_string_pretty(self) {
                let _ = std::fs::write(path, json);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_prompt_stash_roundtrip() {
        let mut store = PromptStashStore::default();
        assert_eq!(store.pop(), None);

        store.push("Hello world prompt".to_string());
        store.push("Second prompt".to_string());

        assert_eq!(store.pop().as_deref(), Some("Second prompt"));
        assert_eq!(store.pop().as_deref(), Some("Hello world prompt"));
        assert_eq!(store.pop(), None);
    }
}
