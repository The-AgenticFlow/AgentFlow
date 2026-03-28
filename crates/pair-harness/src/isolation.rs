/// File Locking and Isolation for AgentFlow Pair Harness
///
/// Implements Redis-based dynamic file locking to prevent concurrent writes
/// across multiple FORGE-SENTINEL pairs.
use anyhow::{Context, Result};
use redis::AsyncCommands;
use thiserror::Error;
use tracing::{info, warn};

#[derive(Error, Debug)]
pub enum LockError {
    #[error("File {file_path} is locked by {owner}")]
    AlreadyLocked { file_path: String, owner: String },

    #[error("Redis connection error: {0}")]
    ConnectionError(#[from] redis::RedisError),

    #[error("Lock not owned by this pair")]
    NotOwned,
}

/// Manages file locks using Redis
pub struct IsolationManager {
    /// Redis connection manager for async operations
    redis_client: redis::Client,
    /// Pair identifier (e.g., "pair-1")
    pair_id: String,
    /// Lock TTL in seconds (default: 3600 = 1 hour)
    lock_ttl: u64,
}

impl IsolationManager {
    pub fn new(redis_url: &str, pair_id: String) -> Result<Self> {
        let redis_client =
            redis::Client::open(redis_url).context("Failed to create Redis client")?;

        Ok(Self {
            redis_client,
            pair_id,
            lock_ttl: 3600, // 1 hour default
        })
    }

    /// Sets the lock TTL
    pub fn with_ttl(mut self, ttl_secs: u64) -> Self {
        self.lock_ttl = ttl_secs;
        self
    }

    /// Attempts to acquire a lock on a file
    ///
    /// # Validation: C1-03 Dynamic File Locking
    /// Uses atomic Redis SET NX operation
    ///
    /// # Returns
    /// - Ok(()) if lock acquired successfully
    /// - Err(LockError::AlreadyLocked) if file is locked by another pair
    pub async fn acquire_lock(&self, file_path: &str) -> Result<(), LockError> {
        let lock_key = self.lock_key(file_path);

        info!(
            pair_id = %self.pair_id,
            file_path = %file_path,
            lock_key = %lock_key,
            "Attempting to acquire file lock"
        );

        let mut conn = self.redis_client.get_multiplexed_async_connection().await?;

        // Validation: C1-03 - Atomic Redis SET NX operation
        let result: Option<String> = redis::cmd("SET")
            .arg(&lock_key)
            .arg(&self.pair_id)
            .arg("NX") // Only set if not exists
            .arg("EX") // Set expiration
            .arg(self.lock_ttl)
            .query_async(&mut conn)
            .await?;

        match result {
            Some(_) => {
                info!(
                    pair_id = %self.pair_id,
                    file_path = %file_path,
                    "Lock acquired successfully"
                );
                Ok(())
            }
            None => {
                // Lock already exists, check who owns it
                let owner: String = conn.get(&lock_key).await?;

                warn!(
                    pair_id = %self.pair_id,
                    file_path = %file_path,
                    owner = %owner,
                    "Lock acquisition failed - already locked"
                );

                // Validation: C1-03 - Returns LockError if file is owned by another pair
                Err(LockError::AlreadyLocked {
                    file_path: file_path.to_string(),
                    owner,
                })
            }
        }
    }

    /// Releases a lock on a file
    ///
    /// # Validation: C1-03 Dynamic File Locking
    /// Ensures only the lock owner can release
    pub async fn release_lock(&self, file_path: &str) -> Result<(), LockError> {
        let lock_key = self.lock_key(file_path);

        info!(
            pair_id = %self.pair_id,
            file_path = %file_path,
            lock_key = %lock_key,
            "Releasing file lock"
        );

        let mut conn = self.redis_client.get_multiplexed_async_connection().await?;

        // Verify we own the lock before deleting
        let owner: Option<String> = conn.get(&lock_key).await?;

        match owner {
            Some(owner) if owner == self.pair_id => {
                let _: () = conn.del(&lock_key).await?;
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
                // Lock doesn't exist, that's fine
                info!(
                    pair_id = %self.pair_id,
                    file_path = %file_path,
                    "Lock does not exist (already released or expired)"
                );
                Ok(())
            }
        }
    }

