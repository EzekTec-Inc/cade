//! Command frecency tracker.
//!
//! Stores usage counts and timestamps for commands, computing frecency scores
//! with logarithmic age decay: score = uses / ln(age_in_hours + 2).

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

/// Tracking record for a single command invocation history.
#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq)]
pub struct CommandUsage {
    /// Number of times the command has been selected.
    pub uses: u32,
    /// Unix timestamp in seconds of the last invocation.
    pub last_used_timestamp: u64,
}

impl CommandUsage {
    /// Calculate frecency score using logarithmic age decay.
    pub fn score(&self, now: u64) -> f64 {
        if self.uses == 0 {
            return 0.0;
        }
        let age_seconds = now.saturating_sub(self.last_used_timestamp);
        let age_hours = (age_seconds as f64) / 3600.0;
        // score = uses / ln(age_hours + 2.0)
        (self.uses as f64) / (age_hours + 2.0).ln()
    }
}

/// Persistent frecency store mapping command triggers to usage statistics.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct FrecencyStore {
    pub commands: HashMap<String, CommandUsage>,
    #[serde(skip)]
    path: Option<PathBuf>,
}

impl FrecencyStore {
    /// Load the frecency store from ~/.cade/palette_frecency.json or fallback.
    pub fn load_default() -> Self {
        let path = dirs::home_dir()
            .map(|h| h.join(".cade").join("palette_frecency.json"))
            .unwrap_or_else(|| PathBuf::from(".cade/palette_frecency.json"));
        Self::load_from_path(&path)
    }

    /// Load the frecency store from a specific file path.
    pub fn load_from_path(path: &Path) -> Self {
        if let Ok(content) = std::fs::read_to_string(path)
            && let Ok(mut store) = serde_json::from_str::<FrecencyStore>(&content)
        {
            store.path = Some(path.to_path_buf());
            return store;
        }
        Self {
            commands: HashMap::new(),
            path: Some(path.to_path_buf()),
        }
    }

    /// Record an invocation of a command.
    pub fn record_use(&mut self, cmd: &str) {
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0);
        let entry = self.commands.entry(cmd.to_string()).or_default();
        entry.uses += 1;
        entry.last_used_timestamp = now;
        self.save();
    }

    /// Save the frecency store to disk.
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

    /// Get frecency bonus score for a command to add to fuzzy ranking.
    pub fn frecency_bonus(&self, cmd: &str) -> i32 {
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0);
        if let Some(usage) = self.commands.get(cmd) {
            let s = usage.score(now);
            (s * 100.0).round() as i32
        } else {
            0
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_frecency_score_decay() {
        let now = 1_000_000u64;
        let recent_usage = CommandUsage {
            uses: 5,
            last_used_timestamp: now, // 0 hours ago
        };
        let old_usage = CommandUsage {
            uses: 5,
            last_used_timestamp: now - (24 * 3600), // 24 hours ago
        };

        let score_recent = recent_usage.score(now);
        let score_old = old_usage.score(now);

        assert!(score_recent > score_old);
        assert!(score_recent > 0.0);
    }

    #[test]
    fn test_frecency_store_record_and_bonus() {
        let mut store = FrecencyStore::default();
        assert_eq!(store.frecency_bonus("quote"), 0);

        store.record_use("quote");
        assert!(store.frecency_bonus("quote") > 0);

        store.record_use("quote");
        assert!(store.frecency_bonus("quote") > 200);
    }
}
