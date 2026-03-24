/// Context file discovery: AGENTS.md, CLAUDE.md, CADE.md
///
/// pi looks for AGENTS.md walking up from cwd to git root.
/// Claude Code looks for CLAUDE.md.
/// We support all three, concatenating them in order (global first,
/// then walking upward from cwd so more specific context wins).
use std::path::{Path, PathBuf};

// region:    --- Types

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ContextScope {
    /// From the agent config directory (e.g. ~/.cade/AGENTS.md).
    Global,
    /// From somewhere in the project directory tree.
    Project,
}

#[derive(Debug, Clone)]
pub struct ContextFile {
    pub path:    PathBuf,
    pub content: String,
    pub scope:   ContextScope,
}

// endregion: --- Types

// region:    --- Discovery

/// File names recognised as context files, in priority order.
const CONTEXT_FILENAMES: &[&str] = &["AGENTS.md", "CLAUDE.md", "CADE.md"];

/// Discover all context files and return them concatenation-ready.
///
/// Order: global (agent_dir) → project files from git root down to cwd.
/// More-specific files (closer to cwd) therefore appear later and can
/// override or extend global instructions.
pub fn discover_context_files(cwd: &Path, agent_dir: &Path) -> Vec<ContextFile> {
    let mut files = Vec::new();

    // -- Global: agent config directory
    for name in CONTEXT_FILENAMES {
        let p = agent_dir.join(name);
        if let Ok(content) = std::fs::read_to_string(&p) {
            files.push(ContextFile { path: p, content, scope: ContextScope::Global });
        }
    }

    // -- Project: walk from git root (or filesystem root) down to cwd,
    //    collecting context files along the path so more-specific wins.
    let ancestors = collect_project_ancestors(cwd);
    for ancestor in ancestors {
        for name in CONTEXT_FILENAMES {
            let p = ancestor.join(name);
            if let Ok(content) = std::fs::read_to_string(&p) {
                // Avoid re-loading global file if agent_dir == cwd ancestor
                let already = files.iter().any(|f| f.path == p);
                if !already {
                    files.push(ContextFile { path: p, content, scope: ContextScope::Project });
                }
            }
        }
    }

    files
}

/// Build the combined system-prompt snippet from a list of context files.
/// Each file is separated by a divider showing its origin path.
pub fn build_context_block(files: &[ContextFile]) -> String {
    if files.is_empty() {
        return String::new();
    }
    let mut out = String::new();
    for f in files {
        out.push_str(&format!("\n\n---\n<!-- context: {} -->\n\n{}", f.path.display(), f.content.trim()));
    }
    out
}

// endregion: --- Discovery

// region:    --- Support

/// Return the list of directories from the git root (or filesystem root)
/// down to and including `cwd`, ordered root-first so callers can iterate
/// from least-specific to most-specific.
fn collect_project_ancestors(cwd: &Path) -> Vec<PathBuf> {
    let git_root = find_git_root(cwd);

    // Collect all ancestors from cwd up to (and including) the stop point.
    let stop = git_root.as_deref().unwrap_or(Path::new("/"));
    let mut chain: Vec<PathBuf> = std::iter::successors(Some(cwd.to_path_buf()), |p| {
        let parent = p.parent()?.to_path_buf();
        // Keep going as long as we haven't passed the stop point.
        if parent.starts_with(stop) || parent == stop {
            Some(parent)
        } else {
            None
        }
    })
    .collect();

    // Ensure stop point itself is included.
    if let Some(root) = &git_root
        && !chain.contains(root) {
            chain.push(root.clone());
        }

    // Reverse so we go root → cwd (least specific → most specific).
    chain.reverse();
    chain
}

/// Find the git root for `path` by walking up looking for a `.git` directory.
fn find_git_root(path: &Path) -> Option<PathBuf> {
    let mut current = if path.is_dir() {
        path.to_path_buf()
    } else {
        path.parent()?.to_path_buf()
    };
    loop {
        if current.join(".git").exists() {
            return Some(current);
        }
        let parent = current.parent()?.to_path_buf();
        if parent == current {
            return None; // filesystem root
        }
        current = parent;
    }
}

// endregion: --- Support

// region:    --- Tests

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::TempDir;

    fn make_dir() -> TempDir {
        tempfile::tempdir().expect("tempdir")
    }

    #[test]
    fn test_discover_context_files_global() {
        // -- Setup & Fixtures
        let agent_dir = make_dir();
        let cwd = make_dir();
        fs::write(agent_dir.path().join("AGENTS.md"), "global instructions").unwrap();

        // -- Exec
        let files = discover_context_files(cwd.path(), agent_dir.path());

        // -- Check
        assert_eq!(files.len(), 1);
        assert_eq!(files[0].scope, ContextScope::Global);
        assert!(files[0].content.contains("global instructions"));
    }

    #[test]
    fn test_discover_context_files_project() {
        // -- Setup & Fixtures
        let agent_dir = make_dir();
        let cwd = make_dir();
        fs::write(cwd.path().join("AGENTS.md"), "project instructions").unwrap();

        // -- Exec
        let files = discover_context_files(cwd.path(), agent_dir.path());

        // -- Check
        assert!(files.iter().any(|f| f.content.contains("project instructions")));
    }

    #[test]
    fn test_discover_context_files_empty() {
        // -- Setup & Fixtures
        let agent_dir = make_dir();
        let cwd = make_dir();

        // -- Exec
        let files = discover_context_files(cwd.path(), agent_dir.path());

        // -- Check
        assert!(files.is_empty());
    }

    #[test]
    fn test_build_context_block_empty() {
        // -- Exec & Check
        assert!(build_context_block(&[]).is_empty());
    }

    #[test]
    fn test_build_context_block_nonempty() {
        // -- Setup & Fixtures
        let file = ContextFile {
            path:    PathBuf::from("/fake/AGENTS.md"),
            content: "some content".to_string(),
            scope:   ContextScope::Global,
        };

        // -- Exec
        let block = build_context_block(&[file]);

        // -- Check
        assert!(block.contains("some content"));
        assert!(block.contains("AGENTS.md"));
    }
}

// endregion: --- Tests
