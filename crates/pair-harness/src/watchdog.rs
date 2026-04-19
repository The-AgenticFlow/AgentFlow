// crates/pair-harness/src/watchdog.rs
//! Watchdog for detecting stalled pairs.
//!
//! Monitors WORKLOG.md updates and alerts if no progress is made
//! within the configured timeout.

use anyhow::{Context, Result};
use chrono::{DateTime, Utc};
use std::fs;
use std::path::PathBuf;
use std::time::{Duration, Instant};
use tracing::{debug, warn};

/// Watchdog for detecting stalled pairs.
pub struct Watchdog {
    /// Path to WORKLOG.md
    worklog_path: PathBuf,
    /// Timeout for no updates
    timeout: Duration,
    /// Last known update time
    last_update: Option<DateTime<Utc>>,
    /// When we last checked
    last_check: Instant,
}

impl Watchdog {
    /// Create a new watchdog.
    pub fn new(shared_dir: PathBuf, timeout_secs: u64) -> Self {
        Self {
            worklog_path: shared_dir.join("WORKLOG.md"),
            timeout: Duration::from_secs(timeout_secs),
            last_update: None,
            last_check: Instant::now(),
        }
    }

    /// Check if the pair is stalled (no WORKLOG update for too long).
    pub fn check_stalled(&mut self) -> Result<WatchdogStatus> {
        // Update our knowledge of the last update time
        self.refresh_last_update()?;

        let now = Utc::now();
        let elapsed = self
            .last_update
            .map(|last| (now - last).num_seconds() as u64)
            .unwrap_or(0);

        // Check if we've exceeded the timeout
        if elapsed > self.timeout.as_secs() {
            warn!(
                elapsed_secs = elapsed,
                timeout_secs = self.timeout.as_secs(),
                "Pair appears stalled"
            );
            return Ok(WatchdogStatus::Stalled {
                last_update: self.last_update,
                elapsed: Duration::from_secs(elapsed),
            });
        }

        // Check if we're approaching the timeout
        let warning_threshold = self.timeout.as_secs() / 2;
        if elapsed > warning_threshold {
            debug!(
                elapsed_secs = elapsed,
                warning_threshold_secs = warning_threshold,
                "Pair approaching stall threshold"
            );
            return Ok(WatchdogStatus::Warning {
                last_update: self.last_update,
                elapsed: Duration::from_secs(elapsed),
            });
        }

        Ok(WatchdogStatus::Active {
            last_update: self.last_update,
        })
    }

    /// Refresh our knowledge of the last update time.
    fn refresh_last_update(&mut self) -> Result<()> {
        if !self.worklog_path.exists() {
            // No worklog yet - this is normal for a new pair
            return Ok(());
        }

        let metadata =
            fs::metadata(&self.worklog_path).context("Failed to read WORKLOG.md metadata")?;

        let modified: std::time::SystemTime = metadata
            .modified()
            .context("Failed to get WORKLOG.md modification time")?;

        let modified_datetime: DateTime<Utc> = modified.into();

        // Only update if it's newer than what we know
        if self
            .last_update
            .map(|l| modified_datetime > l)
            .unwrap_or(true)
        {
            self.last_update = Some(modified_datetime);
            debug!(
                last_update = %modified_datetime.to_rfc3339(),
                "Updated last known WORKLOG modification time"
            );
        }

        Ok(())
    }

    /// Get the last known update time.
    pub fn last_update(&self) -> Option<DateTime<Utc>> {
        self.last_update
    }

    /// Reset the watchdog (call when activity is detected).
    pub fn reset(&mut self) {
        self.last_update = Some(Utc::now());
        self.last_check = Instant::now();
        debug!("Watchdog reset");
    }

    /// Check for segment loop (same segment evaluated too many times).
    pub fn check_segment_loop(
        &self,
        shared_dir: &PathBuf,
        segment: u32,
        max_iterations: u32,
    ) -> Result<bool> {
        let mut eval_count = 0;

        // Count how many eval files exist for this segment
        // (segment-N-eval.md files with CHANGES_REQUESTED)
        for entry in fs::read_dir(shared_dir).context("Failed to read shared directory")? {
            let entry = entry?;
            let filename = entry.file_name().to_string_lossy().to_string();

            if filename == format!("segment-{}-eval.md", segment) {
                // Read the file to check verdict
                let content = fs::read_to_string(entry.path())?;
                if content.contains("CHANGES_REQUESTED") {
                    eval_count += 1;
                }
            }
        }

        if eval_count > max_iterations {
            warn!(
                segment = segment,
                iterations = eval_count,
                max_iterations = max_iterations,
                "Segment loop detected"
            );
            return Ok(true);
        }

        Ok(false)
    }
}

/// Status returned by the watchdog.
#[derive(Debug, Clone)]
pub enum WatchdogStatus {
    /// Pair is active and making progress
    Active { last_update: Option<DateTime<Utc>> },
    /// Pair is approaching stall threshold
    Warning {
        last_update: Option<DateTime<Utc>>,
        elapsed: Duration,
    },
    /// Pair is stalled (no updates for too long)
    Stalled {
        last_update: Option<DateTime<Utc>>,
        elapsed: Duration,
    },
}

impl WatchdogStatus {
    /// Check if the status indicates stalled.
    pub fn is_stalled(&self) -> bool {
        matches!(self, WatchdogStatus::Stalled { .. })
    }

    /// Check if the status indicates a warning.
    pub fn is_warning(&self) -> bool {
        matches!(self, WatchdogStatus::Warning { .. })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::thread;
    use tempfile::tempdir;

    #[test]
    fn test_watchdog_active() {
        let dir = tempdir().unwrap();
        let shared = dir.path().to_path_buf();

        // Create WORKLOG.md
        fs::write(shared.join("WORKLOG.md"), "# Worklog\n").unwrap();

        let mut watchdog = Watchdog::new(shared, 1200); // 20 minutes

        let status = watchdog.check_stalled().unwrap();
        assert!(!status.is_stalled());
    }

    #[test]
    fn test_watchdog_stalled() {
        let dir = tempdir().unwrap();
        let shared = dir.path().to_path_buf();

        // Create WORKLOG.md with old timestamp (simulated by not creating it)
        // Actually, we need to create it and then wait, or mock the time

        let mut watchdog = Watchdog::new(shared.clone(), 1); // 1 second timeout

        // No WORKLOG.md exists - should not be stalled yet
        let status = watchdog.check_stalled().unwrap();
        assert!(!status.is_stalled());

        // Create WORKLOG.md
        fs::write(shared.join("WORKLOG.md"), "# Worklog\n").unwrap();

        // Wait for timeout
        thread::sleep(Duration::from_millis(1100));

        // Now check - should be stalled
        let status = watchdog.check_stalled().unwrap();
        assert!(status.is_stalled());
    }

    #[test]
    fn test_watchdog_reset() {
        let dir = tempdir().unwrap();
        let shared = dir.path().to_path_buf();

        let mut watchdog = Watchdog::new(shared, 1);

        // Reset the watchdog
        watchdog.reset();

        // Should be active
        let status = watchdog.check_stalled().unwrap();
        assert!(!status.is_stalled());
    }
}
