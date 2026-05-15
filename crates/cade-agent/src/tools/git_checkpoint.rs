/// Git-based checkpointing: commit working-tree state before destructive turns.
///
/// When `create_git_checkpoint` is called:
/// 1. If the working tree is dirty, commits with label "cade-cp-<label>".
/// 2. Records the current HEAD commit hash.
/// 3. Returns the `GitCheckpoint` with the `commit_hash`.
///
/// When `restore_git_checkpoint` is called with a commit hash, it resets to that commit.
use std::path::Path;

use crate::Result;

// region:    --- Types

/// Result of creating a git checkpoint.
#[derive(Debug, Clone)]
pub struct GitCheckpoint {
    /// HEAD commit hash at checkpoint time.
    pub commit_hash: Option<String>,
}

// endregion: --- Types

// region:    --- Public API

/// Create a git checkpoint for the working directory by committing dirty state.
///
/// Returns `None` if the directory is not in a git repo.
pub async fn create_git_checkpoint(label: &str, cwd: &Path) -> Option<GitCheckpoint> {
    if !is_git_repo(cwd).await {
        return None;
    }

    let is_dirty = working_tree_dirty(cwd).await;

    if is_dirty {
        let msg = format!("cade-cp-{label}");
        let _ = run_git(cwd, &["add", "-A"]).await;
        let _ = run_git(cwd, &["commit", "-m", &msg]).await;
    }

    let commit_hash = current_git_hash(cwd).await;

    Some(GitCheckpoint { commit_hash })
}

/// Restore a git checkpoint by hard resetting to the commit hash.
///
/// Returns an error message on failure; Ok(()) if the reset was successful.
pub async fn restore_git_checkpoint(commit_hash: &str, cwd: &Path) -> Result<()> {
    if commit_hash.is_empty() {
        return Ok(());
    }
    let out = run_git(cwd, &["reset", "--hard", commit_hash]).await;
    match out {
        Some((exit, _, stderr)) if exit != 0 => {
            Err(crate::Error::custom(format!("git reset failed: {stderr}")))
        }
        None => Err(crate::Error::custom("git not found or failed to run")),
        _ => Ok(()),
    }
}

/// Delete a git checkpoint. Since checkpoints are now commits, we do not delete them to preserve history.
///
/// Returns Ok(()).
pub async fn delete_git_checkpoint(_commit_hash: &str, _cwd: &Path) -> Result<()> {
    Ok(())
}

/// Get the current HEAD commit hash, if inside a git repo.
pub async fn current_git_hash(cwd: &Path) -> Option<String> {
    let (exit, stdout, _) = run_git(cwd, &["rev-parse", "HEAD"]).await?;
    if exit == 0 {
        Some(stdout.trim().to_string())
    } else {
        None
    }
}

// endregion: --- Public API

// region:    --- Support

async fn is_git_repo(cwd: &Path) -> bool {
    matches!(
        run_git(cwd, &["rev-parse", "--git-dir"]).await,
        Some((0, _, _))
    )
}

async fn working_tree_dirty(cwd: &Path) -> bool {
    // `git status --porcelain` outputs nothing when tree is clean
    matches!(run_git(cwd, &["status", "--porcelain"]).await, Some((0, ref out, _)) if !out.trim().is_empty())
}

/// Run a git subcommand in `cwd`, returning (exit_code, stdout, stderr).
async fn run_git(cwd: &Path, args: &[&str]) -> Option<(i32, String, String)> {
    let mut cmd = tokio::process::Command::new("git");
    cade_core::agent_env::apply_agent_env(&mut cmd);
    cmd.args(args).current_dir(cwd);
    let out = cmd.output().await.ok()?;
    let exit = out.status.code().unwrap_or(-1);
    let stdout = String::from_utf8_lossy(&out.stdout).to_string();
    let stderr = String::from_utf8_lossy(&out.stderr).to_string();
    Some((exit, stdout, stderr))
}

// endregion: --- Support

// region:    --- Tests

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_current_git_hash_in_repo() {
        // -- Setup & Fixtures
        let cwd = std::env::current_dir().unwrap();

        // -- Exec
        let hash = current_git_hash(&cwd).await;

        // -- Check — we're inside CADE's own git repo so there should be a hash
        assert!(hash.is_some(), "should have a git hash");
        let h = hash.unwrap();
        assert!(h.len() >= 40, "hash should be at least 40 chars: '{h}'");
    }

    #[tokio::test]
    async fn test_is_git_repo_true() {
        // -- Exec & Check
        assert!(is_git_repo(&std::env::current_dir().unwrap()).await);
    }

    #[tokio::test]
    async fn test_is_git_repo_false() {
        // -- Exec & Check
        assert!(!is_git_repo(std::path::Path::new("/tmp")).await);
    }
}

// endregion: --- Tests