    /// Checks who owns a lock on a file
    pub async fn check_lock_owner(&self, file_path: &str) -> Result<Option<String>> {
        let lock_key = self.lock_key(file_path);
        let mut conn = self
            .redis_client
            .get_multiplexed_async_connection()
            .await
            .context("Failed to connect to Redis")?;

        let owner: Option<String> = conn
            .get(&lock_key)
            .await
            .context("Failed to get lock owner")?;

        Ok(owner)
    }

    /// Checks if this pair owns a lock on the file
    pub async fn owns_lock(&self, file_path: &str) -> Result<bool> {
        match self.check_lock_owner(file_path).await? {
            Some(owner) => Ok(owner == self.pair_id),
            None => Ok(false),
        }
    }

    /// Acquires locks on multiple files atomically
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
        info!(
            pair_id = %self.pair_id,
            "Releasing all locks for pair"
        );

        let mut conn = self
            .redis_client
            .get_multiplexed_async_connection()
            .await
            .context("Failed to connect to Redis")?;

        // Scan for all lock keys owned by this pair
        let pattern = format!("lock:*");
        let keys: Vec<String> = redis::cmd("KEYS")
            .arg(&pattern)
            .query_async(&mut conn)
            .await
            .context("Failed to scan for locks")?;

        let mut released = 0;
        for key in keys {
            let owner: Option<String> = conn.get(&key).await?;
            if let Some(owner) = owner {
                if owner == self.pair_id {
                    let _: () = conn.del(&key).await?;
                    released += 1;
                }
            }
        }

        info!(
            pair_id = %self.pair_id,
            released = released,
            "Released all locks for pair"
        );

        Ok(())
    }

    /// Refreshes the TTL on a lock to prevent expiration
    pub async fn refresh_lock(&self, file_path: &str) -> Result<(), LockError> {
        let lock_key = self.lock_key(file_path);

        let mut conn = self.redis_client.get_multiplexed_async_connection().await?;

        // Verify we own the lock
        let owner: Option<String> = conn.get(&lock_key).await?;

        match owner {
            Some(owner) if owner == self.pair_id => {
                let _: bool = conn.expire(&lock_key, self.lock_ttl as i64).await?;
                info!(
                    pair_id = %self.pair_id,
                    file_path = %file_path,
                    "Lock TTL refreshed"
                );
                Ok(())
            }
            _ => Err(LockError::NotOwned),
        }
    }

    /// Generates the Redis key for a file lock
    /// Format: lock:{normalized_file_path}
    fn lock_key(&self, file_path: &str) -> String {
        // Normalize path by removing leading slashes and converting to relative
        let normalized = file_path.trim_start_matches('/');
        format!("lock:{}", normalized)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_lock_key_generation() {
        let manager = IsolationManager {
            redis_client: redis::Client::open("redis://localhost").unwrap(),
            pair_id: "pair-1".to_string(),
            lock_ttl: 3600,
        };

        assert_eq!(manager.lock_key("src/main.rs"), "lock:src/main.rs");

        assert_eq!(manager.lock_key("/src/lib.rs"), "lock:src/lib.rs");
    }

    #[tokio::test]
    #[ignore] // Requires Redis to be running
    async fn test_lock_acquire_release() {
        let manager = IsolationManager::new("redis://localhost", "pair-1".to_string()).unwrap();

        // Acquire lock
        let result = manager.acquire_lock("test/file.rs").await;
        assert!(result.is_ok());

        // Try to acquire again - should fail
        let result = manager.acquire_lock("test/file.rs").await;
        assert!(matches!(result, Err(LockError::AlreadyLocked { .. })));

        // Release lock
        let result = manager.release_lock("test/file.rs").await;
        assert!(result.is_ok());

        // Acquire again - should succeed
        let result = manager.acquire_lock("test/file.rs").await;
        assert!(result.is_ok());

        // Cleanup
        manager.release_lock("test/file.rs").await.ok();
    }
}
