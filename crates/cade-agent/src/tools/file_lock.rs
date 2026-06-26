use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, OnceLock};

pub struct FileLockManager {
    locks: Mutex<HashMap<PathBuf, Arc<tokio::sync::Mutex<()>>>>,
}

impl FileLockManager {
    pub fn global() -> &'static Self {
        static INSTANCE: OnceLock<FileLockManager> = OnceLock::new();
        INSTANCE.get_or_init(|| Self {
            locks: Mutex::new(HashMap::new()),
        })
    }

    pub async fn acquire_lock(&self, path: &Path) -> tokio::sync::OwnedMutexGuard<()> {
        let abs_path = path.canonicalize().unwrap_or_else(|_| path.to_path_buf());
        let lock = {
            let mut guard = self.locks.lock().unwrap();
            guard.entry(abs_path).or_insert_with(|| Arc::new(tokio::sync::Mutex::new(()))).clone()
        };
        lock.clone().lock_owned().await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

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
}
