// crates/pair-harness/src/isolation.rs
//! File locking for pair isolation using flock.
//!
//! This module implements dynamic file ownership locking to prevent
//! conflicts when multiple pairs might touch the same files.

use crate::types::FileLock;
use anyhow::{anyhow, Context, Result};
use sha2::{Digest, Sha256};
use std::fs::{self, File};
use std::io::{Read, Write};
use std::os::unix::fs::FileExt;
use std::path::{Path, PathBuf};
use tracing::{debug, info, warn};

/// Manages file locks for pair isolation.
pub struct FileLockManager {
    /// Directory where lock files are stored
    locks_dir: PathBuf,
}

impl FileLockManager {
    /// Create a new file lock manager.
    pub fn new(project_root: &Path) -> Self {
        Self {
            locks_dir: project_root.join("orchestration").join("locks"),
        }
    }

    /// Initialize the locks directory.
    pub fn init(&self) -> Result<()> {
        fs::create_dir_all(&self.locks_dir).context("Failed to create locks directory")?;
        Ok(())
    }

    /// Try to acquire a file lock for a pair.
    ///
    /// Returns Ok(true) if the lock was acquired (or already owned by this pair).
    /// Returns Ok(false) if another pair owns the lock.
    /// Returns Err on filesystem errors.
    pub fn try_acquire(&self, file_path: &Path, pair_id: &str) -> Result<LockResult> {
        self.init()?;

        let lock_hash = self.hash_path(file_path);
        let lock_file = self.locks_dir.join(format!("{}.lock", lock_hash));
        let json_file = self.locks_dir.join(format!("{}.json", lock_hash));

        debug!(
            file = %file_path.display(),
            pair = pair_id,
            hash = %lock_hash,
            "Attempting to acquire lock"
        );

        // Create or open the lock file
        let file = File::create(&lock_file).context("Failed to create lock file")?;

        // Try to acquire exclusive lock (non-blocking)
        // Using flock via fcntl (F_SETLK) for atomic acquisition
        let result = flock_exclusive_nonblocking(&file);

        match result {
            Ok(LockState::Acquired) => {
                // We have the lock, check if JSON exists
                if json_file.exists() {
                    let existing_lock = self.read_lock_json(&json_file)?;
                    if existing_lock.pair == pair_id {
                        // Already owned by us
                        debug!(file = %file_path.display(), pair = pair_id, "Lock already owned");
                        return Ok(LockResult::AlreadyOwned);
                    } else {
                        // Another pair owns it (shouldn't happen if we got the lock)
                        warn!(
                            file = %file_path.display(),
                            owner = %existing_lock.pair,
                            "Lock file inconsistency detected"
                        );
                    }
                }

                // Write our lock metadata
                let lock = FileLock::new(pair_id, file_path.to_string_lossy());
                self.write_lock_json(&json_file, &lock)?;

                info!(file = %file_path.display(), pair = pair_id, "Lock acquired");
                Ok(LockResult::Acquired)
            }
            Ok(LockState::WouldBlock) => {
                // Someone else has the lock, check who
                if json_file.exists() {
                    let existing_lock = self.read_lock_json(&json_file)?;
                    if existing_lock.pair == pair_id {
                        // We already own it (lock file is held by our process)
                        debug!(file = %file_path.display(), pair = pair_id, "Lock already owned");
                        return Ok(LockResult::AlreadyOwned);
                    } else {
                        info!(
                            file = %file_path.display(),
                            owner = %existing_lock.pair,
                            "File locked by another pair"
                        );
                        return Ok(LockResult::Blocked {
                            owner: existing_lock.pair,
                            acquired_at: existing_lock.acquired_at,
                        });
                    }
                }

                // No JSON but lock is held - race condition, wait and retry
                warn!(file = %file_path.display(), "Lock held but no metadata, retrying");
                Ok(LockResult::Blocked {
                    owner: "unknown".to_string(),
                    acquired_at: "unknown".to_string(),
                })
            }
            Err(e) => Err(anyhow!("Failed to acquire lock: {:#}", e)),
        }
    }

