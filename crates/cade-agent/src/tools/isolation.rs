use std::collections::{BTreeMap, BTreeSet};
use std::io::{self, Write};
use std::path::{Path, PathBuf};

/// Conflicts found by baseline/child/host reconciliation, before host mutation.
/// `branch_name` is retained for compatibility with branch-enabled callers.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct MergeConflictReport {
    pub branch_name: String,
    pub conflicting_files: Vec<PathBuf>,
    pub raw_error: String,
}

impl std::fmt::Display for MergeConflictReport {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "Workspace merge conflict on '{}'. Conflicting files: [{}] Details: {}",
            self.branch_name,
            self.conflicting_files
                .iter()
                .map(|p| p.display().to_string())
                .collect::<Vec<_>>()
                .join(", "),
            self.raw_error
        )
    }
}

impl std::error::Error for MergeConflictReport {}

#[derive(Clone)]
enum MergeResult {
    Merged,
    Conflict(MergeConflictReport),
    Failed(io::ErrorKind, String),
}

impl MergeResult {
    fn result(&self) -> io::Result<()> {
        match self {
            Self::Merged => Ok(()),
            Self::Conflict(report) => Err(io::Error::other(report.clone())),
            Self::Failed(kind, message) => Err(io::Error::new(*kind, message.clone())),
        }
    }
}

struct FileSnapshot {
    bytes: Vec<u8>,
    permissions: std::fs::Permissions,
}

impl PartialEq for FileSnapshot {
    fn eq(&self, other: &Self) -> bool {
        self.bytes == other.bytes && permissions_equal(&self.permissions, &other.permissions)
    }
}

#[cfg(unix)]
fn permissions_equal(a: &std::fs::Permissions, b: &std::fs::Permissions) -> bool {
    use std::os::unix::fs::PermissionsExt;
    a.mode() == b.mode()
}

#[cfg(not(unix))]
fn permissions_equal(a: &std::fs::Permissions, b: &std::fs::Permissions) -> bool {
    a.readonly() == b.readonly()
}

/// RAII-managed isolated workspace with an immutable, on-disk launch baseline.
/// Only child deltas are reconciled. A divergent host edit/delete is a conflict,
/// never an invitation to restore an old snapshot over the parent.
pub struct IsolatedWorkspace {
    temp_dir: tempfile::TempDir,
    baseline_dir: tempfile::TempDir,
    baseline_paths: BTreeSet<PathBuf>,
    primary_dir: PathBuf,
    git_branch: Option<String>,
    merge_result: tokio::sync::Mutex<Option<MergeResult>>,
}

impl IsolatedWorkspace {
    /// Clone regular, nonignored files. Snapshot the copied bytes, rather than
    /// rereading the live host, so the baseline is exactly what the child saw.
    /// Internal file symlinks are materialized as independent file snapshots,
    /// preserving the legacy clone behavior without following external links.
    /// Directory and external symlinks are excluded.
    ///
    /// # Errors
    /// Returns traversal, read or copy failures; a partial clone is discarded.
    pub fn clone_from(primary: &Path) -> io::Result<Self> {
        let primary = primary.canonicalize()?;
        std::fs::read_dir(&primary)?;
        let tmp = tempfile::tempdir()?;
        let baseline = tempfile::tempdir()?;
        let mut paths = BTreeSet::new();
        for rel in regular_paths(&primary, true)? {
            let dest = tmp.path().join(&rel);
            let original = baseline.path().join(&rel);
            if let Some(parent) = dest.parent() {
                std::fs::create_dir_all(parent)?;
            }
            if let Some(parent) = original.parent() {
                std::fs::create_dir_all(parent)?;
            }
            std::fs::copy(primary.join(&rel), &dest)?;
            std::fs::copy(&dest, original)?;
            paths.insert(rel);
        }
        Ok(Self {
            temp_dir: tmp,
            baseline_dir: baseline,
            baseline_paths: paths,
            primary_dir: primary,
            git_branch: None,
            merge_result: tokio::sync::Mutex::new(None),
        })
    }

