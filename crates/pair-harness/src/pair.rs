// crates/pair-harness/src/pair.rs
//! ForgeSentinelPair - the main pair lifecycle manager.
//!
//! Implements the v3 event-driven architecture where:
//! - FORGE is a long-running process
//! - SENTINEL is spawned fresh per evaluation
//! - The harness uses inotify for zero-polling event detection

use anyhow::{Context, Result};
use std::path::PathBuf;
use std::time::{Duration, Instant};
use tokio::process::Child;
use tracing::{info, warn, error};

use crate::types::{FsEvent, Ticket, PairConfig, PairOutcome, StatusJson};
use crate::worktree::WorktreeManager;
use crate::isolation::FileLockManager;
use crate::process::{ProcessManager, SentinelMode};
use crate::watcher::SharedDirWatcher;
use crate::reset::ResetManager;
use crate::watchdog::Watchdog;
use crate::provision::Provisioner;

const SENTINEL_TIMEOUT_SECS: u64 = 120;
const FORGE_STARTUP_TIMEOUT_SECS: u64 = 300; // 5 minutes to write PLAN.md

struct SentinelTracker {
    mode: SentinelMode,
    spawn_time: Instant,
}

/// The main FORGE-SENTINEL pair lifecycle manager.
pub struct ForgeSentinelPair {
    config: PairConfig,
    worktree: WorktreeManager,
    locks: FileLockManager,
    process: ProcessManager,
    reset: ResetManager,
    watchdog: Watchdog,
    start_time: Instant,
    sentinel_tracker: Option<SentinelTracker>,
    forge_spawn_time: Instant,
}

impl ForgeSentinelPair {
    /// Create a new ForgeSentinelPair.
    pub fn new(config: PairConfig) -> Self {
        // Use the project_root from config (contains .git)
        let project_root = config.project_root.clone();

        Self {
            worktree: WorktreeManager::new(&project_root),
            locks: FileLockManager::new(&project_root),
            process: if let Some(redis_url) = &config.redis_url {
                ProcessManager::with_redis(&config.github_token, redis_url)
            } else {
                ProcessManager::new(&config.github_token)
            },
            reset: ResetManager::new(config.shared.clone(), config.max_resets),
            watchdog: Watchdog::new(config.shared.clone(), config.watchdog_timeout_secs),
            config,
            start_time: Instant::now(),
            sentinel_tracker: None,
            forge_spawn_time: Instant::now(),
        }
    }

    /// Run the pair lifecycle for a ticket.
    ///
    /// This is the main event loop that:
    /// 1. Provisions the worktree and configuration
    /// 2. Spawns FORGE
    /// 3. Watches for filesystem events
    /// 4. Spawns SENTINEL for evaluations
    /// 5. Handles context resets
    /// 6. Returns the final outcome
    pub async fn run(&mut self, ticket: &Ticket) -> Result<PairOutcome> {
        info!(
            pair = %self.config.pair_id,
            ticket = %ticket.id,
            "Starting pair lifecycle"
        );

        self.start_time = Instant::now();

        // 1. Provision worktree
        self.provision_worktree(ticket).await?;

        // 2. Provision configuration files
        self.provision_config(ticket).await?;

        // 3. Seed initial file locks
        self.seed_locks(ticket).await?;

        // 4. Create shared directory structure
        self.create_shared_structure().await?;

        // 5. Write TICKET.md and TASK.md
        self.write_task_context(ticket).await?;

        // 6. Spawn FORGE process
        let mut forge = self.spawn_forge().await?;

        // 7. Start filesystem watcher
        let mut watcher = SharedDirWatcher::new(&self.config.shared)?;

        // 8. Event loop
        let outcome = self.event_loop(&mut forge, &mut watcher).await?;

        // 9. Cleanup
        self.cleanup(&forge).await?;

        info!(
            pair = %self.config.pair_id,
            outcome = ?outcome,
            elapsed = ?self.start_time.elapsed(),
            "Pair lifecycle complete"
        );

        Ok(outcome)
    }

