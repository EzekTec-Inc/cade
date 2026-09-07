//! Core Modified-Files Tracker State Engine (`ModifiedFilesTracker`).
//!
//! Maintains session-baseline snapshots of touched files, computes net line additions
//! and deletions against origin, and automatically prunes files when reverted.

// region:    --- Imports

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use similar::{ChangeTag, TextDiff};

// endregion: --- Imports

// region:    --- Types

/// Net line addition and deletion counts for a single file.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct FileDiffMetrics {
    pub additions: u32,
    pub deletions: u32,
}

/// A single modified file entry ready for UI presentation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ModifiedFileEntry {
    pub relative_path: String,
    pub metrics: FileDiffMetrics,
}

// endregion: --- Types

// region:    --- ModifiedFilesTracker

/// Deep module tracking file modifications and calculating net diffs against session baseline.
#[derive(Debug, Clone, Default)]
pub struct ModifiedFilesTracker {
    /// Initial snapshot of file content prior to its first mutation in the session.
    initial_snapshots: HashMap<PathBuf, String>,
    /// Active net diff metrics keyed by file path.
    diff_metrics: HashMap<PathBuf, FileDiffMetrics>,
}

impl ModifiedFilesTracker {
    pub fn new() -> Self {
        Self::default()
    }

    /// Reset all tracked state (used when switching sessions or resetting workspace).
    pub fn clear(&mut self) {
        self.initial_snapshots.clear();
        self.diff_metrics.clear();
    }

    /// Record a file mutation.
    ///
    /// - If the file has not been seen in this session, `pre_content` is recorded as the baseline snapshot.
    /// - Computes net line addition and deletion metrics between the baseline snapshot and `post_content`.
    /// - If `post_content == baseline` (i.e. zero additions and zero deletions), the entry is pruned.
    pub fn record_mutation(
        &mut self,
        file_path: impl Into<PathBuf>,
        pre_content: &str,
        post_content: &str,
    ) {
        let path = file_path.into();

        // 1. Establish initial baseline snapshot on first edit
        let baseline = self
            .initial_snapshots
            .entry(path.clone())
            .or_insert_with(|| pre_content.to_string());

        // 2. If content is identical to initial baseline, prune from modified list
        if baseline == post_content {
            self.diff_metrics.remove(&path);
            return;
        }

        // 3. Compute net diff against the baseline snapshot
        let metrics = Self::compute_net_metrics(baseline, post_content);
        if metrics.additions == 0 && metrics.deletions == 0 {
            self.diff_metrics.remove(&path);
        } else {
            self.diff_metrics.insert(path, metrics);
        }
    }

    /// Calculate addition and deletion line metrics between two strings.
    fn compute_net_metrics(baseline: &str, current: &str) -> FileDiffMetrics {
        let diff = TextDiff::from_lines(baseline, current);
        let mut additions = 0u32;
        let mut deletions = 0u32;

        for change in diff.iter_all_changes() {
            match change.tag() {
                ChangeTag::Insert => additions += 1,
                ChangeTag::Delete => deletions += 1,
                ChangeTag::Equal => {}
            }
        }

        FileDiffMetrics {
            additions,
            deletions,
        }
    }

    /// Return an alphabetically sorted list of modified file entries formatted relative to `working_dir`.
    pub fn entries(&self, working_dir: &Path) -> Vec<ModifiedFileEntry> {
        let mut list: Vec<ModifiedFileEntry> = self
            .diff_metrics
            .iter()
            .map(|(path, metrics)| {
                let rel = path
                    .strip_prefix(working_dir)
                    .unwrap_or(path)
                    .to_string_lossy()
                    .to_string();
                ModifiedFileEntry {
                    relative_path: rel,
                    metrics: *metrics,
                }
            })
            .collect();

        list.sort_by(|a, b| a.relative_path.cmp(&b.relative_path));
        list
    }

    /// Returns true if no files are currently modified.
    pub fn is_empty(&self) -> bool {
        self.diff_metrics.is_empty()
    }

    /// Number of currently modified files.
    pub fn len(&self) -> usize {
        self.diff_metrics.len()
    }
}

// endregion: --- ModifiedFilesTracker

// region:    --- Tests

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_first_edit_records_initial_snapshot_and_diff() {
        let mut tracker = ModifiedFilesTracker::new();
        let path = PathBuf::from("/workspace/src/main.rs");
        let base_content = "fn main() {\n    println!(\"hello\");\n}\n";
        let edited_content = "fn main() {\n    println!(\"hello\");\n    println!(\"world\");\n}\n";

        tracker.record_mutation(&path, base_content, edited_content);

        assert_eq!(tracker.len(), 1);
        let working_dir = Path::new("/workspace");
        let entries = tracker.entries(working_dir);
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].relative_path, "src/main.rs");
        assert_eq!(entries[0].metrics.additions, 1);
        assert_eq!(entries[0].metrics.deletions, 0);
    }

    #[test]
    fn test_reverting_to_initial_content_prunes_file() {
        let mut tracker = ModifiedFilesTracker::new();
        let path = PathBuf::from("/workspace/src/lib.rs");
        let base_content = "pub fn add(a: i32, b: i32) -> i32 { a + b }\n";
        let mutated = "pub fn add(a: i32, b: i32) -> i32 { a + b + 1 }\n";

        tracker.record_mutation(&path, base_content, mutated);
        assert_eq!(tracker.len(), 1);

        // Revert back to original base content
        tracker.record_mutation(&path, mutated, base_content);
        assert!(tracker.is_empty());
        assert_eq!(tracker.len(), 0);
    }

    #[test]
    fn test_multiple_edits_compute_net_diff_against_origin() {
        let mut tracker = ModifiedFilesTracker::new();
        let path = PathBuf::from("/workspace/src/config.rs");
        let v0 = "line1\nline2\nline3\n";
        let v1 = "line1\nline2_modified\nline3\nline4\n";
        let v2 = "line1\nline2_modified\nline3\nline4\nline5\n";

        tracker.record_mutation(&path, v0, v1);
        tracker.record_mutation(&path, v1, v2);

        let entries = tracker.entries(Path::new("/workspace"));
        assert_eq!(entries.len(), 1);
        // v0 -> v2: line2 changed (+1 -1), line4 added (+1), line5 added (+1) -> additions: 3, deletions: 1
        assert_eq!(entries[0].metrics.additions, 3);
        assert_eq!(entries[0].metrics.deletions, 1);
    }

    #[test]
    fn test_alphabetical_sorting_and_relative_path_display() {
        let mut tracker = ModifiedFilesTracker::new();
        let p_z = PathBuf::from("/repo/src/z_module.rs");
        let p_a = PathBuf::from("/repo/src/a_module.rs");

        tracker.record_mutation(&p_z, "a", "b");
        tracker.record_mutation(&p_a, "c", "d");

        let entries = tracker.entries(Path::new("/repo"));
        assert_eq!(entries.len(), 2);
        assert_eq!(entries[0].relative_path, "src/a_module.rs");
        assert_eq!(entries[1].relative_path, "src/z_module.rs");
    }
}

// endregion: --- Tests
