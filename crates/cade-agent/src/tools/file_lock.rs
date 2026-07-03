use std::collections::HashMap;
use std::path::Path;
use std::sync::{Arc, Mutex, OnceLock};

pub struct FileLockManager {
    locks: Mutex<HashMap<String, Arc<tokio::sync::Mutex<()>>>>,
}

impl FileLockManager {
    pub fn global() -> &'static Self {
        static INSTANCE: OnceLock<FileLockManager> = OnceLock::new();
        INSTANCE.get_or_init(|| Self {
            locks: Mutex::new(HashMap::new()),
        })
    }

    /// Normalize a path to a workspace-relative string key for cross-backend lock safety (ADR 6).
    pub fn normalize_key(path: &Path) -> String {
        let abs_path = path.canonicalize().unwrap_or_else(|_| path.to_path_buf());
        let path_str = abs_path.to_string_lossy();
        
        // Match /sa_ followed by 8 characters (hex/uuid slice) and a slash
        if let Some(idx) = path_str.find("/sa_") {
            let sub_path = &path_str[idx + 1..]; // e.g. "sa_12345678/src/main.rs"
            if let Some(slash_idx) = sub_path.find('/') {
                return sub_path[slash_idx + 1..].to_string();
            }
        }

        if let Ok(cwd) = std::env::current_dir() {
            let abs_cwd = cwd.canonicalize().unwrap_or(cwd);
            if let Ok(relative) = abs_path.strip_prefix(&abs_cwd) {
                return relative.to_string_lossy().to_string();
            }
        }
        abs_path.to_string_lossy().to_string()
    }

    pub async fn acquire_lock(&self, path: &Path) -> tokio::sync::OwnedMutexGuard<()> {
        let key = Self::normalize_key(path);
        let lock = {
            let mut guard = self.locks.lock().unwrap();
            guard.entry(key).or_insert_with(|| Arc::new(tokio::sync::Mutex::new(()))).clone()
        };
        lock.clone().lock_owned().await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    #[test]
    fn test_file_lock_sandbox_normalization() {
        let path = Path::new("/tmp/sa_a5f3bc9d/src/components/home.rs");
        let key = FileLockManager::normalize_key(path);
        assert_eq!(key, "src/components/home.rs");

        let path2 = Path::new("/var/folders/temp/sa_12345678/crates/cade-core/src/lib.rs");
        let key2 = FileLockManager::normalize_key(path2);
        assert_eq!(key2, "crates/cade-core/src/lib.rs");
    }

    #[tokio::test]
    async fn test_file_lock_mutual_exclusion() {
        let manager = FileLockManager::global();
        let path = Path::new("/tmp/test_locking_file.txt");

        let lock1 = manager.acquire_lock(path).await;

        let start_time = std::time::Instant::now();
        let handle = tokio::spawn(async move {
            let _lock2 = FileLockManager::global().acquire_lock(Path::new("/tmp/test_locking_file.txt")).await;
            std::time::Instant::now()
        });

        tokio::time::sleep(Duration::from_millis(50)).await;
        drop(lock1);

        let end_time = handle.await.unwrap();
        assert!(end_time.duration_since(start_time) >= Duration::from_millis(50));
    }

    #[tokio::test]
    async fn test_file_lock_concurrent_sandbox() {
        let manager = FileLockManager::global();
        
        let path_sandbox1 = Path::new("/tmp/sa_12345678/src/main.rs");
        let path_sandbox2 = Path::new("/tmp/sa_abcdef01/src/main.rs");

        let lock1 = manager.acquire_lock(path_sandbox1).await;

        let start_time = std::time::Instant::now();
        let handle = tokio::spawn(async move {
            let _lock2 = FileLockManager::global().acquire_lock(path_sandbox2).await;
            std::time::Instant::now()
        });

        tokio::time::sleep(Duration::from_millis(50)).await;
        drop(lock1);

        let end_time = handle.await.unwrap();
        assert!(end_time.duration_since(start_time) >= Duration::from_millis(50));
    }
}