    /// The main event loop.
    async fn event_loop(
        &mut self,
        forge: &mut Child,
        watcher: &mut SharedDirWatcher,
    ) -> Result<PairOutcome> {
        loop {
            // Check for SENTINEL timeout
            if let Some(tracker) = &self.sentinel_tracker {
                if tracker.spawn_time.elapsed().as_secs() > SENTINEL_TIMEOUT_SECS {
                    warn!(
                        mode = ?tracker.mode,
                        "SENTINEL timed out after {}s",
                        SENTINEL_TIMEOUT_SECS
                    );
                    self.sentinel_tracker = None;
                }
            }

            // Check for FORGE startup timeout (no PLAN.md written)
            let plan_path = self.config.shared.join("PLAN.md");
            if !plan_path.exists() 
                && self.forge_spawn_time.elapsed().as_secs() > FORGE_STARTUP_TIMEOUT_SECS {
                error!(
                    "FORGE startup timeout - no PLAN.md after {}s",
                    FORGE_STARTUP_TIMEOUT_SECS
                );
                
                // Check if FORGE is still running
                if self.process.is_running(forge).await {
                    warn!("Killing stuck FORGE process and respawning");
                    self.process.kill(forge).await?;
                    *forge = self.spawn_forge_resume().await?;
                    self.reset.increment_reset();
                }
            }

            // Check for filesystem events (with timeout)
            let event = watcher.recv_timeout(Duration::from_millis(100));

            if let Some(evt) = event {
                match evt {
                    FsEvent::PlanWritten => {
                        if self.sentinel_tracker.is_none() {
                            info!("PLAN.md written - spawning SENTINEL for plan review");
                            self.spawn_sentinel_for_plan().await?;
                        } else {
                            warn!("SENTINEL already active - skipping duplicate spawn");
                        }
                    }

                    FsEvent::ContractWritten => {
                        self.sentinel_tracker = None;
                        let status = self.read_contract_status().await?;
                        if status == "AGREED" {
                            info!("Contract agreed - FORGE can begin implementation");
                        } else {
                            info!("Contract has issues - FORGE must revise plan");
                        }
                    }

                    FsEvent::WorklogUpdated => {
                        let segment_n = self.extract_latest_segment().await?;
                        info!("Segment {} complete - spawning SENTINEL for eval", segment_n);
                        self.spawn_sentinel_for_segment(segment_n).await?;
                        self.watchdog.reset();
                    }

                    FsEvent::SegmentEvalWritten(n) => {
                        self.sentinel_tracker = None;
                        info!("Segment {} evaluation complete", n);
                    }

                    FsEvent::FinalReviewWritten => {
                        self.sentinel_tracker = None;
                        let verdict = self.read_final_review_verdict().await?;
                        if verdict == "APPROVED" {
                            info!("Final review APPROVED - FORGE can open PR");
                        } else {
                            info!("Final review REJECTED - FORGE must fix issues");
                        }
                    }

                    FsEvent::StatusJsonWritten => {
                        self.sentinel_tracker = None;
                        let status = self.read_status().await?;
                        return Ok(status);
                    }

                    FsEvent::HandoffWritten => {
                        self.sentinel_tracker = None;
                        info!("Context reset - respawning FORGE");
                        self.process.kill(forge).await?;
                        *forge = self.spawn_forge_resume().await?;
                        self.reset.increment_reset();
                    }
                }
            }

            // Check watchdog (every ~60 seconds)
            if self.start_time.elapsed().as_secs() % 60 == 0 {
                let status = self.watchdog.check_stalled()?;
                if status.is_stalled() {
                    warn!("Pair stalled - no WORKLOG update for too long");
                }
            }

            // Check if FORGE has exited
            if !self.process.is_running(forge).await {
                if self.reset.has_handoff() {
                    // Clean handoff - respawn
                    info!("FORGE exited with handoff - respawning");
                    *forge = self.spawn_forge_resume().await?;
                    self.reset.increment_reset();
                } else if self.config.shared.join("STATUS.json").exists() {
                    // Terminal state - read and return
                    let status = self.read_status().await?;
                    return Ok(status);
                } else if self.has_progress_files().await {
                    // FORGE made progress - check if SENTINEL is active
                    if self.sentinel_tracker.is_some() {
                        // SENTINEL is working - wait for it
                        info!("FORGE exited but SENTINEL is active - waiting for completion");
                        tokio::time::sleep(Duration::from_secs(5)).await;
                    } else {
                        // No active SENTINEL - check if we're waiting for one
                        let waiting_for_sentinel = self.waiting_for_sentinel_output().await;
                        if waiting_for_sentinel {
                            warn!("FORGE exited while waiting for SENTINEL output - spawning SENTINEL");
                            if self.config.shared.join("PLAN.md").exists() 
                                && !self.config.shared.join("CONTRACT.md").exists() {
                                self.spawn_sentinel_for_plan().await?;
                            }
                            tokio::time::sleep(Duration::from_secs(5)).await;
                        } else {
                            // Normal respawn
                            info!("FORGE exited after making progress - respawning to continue");
                            *forge = self.spawn_forge_resume().await?;
                            self.reset.increment_reset();
                        }
                    }
                } else {
                    // Unclean exit - synthesize handoff and respawn
                    warn!("FORGE exited unexpectedly - synthesizing handoff");
                    self.reset.synthesize_handoff().await?;
                    *forge = self.spawn_forge_resume().await?;
                    self.reset.increment_reset();
                }
            }

            // Check reset limit
            if self.reset.reset_count() >= self.config.max_resets {
                warn!("Max resets exceeded - fuel exhausted");
                return Ok(PairOutcome::FuelExhausted {
                    reason: "Maximum context resets exceeded".to_string(),
                    reset_count: self.reset.reset_count(),
                });
            }

            // Small sleep to prevent busy loop
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
    }

    /// Provision the worktree for this pair.
    async fn provision_worktree(&self, ticket: &Ticket) -> Result<()> {
        self.worktree.create_worktree(&self.config.pair_id, &ticket.id)
            .context("Failed to create worktree")?;
        Ok(())
    }

    /// Provision configuration files.
    async fn provision_config(&self, _ticket: &Ticket) -> Result<()> {
        // Use project_root where .sprintless/plugin exists
        let provisioner = Provisioner::new(&self.config.project_root);

        provisioner.provision_pair(
            &self.config.pair_id,
            &self.config.worktree,
            &self.config.shared,
            &self.config.github_token,
            self.config.redis_url.as_deref(),
        ).await
    }

    /// Seed initial file locks for the ticket.
    async fn seed_locks(&self, ticket: &Ticket) -> Result<()> {
        self.locks.seed_locks(&ticket.touched_files, &self.config.pair_id)?;
        Ok(())
    }

    /// Create shared directory structure.
    async fn create_shared_structure(&self) -> Result<()> {
        let provisioner = Provisioner::new(&self.config.project_root);
        provisioner.create_shared_structure(&self.config.shared)
    }

    /// Write TICKET.md and TASK.md to shared directory.
    async fn write_task_context(&self, ticket: &Ticket) -> Result<()> {
        let provisioner = Provisioner::new(&self.config.project_root);
        provisioner.write_ticket(&self.config.shared, ticket)?;

        // Write a basic TASK.md
        let task = format!(
            "Implement ticket {}.\n\nBranch: {}\n\nWhen done, open a PR and write STATUS.json.",
            ticket.id,
            WorktreeManager::branch_name(&self.config.pair_id, &ticket.id)
        );
        provisioner.write_task(&self.config.shared, &task)
    }

    /// Spawn FORGE process.
    async fn spawn_forge(&mut self) -> Result<Child> {
        self.forge_spawn_time = Instant::now();
        self.process.spawn_forge(
            &self.config.pair_id,
            "", // ticket_id is in the environment already
            &self.config.worktree,
            &self.config.shared,
        ).await
    }

    /// Spawn FORGE process in resume mode.
    async fn spawn_forge_resume(&mut self) -> Result<Child> {
        self.forge_spawn_time = Instant::now();
        self.process.spawn_forge_resume(
            &self.config.pair_id,
            "",
            &self.config.worktree,
            &self.config.shared,
        ).await
    }

    /// Spawn SENTINEL for plan review.
    async fn spawn_sentinel_for_plan(&mut self) -> Result<()> {
        let _child = self.process.spawn_sentinel(
            &self.config.pair_id,
            "",
            SentinelMode::PlanReview,
            &self.config.worktree,
            &self.config.shared,
        ).await?;

        self.sentinel_tracker = Some(SentinelTracker {
            mode: SentinelMode::PlanReview,
            spawn_time: Instant::now(),
        });

        Ok(())
    }

    /// Spawn SENTINEL for segment evaluation.
    async fn spawn_sentinel_for_segment(&mut self, segment: u32) -> Result<()> {
        let _child = self.process.spawn_sentinel(
            &self.config.pair_id,
            "",
            SentinelMode::SegmentEval(segment),
            &self.config.worktree,
            &self.config.shared,
        ).await?;

        self.sentinel_tracker = Some(SentinelTracker {
            mode: SentinelMode::SegmentEval(segment),
            spawn_time: Instant::now(),
        });

        Ok(())
    }

    /// Read CONTRACT.md status.
    async fn read_contract_status(&self) -> Result<String> {
        let path = self.config.shared.join("CONTRACT.md");
        if !path.exists() {
            return Ok("UNKNOWN".to_string());
        }

        let content = tokio::fs::read_to_string(&path).await?;
        if content.contains("status: AGREED") || content.contains("status: \"AGREED\"") {
            Ok("AGREED".to_string())
        } else if content.contains("status: ISSUES") || content.contains("status: \"ISSUES\"") {
            Ok("ISSUES".to_string())
        } else {
            Ok("UNKNOWN".to_string())
        }
    }

    /// Extract the latest segment number from WORKLOG.md.
    async fn extract_latest_segment(&self) -> Result<u32> {
        let path = self.config.shared.join("WORKLOG.md");
        if !path.exists() {
            return Ok(0);
        }

        let content = tokio::fs::read_to_string(&path).await?;

        // Find the last "## Segment N" header
        let mut latest = 0;
        for line in content.lines() {
            if line.starts_with("## Segment") {
                if let Some(n) = line
                    .split_whitespace()
                    .nth(2)
                    .and_then(|s| s.parse::<u32>().ok())
                {
                    latest = n;
                }
            }
        }

        Ok(latest)
    }

    /// Read final-review.md verdict.
    async fn read_final_review_verdict(&self) -> Result<String> {
        let path = self.config.shared.join("final-review.md");
        if !path.exists() {
            return Ok("UNKNOWN".to_string());
        }

        let content = tokio::fs::read_to_string(&path).await?;
        if content.contains("APPROVED") {
            Ok("APPROVED".to_string())
        } else if content.contains("REJECTED") {
            Ok("REJECTED".to_string())
        } else {
            Ok("UNKNOWN".to_string())
        }
    }

    /// Read STATUS.json and convert to PairOutcome.
    async fn read_status(&self) -> Result<PairOutcome> {
        let path = self.config.shared.join("STATUS.json");
        if !path.exists() {
            return Ok(PairOutcome::FuelExhausted {
                reason: "No STATUS.json written".to_string(),
                reset_count: self.reset.reset_count(),
            });
        }

        let content = tokio::fs::read_to_string(&path).await?;
        let status: StatusJson = serde_json::from_str(&content)?;

        match status.status.as_str() {
            "PR_OPENED" => Ok(PairOutcome::PrOpened {
                pr_url: status.pr_url.clone().unwrap_or_default(),
                pr_number: status.pr_number.unwrap_or(0),
                branch: status.branch.clone().unwrap_or_default(),
            }),
            "COMPLETED" | "complete" | "completed" => {
                // These statuses require a PR URL to be considered successful
                if status.pr_url.is_some() && !status.pr_url.as_ref().unwrap().is_empty() {
                    Ok(PairOutcome::PrOpened {
                        pr_url: status.pr_url.clone().unwrap_or_default(),
                        pr_number: status.pr_number.unwrap_or(0),
                        branch: status.branch.clone().unwrap_or_default(),
                    })
                } else {
                    Ok(PairOutcome::Blocked {
                        reason: "Work complete but PR not created - needs push/PR creation".to_string(),
                        blockers: vec![],
                    })
                }
            }
            "IMPLEMENTATION_COMPLETE" => {
                // Implementation done but not pushed/PR created - treat as blocked
                Ok(PairOutcome::Blocked {
                    reason: "Implementation complete but PR not created - needs push/PR creation".to_string(),
                    blockers: vec![],
                })
            }
            "BLOCKED" => Ok(PairOutcome::Blocked {
                reason: "See blockers".to_string(),
                blockers: status.blockers,
            }),
            "FUEL_EXHAUSTED" => Ok(PairOutcome::FuelExhausted {
                reason: "Fuel exhausted".to_string(),
                reset_count: status.context_resets,
            }),
            _ => Ok(PairOutcome::FuelExhausted {
                reason: format!("Unknown status: {}", status.status),
                reset_count: self.reset.reset_count(),
            }),
        }
    }

    /// Check if FORGE has made progress (PLAN.md or WORKLOG.md exists).
    async fn has_progress_files(&self) -> bool {
        let plan_path = self.config.shared.join("PLAN.md");
        let worklog_path = self.config.shared.join("WORKLOG.md");
        
        plan_path.exists() || worklog_path.exists()
    }

    /// Check if we're waiting for SENTINEL output (plan reviewed but no contract).
    async fn waiting_for_sentinel_output(&self) -> bool {
        let plan_path = self.config.shared.join("PLAN.md");
        let contract_path = self.config.shared.join("CONTRACT.md");
        let worklog_path = self.config.shared.join("WORKLOG.md");
        
        // Waiting for plan review
        if plan_path.exists() && !contract_path.exists() {
            return true;
        }
        
        // Waiting for segment eval (WORKLOG exists but no corresponding eval)
        if worklog_path.exists() {
            if let Ok(segment) = self.extract_latest_segment().await {
                if segment > 0 {
                    let eval_path = self.config.shared.join(format!("segment-{}-eval.md", segment));
                    if !eval_path.exists() {
                        return true;
                    }
                }
            }
        }
        
        false
    }

    /// Cleanup after pair completion.
    async fn cleanup(&self, _forge: &Child) -> Result<()> {
        // Release all file locks
        self.locks.release_all_for_pair(&self.config.pair_id)?;

        // Remove worktree (optional - could keep for debugging)
        // self.worktree.remove_worktree(&self.config.pair_id)?;

        info!(pair = %self.config.pair_id, "Cleanup complete");
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_pair_config_creation() {
        let config = PairConfig::new(
            "pair-1",
            std::path::Path::new("/project"),
            "ghp_test",
        );

        assert_eq!(config.pair_id, "pair-1");
        assert!(config.worktree.ends_with("worktrees/pair-1"));
        assert!(config.shared.ends_with(".sprintless/pairs/pair-1/shared"));
        assert!(config.redis_url.is_none());
    }

    #[test]
    fn test_pair_config_with_redis() {
        let config = PairConfig::with_redis(
            "pair-1",
            std::path::Path::new("/project"),
            "redis://localhost",
            "ghp_test",
        );

        assert_eq!(config.pair_id, "pair-1");
        assert!(config.redis_url.is_some());
        assert_eq!(config.redis_url.as_deref(), Some("redis://localhost"));
    }
}