    /// Enable a private Git repository for child-side Git tools. Host history,
    /// index, remotes and worktree are never changed by merge-back. Both modes
    /// use the same baseline-aware reconciliation (no unrelated-history merge).
    pub async fn with_git_branch(mut self, branch_name: &str) -> Self {
        let setup = async {
            run_cmd(self.path(), &["init"]).await?;
            run_cmd(self.path(), &["config", "user.name", "CADE Subagent"]).await?;
            run_cmd(self.path(), &["config", "user.email", "subagent@cade.ai"]).await?;
            run_cmd(self.path(), &["checkout", "-b", branch_name]).await?;
            run_cmd(self.path(), &["add", "-A"]).await?;
            run_cmd(
                self.path(),
                &["commit", "--allow-empty", "-m", "Initial sandboxed state"],
            )
            .await
        }
        .await;
        match setup {
            Ok(()) => self.git_branch = Some(branch_name.to_string()),
            Err(error) => {
                tracing::warn!(%error, "private workspace Git setup failed; using snapshot reconciliation")
            }
        }
        self
    }

    /// The child's absolute working directory.
    pub fn path(&self) -> &Path {
        self.temp_dir.path()
    }

    /// The canonical host workspace root.
    pub fn primary_path(&self) -> &Path {
        &self.primary_dir
    }

    const fn merge_mutex() -> &'static tokio::sync::Mutex<()> {
        static MUTEX: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());
        &MUTEX
    }

    /// Merge once, including a failed attempt. All candidate paths are locked
    /// and preflighted before any host writes. Conflicts reject the entire delta.
    /// Writes are staged before replacement; I/O failures roll back applied
    /// files where they still match this merge's output. No await occurs during
    /// host mutation, so cancellation cannot interrupt half an applied delta.
    ///
    /// # Errors
    /// Returns a sorted [`MergeConflictReport`] or an I/O/rollback failure.
    pub async fn merge_back(&self) -> io::Result<()> {
        let mut cached = self.merge_result.lock().await;
        if let Some(result) = cached.as_ref() {
            return result.result();
        }
        let _merge_lock = Self::merge_mutex().lock().await;
        let result = match self.reconcile().await {
            Ok(()) => MergeResult::Merged,
            Err(error) => match error
                .get_ref()
                .and_then(|e| e.downcast_ref::<MergeConflictReport>())
            {
                Some(report) => MergeResult::Conflict(report.clone()),
                None => MergeResult::Failed(error.kind(), error.to_string()),
            },
        };
        let output = result.result();
        *cached = Some(result);
        output
    }

    async fn reconcile(&self) -> io::Result<()> {
        let mut paths = regular_paths(self.path(), false)?;
        // A changed .gitignore must not turn a baseline file into a deletion.
        paths.extend(self.baseline_paths.iter().cloned());
        let mut changes = BTreeMap::new();
        for rel in paths {
            if !safe_parent(self.path(), &self.path().join(&rel))? {
                return Err(io::Error::other(format!(
                    "Child changed directory topology at {}",
                    rel.display()
                )));
            }
            let baseline = read_snapshot(&self.baseline_dir.path().join(&rel))?;
            let child = read_snapshot(&self.path().join(&rel))?;
            if baseline != child {
                changes.insert(rel, (baseline, child));
            }
        }

        let manager = crate::tools::file_lock::FileLockManager::global();
        let mut locks = Vec::new();
        let mut keys = BTreeSet::new();
        for rel in changes.keys() {
            let dest = self.primary_dir.join(rel);
            if keys.insert(crate::tools::file_lock::FileLockManager::normalize_key(
                &dest,
            )) {
                locks.push(manager.acquire_lock(&dest).await);
            }
        }
        let mut conflicts = Vec::new();
        let mut plan = BTreeMap::new();
        for (rel, (baseline, child)) in changes {
            let dest = self.primary_dir.join(&rel);
            if !safe_parent(&self.primary_dir, &dest)? {
                conflicts.push(rel);
                continue;
            }
            let host = match read_snapshot(&dest) {
                Ok(host) => host,
                Err(error) if error.kind() == io::ErrorKind::InvalidInput => {
                    conflicts.push(rel);
                    continue;
                }
                Err(error) => return Err(error),
            };
            if host == child {
                continue; // Identical edits/deletions already applied by the parent.
            }
            if host != baseline {
                conflicts.push(rel);
            } else {
                plan.insert(rel, (host, child));
            }
        }
        if !conflicts.is_empty() {
            return Err(io::Error::other(MergeConflictReport {
                branch_name: self.git_branch.clone().unwrap_or_else(|| "snapshot".into()),
                conflicting_files: conflicts,
                raw_error: "Host changed since child launch; no isolated changes were applied"
                    .into(),
            }));
        }
        apply_plan(&self.primary_dir, &plan)
    }

    /// Explicitly close both snapshots, surfacing cleanup failure to the session.
    ///
    /// # Errors
    /// Returns all temporary-directory cleanup errors, rather than hiding them.
    pub fn close(self) -> io::Result<()> {
        let mut errors = Vec::new();
        if let Err(error) = self.temp_dir.close() {
            errors.push(format!("child workspace: {error}"));
        }
        if let Err(error) = self.baseline_dir.close() {
            errors.push(format!("launch baseline: {error}"));
        }
        if errors.is_empty() {
            Ok(())
        } else {
            Err(io::Error::other(errors.join("; ")))
        }
    }
}