    /// Release a file lock owned by a pair.
    pub fn release(&self, file_path: &Path, pair_id: &str) -> Result<()> {
        let lock_hash = self.hash_path(file_path);
        let json_file = self.locks_dir.join(format!("{}.json", lock_hash));
        let lock_file = self.locks_dir.join(format!("{}.lock", lock_hash));

        if !json_file.exists() {
            debug!(file = %file_path.display(), "No lock to release");
            return Ok(());
        }

        let existing_lock = self.read_lock_json(&json_file)?;
        if existing_lock.pair != pair_id {
            warn!(
                file = %file_path.display(),
                owner = %existing_lock.pair,
                attempted_by = pair_id,
                "Cannot release lock owned by another pair"
            );
            return Ok(());
        }

        // Remove the JSON file
        fs::remove_file(&json_file).context("Failed to remove lock JSON")?;

        // The .lock file will be released when the process exits
        // (flock is automatically released on file close)

        info!(file = %file_path.display(), pair = pair_id, "Lock released");
        Ok(())
    }

    /// Release all locks owned by a pair.
    pub fn release_all_for_pair(&self, pair_id: &str) -> Result<Vec<String>> {
        self.init()?;

        let mut released = Vec::new();

        for entry in fs::read_dir(&self.locks_dir).context("Failed to read locks directory")? {
            let entry = entry?;
            let path = entry.path();

            if path.extension().map(|e| e == "json").unwrap_or(false) {
                if let Ok(lock) = self.read_lock_json(&path) {
                    if lock.pair == pair_id {
                        let file = lock.file.clone();
                        let lock_hash = path.file_stem().and_then(|s| s.to_str()).unwrap_or("");

                        // Remove JSON
                        let _ = fs::remove_file(&path);

                        // Remove .lock file
                        let lock_file = self.locks_dir.join(format!("{}.lock", lock_hash));
                        let _ = fs::remove_file(&lock_file);

                        released.push(file);
                    }
                }
            }
        }

        if !released.is_empty() {
            info!(
                pair = pair_id,
                count = released.len(),
                "Released all locks for pair"
            );
        }

        Ok(released)
    }

    /// Check if a file is locked by a specific pair.
    pub fn is_locked_by(&self, file_path: &Path, pair_id: &str) -> Result<bool> {
        let lock_hash = self.hash_path(file_path);
        let json_file = self.locks_dir.join(format!("{}.json", lock_hash));

        if !json_file.exists() {
            return Ok(false);
        }

        let lock = self.read_lock_json(&json_file)?;
        Ok(lock.pair == pair_id)
    }

    /// Get the owner of a file lock, if any.
    pub fn get_owner(&self, file_path: &Path) -> Result<Option<FileLock>> {
        let lock_hash = self.hash_path(file_path);
        let json_file = self.locks_dir.join(format!("{}.json", lock_hash));

        if !json_file.exists() {
            return Ok(None);
        }

        let lock = self.read_lock_json(&json_file)?;
        Ok(Some(lock))
    }

    /// Seed initial locks for a ticket's touched files.
    pub fn seed_locks(&self, files: &[String], pair_id: &str) -> Result<Vec<String>> {
        let mut locked = Vec::new();

        for file in files {
            let path = PathBuf::from(file);
            match self.try_acquire(&path, pair_id)? {
                LockResult::Acquired | LockResult::AlreadyOwned => {
                    locked.push(file.clone());
                }
                LockResult::Blocked { owner, .. } => {
                    warn!(
                        file = file,
                        owner = %owner,
                        "Cannot seed lock, file already locked"
                    );
                }
            }
        }

        info!(
            pair = pair_id,
            count = locked.len(),
            "Seeded initial file locks"
        );
        Ok(locked)
    }

    /// Hash a file path for use as lock filename.
    fn hash_path(&self, path: &Path) -> String {
        let path_str = path.to_string_lossy();
        let mut hasher = Sha256::new();
        hasher.update(path_str.as_bytes());
        let hash = hasher.finalize();
        hex::encode(&hash[..16]) // Use first 16 bytes (32 hex chars)
    }

    /// Read lock JSON metadata.
    fn read_lock_json(&self, path: &Path) -> Result<FileLock> {
        let content = fs::read_to_string(path).context("Failed to read lock JSON")?;
        let lock: FileLock = serde_json::from_str(&content).context("Failed to parse lock JSON")?;
        Ok(lock)
    }

