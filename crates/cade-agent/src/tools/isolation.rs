use std::io;
use std::path::{Path, PathBuf};

/// RAII-managed isolated temporary workspace for concurrent execution.
/// Clones files from the primary workspace (respecting .gitignore / standard ignore rules),
/// and supports safely merging modified files back with global file lock coordination.
pub struct IsolatedWorkspace {
    temp_dir: tempfile::TempDir,
    primary_dir: PathBuf,
    git_branch: Option<String>,
}

impl IsolatedWorkspace {
    /// Create a sandboxed temporary clone of the primary workspace.
    /// Uses standard ignore walking to skip ignored folders (e.g. target, node_modules).
    pub fn clone_from(primary: &Path) -> io::Result<Self> {
        let tmp = tempfile::tempdir()?;
        let walker = ignore::WalkBuilder::new(primary)
            .standard_filters(true)
            .hidden(false)
            .build();

        for entry in walker.flatten() {
            let path = entry.path();
            if path.is_file()
                && let Ok(rel_path) = path.strip_prefix(primary)
            {
                let dest_path = tmp.path().join(rel_path);
                if let Some(parent) = dest_path.parent() {
                    std::fs::create_dir_all(parent)?;
                }
                std::fs::copy(path, dest_path)?;
            }
        }

        Ok(Self {
            temp_dir: tmp,
            primary_dir: primary.to_path_buf(),
            git_branch: None,
        })
    }

    /// Enable Git branch sandboxing. Spawns an isolated git branch with the specified name
    /// from the current HEAD of the primary workspace, and sets up the temporary folder to track it.
    pub async fn with_git_branch(mut self, branch_name: &str) -> Self {
        let is_git = match tokio::process::Command::new("git")
            .args(["rev-parse", "--git-dir"])
            .current_dir(&self.primary_dir)
            .output()
            .await
        {
            Ok(out) => out.status.success(),
            Err(_) => false,
        };

        if is_git {
            let temp_path = self.temp_dir.path();

            // Setup git repo in temporary sandbox folder
            let mut success = true;
            if run_cmd(temp_path, &["init"]).await.is_err() {
                success = false;
            }
            if run_cmd(temp_path, &["config", "user.name", "CADE Subagent"])
                .await
                .is_err()
            {
                success = false;
            }
            if run_cmd(temp_path, &["config", "user.email", "subagent@cade.ai"])
                .await
                .is_err()
            {
                success = false;
            }
            if run_cmd(temp_path, &["checkout", "-b", branch_name])
                .await
                .is_err()
            {
                success = false;
            }
            if run_cmd(temp_path, &["add", "-A"]).await.is_err() {
                success = false;
            }
            if run_cmd(temp_path, &["commit", "-m", "Initial sandboxed state"])
                .await
                .is_err()
            {
                success = false;
            }

            if success {
                self.git_branch = Some(branch_name.to_string());
            }
        }

        self
    }

    /// Retrieve the absolute path to the temporary sandboxed workspace.
    pub fn path(&self) -> &Path {
        self.temp_dir.path()
    }

    /// Retrieve the absolute path to the primary/host workspace.
    pub fn primary_path(&self) -> &Path {
        &self.primary_dir
    }

    /// Scan the temporary directory and safely copy all modified or new files
    /// back to the primary workspace, acquiring exclusive file locks dynamically
    /// to prevent concurrent write collisions.
    pub async fn merge_back(&self) -> io::Result<()> {
        if let Some(ref branch) = self.git_branch {
            let temp_path = self.temp_dir.path();

            // 1. In sandbox: check if there are changes and commit them
            let (status_exit, status_out, _) =
                run_cmd(temp_path, &["status", "--porcelain"]).await?;
            if status_exit == 0 && !status_out.trim().is_empty() {
                run_cmd(temp_path, &["add", "-A"]).await?;
                run_cmd(
                    temp_path,
                    &["commit", "-m", "Subagent task completion changes"],
                )
                .await?;
            }

            // 2. In primary workspace: fetch the sandboxed branch and merge it
            let remote_name = format!("sandbox-{}", branch);
            let temp_path_str = temp_path.to_string_lossy().to_string();

            // Add temp sandbox as remote
            let _ = run_cmd(
                &self.primary_dir,
                &["remote", "add", &remote_name, &temp_path_str],
            )
            .await;

            // Fetch from sandbox
            let (fetch_exit, _, fetch_err) =
                run_cmd(&self.primary_dir, &["fetch", &remote_name, branch]).await?;
            if fetch_exit != 0 {
                let _ = run_cmd(&self.primary_dir, &["remote", "remove", &remote_name]).await;
                return Err(io::Error::other(format!(
                    "git fetch from sandbox failed: {fetch_err}"
                )));
            }

            // Merge fetched branch (from FETCH_HEAD)
            let (merge_exit, _, merge_err) =
                run_cmd(&self.primary_dir, &["merge", "FETCH_HEAD", "--no-edit"]).await?;

            // Clean up remote
            let _ = run_cmd(&self.primary_dir, &["remote", "remove", &remote_name]).await;

            if merge_exit != 0 {
                return Err(io::Error::other(format!(
                    "git merge conflict occurred: {merge_err}. Please resolve manually or accept-edits."
                )));
            }

            tracing::info!(
                "Git Branch Sandboxing merged changes back from branch: {}",
                branch
            );
            Ok(())
        } else {
            let temp_path = self.temp_dir.path();
            let walker = ignore::WalkBuilder::new(temp_path)
                .standard_filters(true)
                .hidden(false)
                .build();

            for entry in walker.flatten() {
                let path = entry.path();
                if path.is_file()
                    && let Ok(rel_path) = path.strip_prefix(temp_path)
                {
                    let dest_path = self.primary_dir.join(rel_path);

                    // Check if file content differs or does not exist
                    let temp_bytes = std::fs::read(path)?;
                    let host_bytes_opt = std::fs::read(&dest_path).ok();

                    if host_bytes_opt.is_none() || host_bytes_opt.unwrap() != temp_bytes {
                        if let Some(parent) = dest_path.parent() {
                            std::fs::create_dir_all(parent)?;
                        }

                        // Acquire global file lock during final merge step (ADR 6)
                        let lock_manager = crate::tools::file_lock::FileLockManager::global();
                        let _lock = lock_manager.acquire_lock(&dest_path).await;

                        std::fs::write(&dest_path, &temp_bytes)?;
                        tracing::info!("Workspace Isolation merged file back: {:?}", rel_path);
                    }
                }
            }
            Ok(())
        }
    }
}

async fn run_cmd(dir: &Path, args: &[&str]) -> io::Result<(i32, String, String)> {
    let mut cmd = tokio::process::Command::new("git");
    cmd.args(args).current_dir(dir);
    let out = cmd.output().await?;
    let exit = out.status.code().unwrap_or(-1);
    let stdout = String::from_utf8_lossy(&out.stdout).to_string();
    let stderr = String::from_utf8_lossy(&out.stderr).to_string();
    Ok((exit, stdout, stderr))
}