fn regular_paths(root: &Path, allow_source_links: bool) -> io::Result<BTreeSet<PathBuf>> {
    std::fs::read_dir(root)?;
    let walker = ignore::WalkBuilder::new(root)
        .standard_filters(true)
        .require_git(false)
        .hidden(false)
        .follow_links(false)
        .filter_entry(|entry| entry.file_name() != ".git")
        .build();
    let mut paths = BTreeSet::new();
    for entry in walker {
        let entry = entry.map_err(io::Error::other)?;
        if entry.file_type().is_some_and(|kind| kind.is_symlink()) {
            if !allow_source_links {
                return Err(io::Error::other(format!(
                    "Child symlink changes are not supported: {}",
                    entry.path().display()
                )));
            }
            if let Ok(target) = entry.path().canonicalize()
                && target.starts_with(root)
                && target.is_file()
            {
                let rel = entry.path().strip_prefix(root).map_err(io::Error::other)?;
                paths.insert(rel.to_path_buf());
            }
            continue;
        }
        if entry.file_type().is_some_and(|kind| kind.is_file()) {
            let rel = entry.path().strip_prefix(root).map_err(io::Error::other)?;
            paths.insert(rel.to_path_buf());
        }
    }
    Ok(paths)
}

fn read_snapshot(path: &Path) -> io::Result<Option<FileSnapshot>> {
    let metadata = match std::fs::symlink_metadata(path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error),
    };
    if !metadata.is_file() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            format!("{} is not a regular file", path.display()),
        ));
    }
    Ok(Some(FileSnapshot {
        bytes: std::fs::read(path)?,
        permissions: metadata.permissions(),
    }))
}

// Never follow a host symlink (including the root being replaced since clone).
fn safe_parent(root: &Path, dest: &Path) -> io::Result<bool> {
    let mut parent = dest.parent();
    while let Some(path) = parent {
        match std::fs::symlink_metadata(path) {
            Ok(meta) if !meta.is_dir() || meta.file_type().is_symlink() => return Ok(false),
            Ok(_) => {}
            Err(error) if error.kind() == io::ErrorKind::NotFound => {}
            Err(error) => return Err(error),
        }
        if path == root {
            return Ok(path.exists());
        }
        parent = path.parent();
    }
    Ok(false)
}

type MergePlan = BTreeMap<PathBuf, (Option<FileSnapshot>, Option<FileSnapshot>)>;

fn stage_file(dest: &Path, file: &FileSnapshot) -> io::Result<tempfile::NamedTempFile> {
    let parent = dest
        .parent()
        .ok_or_else(|| io::Error::other("missing destination parent"))?;
    let mut staged = tempfile::NamedTempFile::new_in(parent)?;
    staged.write_all(&file.bytes)?;
    staged.as_file().set_permissions(file.permissions.clone())?;
    staged.as_file().sync_all()?;
    Ok(staged)
}

fn replace_file(dest: &Path, staged: tempfile::NamedTempFile) -> io::Result<()> {
    staged
        .persist(dest)
        .map(|_| ())
        .map_err(|error| error.error)
}

fn apply_plan(root: &Path, plan: &MergePlan) -> io::Result<()> {
    apply_plan_with(root, plan, |dest, staged| match staged {
        Some(file) => replace_file(dest, file),
        None => std::fs::remove_file(dest),
    })
}