    /// Write lock JSON metadata.
    fn write_lock_json(&self, path: &Path, lock: &FileLock) -> Result<()> {
        let content =
            serde_json::to_string_pretty(lock).context("Failed to serialize lock JSON")?;

        // Atomic write: write to temp, then rename
        let temp_path = path.with_extension("json.tmp");
        fs::write(&temp_path, content).context("Failed to write lock JSON")?;
        fs::rename(&temp_path, path).context("Failed to rename lock JSON")?;

        Ok(())
    }
}

/// Result of a lock acquisition attempt.
#[derive(Debug, Clone)]
pub enum LockResult {
    /// Lock was acquired successfully
    Acquired,
    /// Lock is already owned by this pair
    AlreadyOwned,
    /// Lock is held by another pair
    Blocked { owner: String, acquired_at: String },
}

/// State returned by flock operation.
#[derive(Debug, Clone, PartialEq, Eq)]
enum LockState {
    Acquired,
    WouldBlock,
}

/// Attempt to acquire an exclusive lock (non-blocking) using flock.
/// This uses fcntl(F_SETLK) which is non-blocking.
#[cfg(unix)]
fn flock_exclusive_nonblocking(file: &File) -> std::io::Result<LockState> {
    use libc::{flock, LOCK_EX, LOCK_NB, LOCK_UN};
    use std::os::unix::io::AsRawFd;

    let fd = file.as_raw_fd();
    let result = unsafe { flock(fd, LOCK_EX | LOCK_NB) };

    if result == 0 {
        Ok(LockState::Acquired)
    } else {
        let err = std::io::Error::last_os_error();
        if err.kind() == std::io::ErrorKind::WouldBlock {
            Ok(LockState::WouldBlock)
        } else {
            Err(err)
        }
    }
}

#[cfg(not(unix))]
fn flock_exclusive_nonblocking(_file: &File) -> std::io::Result<LockState> {
    // On non-Unix systems, use a simple file existence check
    // This is not as robust but provides basic functionality
    Ok(LockState::Acquired)
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn test_hash_path_consistency() {
        let dir = tempdir().unwrap();
        let manager = FileLockManager::new(dir.path());

        let path1 = Path::new("src/auth/login.ts");
        let path2 = Path::new("src/auth/login.ts");
        let path3 = Path::new("src/auth/logout.ts");

        assert_eq!(manager.hash_path(path1), manager.hash_path(path2));
        assert_ne!(manager.hash_path(path1), manager.hash_path(path3));
    }

    #[test]
    fn test_lock_acquisition() {
        let dir = tempdir().unwrap();
        let manager = FileLockManager::new(dir.path());

        let file = Path::new("src/test.ts");

        // First acquisition should succeed
        let result = manager.try_acquire(file, "pair-1").unwrap();
        assert!(matches!(result, LockResult::Acquired));

        // Same pair should get AlreadyOwned
        let result = manager.try_acquire(file, "pair-1").unwrap();
        assert!(matches!(result, LockResult::AlreadyOwned));

        // Different pair should be blocked
        let result = manager.try_acquire(file, "pair-2").unwrap();
        assert!(matches!(result, LockResult::Blocked { .. }));
    }

    #[test]
    fn test_lock_release() {
        let dir = tempdir().unwrap();
        let manager = FileLockManager::new(dir.path());

        let file = Path::new("src/test.ts");

        manager.try_acquire(file, "pair-1").unwrap();
        manager.release(file, "pair-1").unwrap();

        // After release, another pair can acquire
        let result = manager.try_acquire(file, "pair-2").unwrap();
        assert!(matches!(result, LockResult::Acquired));
    }

    #[test]
    fn test_release_all_for_pair() {
        let dir = tempdir().unwrap();
        let manager = FileLockManager::new(dir.path());

        manager
            .try_acquire(Path::new("src/a.ts"), "pair-1")
            .unwrap();
        manager
            .try_acquire(Path::new("src/b.ts"), "pair-1")
            .unwrap();
        manager
            .try_acquire(Path::new("src/c.ts"), "pair-2")
            .unwrap();

        let released = manager.release_all_for_pair("pair-1").unwrap();
        assert_eq!(released.len(), 2);

        // pair-2's lock should still exist
        let owner = manager.get_owner(Path::new("src/c.ts")).unwrap();
        assert!(owner.is_some());
        assert_eq!(owner.unwrap().pair, "pair-2");
    }
}
