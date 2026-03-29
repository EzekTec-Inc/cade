/// Git-based checkpointing: stash working-tree state before destructive turns.
///
/// When `create_git_checkpoint` is called:
/// 1. If the working tree is dirty, creates a stash labelled "cade-cp-<label>" and returns the stash ref.
/// 2. Records the current HEAD commit hash.
/// 3. Returns (stash_ref, commit_hash) — either or both may be None.
///
/// When `restore_git_checkpoint` is called with a stash ref, it pops that stash.
use std::path::Path;

use crate::Result;

// region:    --- Types

/// Result of creating a git checkpoint.
#[derive(Debug, Clone)]
pub struct GitCheckpoint {
    /// The git stash ref (e.g. "stash@{0}") if dirty state was stashed.
    pub stash_ref: Option<String>,
    /// HEAD commit hash at checkpoint time.
    pub commit_hash: Option<String>,
}

// endregion: --- Types

// region:    --- Public API

/// Create a git checkpoint for the working directory.
///
/// Returns `None` if the directory is not in a git repo.
pub async fn create_git_checkpoint(label: &str, cwd: &Path) -> Option<GitCheckpoint> {
    if !is_git_repo(cwd).await {
        return None;
    }

    let commit_hash = current_git_hash(cwd).await;
    let is_dirty = working_tree_dirty(cwd).await;

    let stash_ref = if is_dirty {
        create_stash(label, cwd).await
    } else {
        None
    };

    Some(GitCheckpoint {
        stash_ref,
        commit_hash,
    })
}

/// Restore a git checkpoint by popping the stash ref.
///
/// Returns an error message on failure; Ok(()) if the stash applied cleanly or
/// if `stash_ref` is None (no-op).
pub async fn restore_git_checkpoint(stash_ref: &str, cwd: &Path) -> Result<()> {
    if stash_ref.is_empty() {
        return Ok(());
    }
    let out = run_git(cwd, &["stash", "apply", stash_ref]).await;
    match out {
        Some((exit, _, stderr)) if exit != 0 => Err(crate::Error::custom(format!(
            "git stash apply failed: {stderr}"
        ))),
        None => Err(crate::Error::custom("git not found or failed to run")),
        _ => Ok(()),
    }
}

/// Delete a git checkpoint by dropping the stash ref.
///
/// Returns an error message on failure; Ok(()) if dropped cleanly or
/// if `stash_ref` is None (no-op).
pub async fn delete_git_checkpoint(stash_ref: &str, cwd: &Path) -> Result<()> {
    if stash_ref.is_empty() {
        return Ok(());
    }
    let out = run_git(cwd, &["stash", "drop", stash_ref]).await;
    match out {
        Some((exit, _, stderr)) if exit != 0 => Err(crate::Error::custom(format!(
            "git stash drop failed: {stderr}"
        ))),
        None => Err(crate::Error::custom("git not found or failed to run")),
        _ => Ok(()),
    }
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

async fn create_stash(label: &str, cwd: &Path) -> Option<String> {
    let msg = format!("cade-cp-{label}");
    let (exit, _, _) = run_git(cwd, &["stash", "push", "-u", "-m", &msg]).await?;
    if exit != 0 {
        return None;
    }
    // Get the stash ref — it's always stash@{0} immediately after push
    let (exit2, stdout, _) = run_git(cwd, &["stash", "list", "--format=%gd", "-1"]).await?;
    if exit2 == 0 {
        let r = stdout.trim().to_string();
        if r.is_empty() { None } else { Some(r) }
    } else {
        Some("stash@{0}".to_string())
    }
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