fn apply_plan_with(
    root: &Path,
    plan: &MergePlan,
    mut apply: impl FnMut(&Path, Option<tempfile::NamedTempFile>) -> io::Result<()>,
) -> io::Result<()> {
    let mut staged = BTreeMap::new();
    let mut created_dirs = Vec::new();
    let mut applied = Vec::new();
    let result = (|| {
        for (rel, (_, child)) in plan {
            if let Some(child) = child {
                let dest = root.join(rel);
                if !safe_parent(root, &dest)? {
                    return Err(io::Error::other(format!(
                        "Host directory changed while staging {}",
                        rel.display()
                    )));
                }
                let mut missing = Vec::new();
                let mut parent = dest.parent();
                while let Some(path) = parent {
                    if path.exists() {
                        break;
                    }
                    missing.push(path.to_path_buf());
                    parent = path.parent();
                }
                for dir in missing.into_iter().rev() {
                    std::fs::create_dir(&dir)?;
                    created_dirs.push(dir);
                }
                staged.insert(rel.clone(), stage_file(&dest, child)?);
            }
        }
        // Staging can take time. Recheck the complete host delta before the
        // first replacement, including edits from uncoordinated external tools.
        for (rel, (host, _)) in plan {
            let dest = root.join(rel);
            if !safe_parent(root, &dest)? || read_snapshot(&dest)? != *host {
                return Err(io::Error::other(format!(
                    "Host changed while staging {}; no files were applied",
                    rel.display()
                )));
            }
        }
        for (rel, (host, child)) in plan {
            let dest = root.join(rel);
            if !safe_parent(root, &dest)? || read_snapshot(&dest)? != *host {
                return Err(io::Error::other(format!(
                    "Host changed before applying {}",
                    rel.display()
                )));
            }
            let staged = match child {
                Some(_) => {
                    let file = staged
                        .remove(rel)
                        .ok_or_else(|| io::Error::other("missing staged file"))?;
                    Some(file)
                }
                None => None,
            };
            apply(&dest, staged)?;
            applied.push(rel.clone());
        }
        Ok(())
    })();
    drop(staged);
    if let Err(error) = result {
        let mut failures = vec![error.to_string()];
        for rel in applied.into_iter().rev() {
            let (host, child) = &plan[&rel];
            let dest = root.join(&rel);
            let rollback = (|| {
                if !safe_parent(root, &dest)? || read_snapshot(&dest)? != *child {
                    return Err(io::Error::other(
                        "host changed after merge; rollback preserved host",
                    ));
                }
                match host {
                    Some(host) => replace_file(&dest, stage_file(&dest, host)?),
                    None => std::fs::remove_file(&dest),
                }
            })();
            if let Err(error) = rollback {
                failures.push(format!("rollback {}: {error}", rel.display()));
            }
        }
        for dir in created_dirs.into_iter().rev() {
            // Remove only empty directories created by this attempt.
            if let Err(error) = std::fs::remove_dir(&dir)
                && error.kind() != io::ErrorKind::DirectoryNotEmpty
            {
                failures.push(format!("cleanup {}: {error}", dir.display()));
            }
        }
        return Err(io::Error::other(failures.join("; ")));
    }
    Ok(())
}

