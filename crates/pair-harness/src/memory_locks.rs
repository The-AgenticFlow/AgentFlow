/// In-Memory File Locking for AgentFlow Pair Harness
///
/// Replaces Redis with an in-memory HashMap for file locking during development/testing.
/// Thread-safe via Arc<Mutex<>>.
use anyhow::Result;
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::Mutex;
use tracing::{info, warn};

use crate::isolation::LockError;

/// In-memory file lock manager
#[derive(Clone)]
pub struct MemoryLockManager {
    /// Map of file_path -> pair_id
    locks: Arc<Mutex<HashMap<String, String>>>,
    /// Pair identifier
    pair_id: String,
}

impl MemoryLockManager {
    pub fn new(pair_id: String) -> Self {
        Self {
            locks: Arc::new(Mutex::new(HashMap::new())),
            pair_id,
        }
    }

    /// Shared constructor for multiple pairs using the same lock map
    pub fn with_shared_locks(locks: Arc<Mutex<HashMap<String, String>>>, pair_id: String) -> Self {
        Self { locks, pair_id }
    }

    /// Attempts to acquire a lock on a file
    ///
    /// # Validation: C1-03 Dynamic File Locking
    /// Uses atomic in-memory operation (Mutex-guarded HashMap)
    pub async fn acquire_lock(&self, file_path: &str) -> Result<(), LockError> {
        let mut locks = self.locks.lock().await;

        info!(
            pair_id = %self.pair_id,
            file_path = %file_path,
            "Attempting to acquire file lock"
        );

        // Validation: C1-03 - Atomic check-and-set via Mutex
        match locks.get(file_path) {
            Some(owner) if owner != &self.pair_id => {
                warn!(
                    pair_id = %self.pair_id,
                    file_path = %file_path,
                    owner = %owner,
                    "Lock acquisition failed - already locked"
                );
                Err(LockError::AlreadyLocked {
                    file_path: file_path.to_string(),
                    owner: owner.clone(),
                })
            }
            _ => {
                // Either no lock exists or we already own it
                locks.insert(file_path.to_string(), self.pair_id.clone());
                info!(
                    pair_id = %self.pair_id,
                    file_path = %file_path,
                    "Lock acquired successfully"
                );
                Ok(())
            }
        }
    }

    /// Releases a lock on a file
    pub async fn release_lock(&self, file_path: &str) -> Result<(), LockError> {
        let mut locks = self.locks.lock().await;

        info!(
            pair_id = %self.pair_id,
            file_path = %file_path,
            "Releasing file lock"
        );

        match locks.get(file_path) {
            Some(owner) if owner == &self.pair_id => {
                locks.remove(file_path);
                info!(
                    pair_id = %self.pair_id,
                    file_path = %file_path,
                    "Lock released successfully"
                );
                Ok(())
            }
            Some(other_owner) => {
                warn!(
                    pair_id = %self.pair_id,
                    file_path = %file_path,
                    actual_owner = %other_owner,
                    "Cannot release lock - not owned by this pair"
                );
                Err(LockError::NotOwned)
            }
            None => {
                info!(
                    pair_id = %self.pair_id,
                    file_path = %file_path,
                    "Lock does not exist (already released)"
                );
                Ok(())
            }
        }
    }

    /// Checks who owns a lock on a file
    pub async fn check_lock_owner(&self, file_path: &str) -> Result<Option<String>> {
        let locks = self.locks.lock().await;
        Ok(locks.get(file_path).cloned())
    }

    /// Checks if this pair owns a lock on the file
    pub async fn owns_lock(&self, file_path: &str) -> Result<bool> {
        match self.check_lock_owner(file_path).await? {
            Some(owner) => Ok(owner == self.pair_id),
            None => Ok(false),
        }
    }

    /// Acquires locks on multiple files
    /// Returns the list of files that couldn't be locked with their owners
    pub async fn acquire_locks_batch(
        &self,
        file_paths: &[String],
    ) -> Result<Vec<(String, String)>> {
        let mut failed_locks = Vec::new();

        for file_path in file_paths {
            match self.acquire_lock(file_path).await {
                Ok(()) => continue,
                Err(LockError::AlreadyLocked { file_path, owner }) => {
                    failed_locks.push((file_path, owner));
                }
                Err(e) => return Err(e.into()),
            }
        }

        Ok(failed_locks)
    }

    /// Releases all locks owned by this pair
    pub async fn release_all_locks(&self) -> Result<()> {
        let mut locks = self.locks.lock().await;

        info!(
            pair_id = %self.pair_id,
            "Releasing all locks for pair"
        );

        // Find all locks owned by this pair
        let owned_files: Vec<String> = locks
            .iter()
            .filter(|(_, owner)| *owner == &self.pair_id)
            .map(|(file, _)| file.clone())
            .collect();

        let released = owned_files.len();
        for file in owned_files {
            locks.remove(&file);
        }

        info!(
            pair_id = %self.pair_id,
            released = released,
            "Released all locks for pair"
        );

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_lock_acquire_release() {
        let manager = MemoryLockManager::new("pair-1".to_string());

        // Acquire lock
        let result = manager.acquire_lock("test/file.rs").await;
        assert!(result.is_ok());

        // Try to acquire again - should succeed (we own it)
        let result = manager.acquire_lock("test/file.rs").await;
        assert!(result.is_ok());

        // Release lock
        let result = manager.release_lock("test/file.rs").await;
        assert!(result.is_ok());

        // Check lock is gone
        let owner = manager.check_lock_owner("test/file.rs").await.unwrap();
        assert!(owner.is_none());
    }

    #[tokio::test]
    async fn test_lock_conflict() {
        let shared_locks = Arc::new(Mutex::new(HashMap::new()));

        let manager1 =
            MemoryLockManager::with_shared_locks(shared_locks.clone(), "pair-1".to_string());
        let manager2 = MemoryLockManager::with_shared_locks(shared_locks, "pair-2".to_string());

        // Pair 1 acquires lock
        manager1.acquire_lock("test/file.rs").await.unwrap();

        // Pair 2 tries to acquire - should fail
        let result = manager2.acquire_lock("test/file.rs").await;
        assert!(matches!(result, Err(LockError::AlreadyLocked { .. })));

        // Pair 2 cannot release Pair 1's lock
        let result = manager2.release_lock("test/file.rs").await;
        assert!(matches!(result, Err(LockError::NotOwned)));

        // Pair 1 releases
        manager1.release_lock("test/file.rs").await.unwrap();

        // Now Pair 2 can acquire
        let result = manager2.acquire_lock("test/file.rs").await;
        assert!(result.is_ok());
    }
}
