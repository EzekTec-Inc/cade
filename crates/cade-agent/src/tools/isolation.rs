use std::io;
use std::path::{Path, PathBuf};

/// RAII-managed isolated temporary workspace for concurrent execution.
/// Clones files from the primary workspace (respecting .gitignore / standard ignore rules),
/// and supports safely merging modified files back with global file lock coordination.
pub struct IsolatedWorkspace {
    temp_dir: tempfile::TempDir,
    primary_dir: PathBuf,
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
        })
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
        let temp_path = self.temp_dir.path();
        let walker = ignore::WalkBuilder::new(temp_path)
            .standard_filters(true)
            .hidden(false)
            .build();

        for entry in walker {
            if let Ok(entry) = entry {
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
        }
        Ok(())
    }
}