async fn run_cmd(dir: &Path, args: &[&str]) -> io::Result<()> {
    let out = tokio::process::Command::new("git")
        .args(args)
        .current_dir(dir)
        .kill_on_drop(true)
        .output()
        .await?;
    if out.status.success() {
        Ok(())
    } else {
        Err(io::Error::other(format!(
            "git {}: {}",
            args.join(" "),
            String::from_utf8_lossy(&out.stderr)
        )))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn isolated_workspace_reconciles_child_delta_and_preserves_parent_edits_and_deletes()
    -> io::Result<()> {
        let host = tempfile::tempdir()?;
        for name in ["child", "parent", "parent-delete", "child-delete"] {
            std::fs::write(host.path().join(name), "baseline")?;
        }
        let child = IsolatedWorkspace::clone_from(host.path())?;
        std::fs::write(child.path().join("child"), "child edit")?;
        std::fs::write(child.path().join("new"), "child addition")?;
        std::fs::remove_file(child.path().join("child-delete"))?;
        std::fs::write(host.path().join("parent"), "parent edit")?;
        std::fs::write(host.path().join("parent-new"), "parent addition")?;
        std::fs::remove_file(host.path().join("parent-delete"))?;
        child.merge_back().await?;
        assert_eq!(
            std::fs::read_to_string(host.path().join("child"))?,
            "child edit"
        );
        assert_eq!(
            std::fs::read_to_string(host.path().join("new"))?,
            "child addition"
        );
        assert_eq!(
            std::fs::read_to_string(host.path().join("parent"))?,
            "parent edit"
        );
        assert_eq!(
            std::fs::read_to_string(host.path().join("parent-new"))?,
            "parent addition"
        );
        assert!(!host.path().join("parent-delete").exists());
        assert!(!host.path().join("child-delete").exists());
        // Once-only: later parent edits are not reapplied or reclassified.
        std::fs::write(host.path().join("child"), "later parent edit")?;
        child.merge_back().await?;
        assert_eq!(
            std::fs::read_to_string(host.path().join("child"))?,
            "later parent edit"
        );
        Ok(())
    }

    #[tokio::test]
    async fn isolated_workspace_conflicts_preflight_entire_delta_and_failed_merge_is_once_only()
    -> io::Result<()> {
        for parent_deletes in [false, true] {
            let host = tempfile::tempdir()?;
            std::fs::write(host.path().join("z-conflict"), "baseline")?;
            let child = IsolatedWorkspace::clone_from(host.path())?;
            std::fs::write(child.path().join("a-new"), "must not be applied")?;
            std::fs::write(child.path().join("z-conflict"), "child edit")?;
            if parent_deletes {
                std::fs::remove_file(host.path().join("z-conflict"))?;
            } else {
                std::fs::write(host.path().join("z-conflict"), "parent edit")?;
            }
            let error = child.merge_back().await.unwrap_err();
            let report = error
                .get_ref()
                .unwrap()
                .downcast_ref::<MergeConflictReport>()
                .unwrap();
            assert_eq!(report.conflicting_files, vec![PathBuf::from("z-conflict")]);
            assert!(!host.path().join("a-new").exists());
            assert_eq!(host.path().join("z-conflict").exists(), !parent_deletes);
            // Repairing the host cannot trigger a second merge attempt.
            std::fs::write(host.path().join("z-conflict"), "baseline")?;
            assert_eq!(
                child.merge_back().await.unwrap_err().to_string(),
                error.to_string()
            );
            assert!(!host.path().join("a-new").exists());
        }
        Ok(())
    }

    #[tokio::test]
    async fn isolated_workspace_child_delete_conflicts_with_parent_edit_and_new_path_collision()
    -> io::Result<()> {
        let host = tempfile::tempdir()?;
        std::fs::write(host.path().join("deleted"), "baseline")?;
        let child = IsolatedWorkspace::clone_from(host.path())?;
        std::fs::remove_file(child.path().join("deleted"))?;
        std::fs::write(child.path().join("new"), "child")?;
        std::fs::write(host.path().join("deleted"), "parent")?;
        std::fs::write(host.path().join("new"), "parent")?;
        let error = child.merge_back().await.unwrap_err();
        let report = error
            .get_ref()
            .unwrap()
            .downcast_ref::<MergeConflictReport>()
            .unwrap();
        assert_eq!(
            report.conflicting_files,
            vec![PathBuf::from("deleted"), PathBuf::from("new")]
        );
        assert_eq!(
            std::fs::read_to_string(host.path().join("deleted"))?,
            "parent"
        );
        assert_eq!(std::fs::read_to_string(host.path().join("new"))?, "parent");
        Ok(())
    }

    #[tokio::test]
    async fn isolated_workspace_concurrent_children_merge_only_their_deltas() -> io::Result<()> {
        let host = tempfile::tempdir()?;
        std::fs::write(host.path().join("one"), "baseline")?;
        std::fs::write(host.path().join("two"), "baseline")?;
        let one = IsolatedWorkspace::clone_from(host.path())?;
        let two = IsolatedWorkspace::clone_from(host.path())?;
        std::fs::write(one.path().join("one"), "first child")?;
        std::fs::write(two.path().join("two"), "second child")?;
        let (a, b) = tokio::join!(one.merge_back(), two.merge_back());
        a?;
        b?;
        assert_eq!(
            std::fs::read_to_string(host.path().join("one"))?,
            "first child"
        );
        assert_eq!(
            std::fs::read_to_string(host.path().join("two"))?,
            "second child"
        );
        Ok(())
    }

    #[tokio::test]
    async fn isolated_workspace_checks_parent_after_waiting_for_file_lock() -> io::Result<()> {
        let host = tempfile::tempdir()?;
        let file = host.path().join("file");
        std::fs::write(&file, "baseline")?;
        let child = IsolatedWorkspace::clone_from(host.path())?;
        std::fs::write(child.path().join("file"), "child")?;
        let lock = crate::tools::file_lock::FileLockManager::global()
            .acquire_lock(&file)
            .await;
        let mut merge = Box::pin(child.merge_back());
        assert!(
            tokio::time::timeout(std::time::Duration::from_millis(10), &mut merge)
                .await
                .is_err()
        );
        std::fs::write(&file, "parent edited while merge waited")?;
        drop(lock);
        assert!(merge.await.is_err());
        assert_eq!(
            std::fs::read_to_string(file)?,
            "parent edited while merge waited"
        );
        Ok(())
    }

    #[tokio::test]
    async fn isolated_workspace_merge_failure_leaves_host_files_untouched() -> io::Result<()> {
        let host = tempfile::tempdir()?;
        std::fs::write(host.path().join("a"), "baseline")?;
        let child = IsolatedWorkspace::clone_from(host.path())?;
        std::fs::write(child.path().join("a"), "child")?;
        // A destination ancestor changed into a file; preflight rejects the
        // whole plan before the earlier lexicographic path can be written.
        std::fs::create_dir(child.path().join("z"))?;
        std::fs::write(child.path().join("z/new"), "child")?;
        std::fs::write(host.path().join("z"), "parent file")?;
        assert!(child.merge_back().await.is_err());
        assert_eq!(std::fs::read_to_string(host.path().join("a"))?, "baseline");
        assert_eq!(
            std::fs::read_to_string(host.path().join("z"))?,
            "parent file"
        );
        Ok(())
    }

    #[test]
    fn isolated_workspace_io_failure_rolls_back_applied_files_and_deletes() -> io::Result<()> {
        let host = tempfile::tempdir()?;
        for name in ["a-delete", "b-edit", "z-fail"] {
            std::fs::write(host.path().join(name), "baseline")?;
        }
        let mut plan = BTreeMap::new();
        for name in ["a-delete", "b-edit", "z-fail"] {
            let original = read_snapshot(&host.path().join(name))?;
            let child = if name == "a-delete" {
                None
            } else {
                Some(FileSnapshot {
                    bytes: b"child".to_vec(),
                    permissions: original.as_ref().unwrap().permissions.clone(),
                })
            };
            plan.insert(PathBuf::from(name), (original, child));
        }
        let error = apply_plan_with(host.path(), &plan, |dest, staged| {
            if dest.ends_with("z-fail") {
                return Err(io::Error::other("injected destination I/O failure"));
            }
            match staged {
                Some(file) => replace_file(dest, file),
                None => std::fs::remove_file(dest),
            }
        })
        .unwrap_err();
        assert!(
            error
                .to_string()
                .contains("injected destination I/O failure")
        );
        for name in ["a-delete", "b-edit", "z-fail"] {
            assert_eq!(std::fs::read_to_string(host.path().join(name))?, "baseline");
        }
        assert_eq!(
            std::fs::read_dir(host.path())?.count(),
            3,
            "no staging leaks"
        );
        Ok(())
    }

    #[tokio::test]
    async fn isolated_workspace_identical_edits_and_deletes_are_not_conflicts() -> io::Result<()> {
        let host = tempfile::tempdir()?;
        std::fs::write(host.path().join("edit"), "baseline")?;
        std::fs::write(host.path().join("delete"), "baseline")?;
        let child = IsolatedWorkspace::clone_from(host.path())?;
        for root in [host.path(), child.path()] {
            std::fs::write(root.join("edit"), "same edit")?;
            std::fs::write(root.join("new"), "same addition")?;
            std::fs::remove_file(root.join("delete"))?;
        }
        child.merge_back().await?;
        assert_eq!(
            std::fs::read_to_string(host.path().join("edit"))?,
            "same edit"
        );
        assert_eq!(
            std::fs::read_to_string(host.path().join("new"))?,
            "same addition"
        );
        assert!(!host.path().join("delete").exists());
        Ok(())
    }

    #[tokio::test]
    async fn isolated_workspace_ignore_change_cannot_hide_a_baseline_edit() -> io::Result<()> {
        let host = tempfile::tempdir()?;
        std::fs::write(host.path().join("file"), "baseline")?;
        let child = IsolatedWorkspace::clone_from(host.path())?;
        std::fs::write(child.path().join(".ignore"), "file\n")?;
        std::fs::write(child.path().join("file"), "child edit now hidden by ignore")?;
        child.merge_back().await?;
        assert_eq!(
            std::fs::read_to_string(host.path().join("file"))?,
            "child edit now hidden by ignore"
        );
        Ok(())
    }

    #[test]
    fn isolated_workspace_rollback_preserves_a_parent_edit_made_after_application() -> io::Result<()>
    {
        let host = tempfile::tempdir()?;
        std::fs::write(host.path().join("a-edit"), "baseline")?;
        std::fs::write(host.path().join("z-fail"), "baseline")?;
        let mut plan = BTreeMap::new();
        for name in ["a-edit", "z-fail"] {
            let original = read_snapshot(&host.path().join(name))?;
            let child = Some(FileSnapshot {
                bytes: b"child".to_vec(),
                permissions: original.as_ref().unwrap().permissions.clone(),
            });
            plan.insert(PathBuf::from(name), (original, child));
        }
        let error = apply_plan_with(host.path(), &plan, |dest, staged| {
            if dest.ends_with("z-fail") {
                std::fs::write(host.path().join("a-edit"), "parent after application")?;
                return Err(io::Error::other("injected I/O failure"));
            }
            replace_file(dest, staged.unwrap())
        })
        .unwrap_err();
        assert!(error.to_string().contains("rollback preserved host"));
        assert_eq!(
            std::fs::read_to_string(host.path().join("a-edit"))?,
            "parent after application"
        );
        assert_eq!(
            std::fs::read_to_string(host.path().join("z-fail"))?,
            "baseline"
        );
        Ok(())
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn isolated_workspace_materializes_internal_file_links_without_replacing_host_links()
    -> io::Result<()> {
        let host = tempfile::tempdir()?;
        std::fs::write(host.path().join("target"), "baseline")?;
        std::os::unix::fs::symlink("target", host.path().join("link"))?;
        let child = IsolatedWorkspace::clone_from(host.path())?;
        assert_eq!(
            std::fs::read_to_string(child.path().join("link"))?,
            "baseline"
        );
        assert!(
            !std::fs::symlink_metadata(child.path().join("link"))?
                .file_type()
                .is_symlink()
        );
        std::fs::write(child.path().join("target"), "child edit")?;
        child.merge_back().await?;
        assert!(
            std::fs::symlink_metadata(host.path().join("link"))?
                .file_type()
                .is_symlink()
        );
        assert_eq!(
            std::fs::read_to_string(host.path().join("link"))?,
            "child edit"
        );
        Ok(())
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn isolated_workspace_preserves_file_modes_and_detects_parent_chmod_conflict()
    -> io::Result<()> {
        use std::os::unix::fs::PermissionsExt;
        let host = tempfile::tempdir()?;
        let file = host.path().join("script");
        std::fs::write(&file, "baseline")?;
        std::fs::set_permissions(&file, std::fs::Permissions::from_mode(0o640))?;
        let child = IsolatedWorkspace::clone_from(host.path())?;
        std::fs::set_permissions(
            child.path().join("script"),
            std::fs::Permissions::from_mode(0o755),
        )?;
        child.merge_back().await?;
        assert_eq!(
            std::fs::metadata(&file)?.permissions().mode() & 0o777,
            0o755
        );
        let next = IsolatedWorkspace::clone_from(host.path())?;
        std::fs::write(next.path().join("script"), "child edit")?;
        std::fs::set_permissions(&file, std::fs::Permissions::from_mode(0o600))?;
        assert!(next.merge_back().await.is_err());
        assert_eq!(std::fs::read_to_string(&file)?, "baseline");
        assert_eq!(
            std::fs::metadata(&file)?.permissions().mode() & 0o777,
            0o600
        );
        Ok(())
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn isolated_workspace_rejects_host_symlink_ancestor() -> io::Result<()> {
        let host = tempfile::tempdir()?;
        let outside = tempfile::tempdir()?;
        let child = IsolatedWorkspace::clone_from(host.path())?;
        std::fs::create_dir(child.path().join("dir"))?;
        std::fs::write(child.path().join("dir/file"), "child")?;
        std::os::unix::fs::symlink(outside.path(), host.path().join("dir"))?;
        assert!(child.merge_back().await.is_err());
        assert!(!outside.path().join("file").exists());
        Ok(())
    }
}
