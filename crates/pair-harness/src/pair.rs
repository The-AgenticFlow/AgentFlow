// crates/pair-harness/src/pair.rs
//! ForgeSentinelPair - the main pair lifecycle manager.
//!
//! Implements the v3 event-driven architecture where:
//! - FORGE is a long-running process
//! - SENTINEL is spawned fresh per evaluation
//! - The harness uses inotify for zero-polling event detection

use anyhow::{Context, Result};
use serde_json::Value;
use std::time::{Duration, Instant};
use tokio::process::Child;
use tracing::{debug, error, info, warn};

use crate::isolation::FileLockManager;
use crate::process::{ProcessManager, SentinelMode};
use crate::provision::Provisioner;
use crate::reset::ResetManager;
use crate::types::{FsEvent, PairConfig, PairOutcome, StatusJson, Ticket};
use crate::watchdog::Watchdog;
use crate::watcher::SharedDirWatcher;
use crate::worktree::WorktreeManager;

const SENTINEL_TIMEOUT_SECS: u64 = 120;
const FORGE_STARTUP_TIMEOUT_SECS: u64 = 300; // 5 minutes to write PLAN.md

struct SentinelTracker {
    mode: SentinelMode,
    spawn_time: Instant,
    child: Child,
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
    ticket_id: String,
    plan_approved: bool,
    final_approved: bool,
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
            ticket_id: String::new(),
            plan_approved: false,
            final_approved: false,
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
        self.ticket_id = ticket.id.clone();

        // Check if this is a resume with existing approved plan
        let contract_path = self.config.shared.join("CONTRACT.md");
        if contract_path.exists() {
            if let Ok(content) = tokio::fs::read_to_string(&contract_path).await {
                if content.contains("status: AGREED") || content.contains("status: \"AGREED\"") {
                    self.plan_approved = true;
                    info!("Resuming with approved plan - skipping plan review phase");
                }
            }
        }

        // Check if this is a resume with existing final approval
        let final_review_path = self.config.shared.join("final-review.md");
        if final_review_path.exists() {
            if let Ok(content) = tokio::fs::read_to_string(&final_review_path).await {
                if content.contains("APPROVED") {
                    self.final_approved = true;
                    info!("Resuming with final approval - FORGE should create PR");
                }
            }
        }

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
            // Check if SENTINEL has already exited.
            if let Some(tracker) = &mut self.sentinel_tracker {
                match tracker.child.try_wait() {
                    Ok(Some(status)) => {
                        let mode = tracker.mode.clone();
                        if status.success() {
                            self.materialize_sentinel_artifact(&mode).await?;
                        } else {
                            warn!(
                                mode = ?mode,
                                exit_code = ?status.code(),
                                "SENTINEL exited with error before producing a watched artifact"
                            );
                        }
                        self.sentinel_tracker = None;
                    }
                    Ok(None) => {}
                    Err(e) => {
                        warn!(mode = ?tracker.mode, error = %e, "Failed to poll SENTINEL status");
                        self.sentinel_tracker = None;
                    }
                }
            }

            // Check for SENTINEL timeout
            if let Some(tracker) = &mut self.sentinel_tracker {
                if tracker.spawn_time.elapsed().as_secs() > SENTINEL_TIMEOUT_SECS {
                    warn!(
                        mode = ?tracker.mode,
                        "SENTINEL timed out after {}s",
                        SENTINEL_TIMEOUT_SECS
                    );
                    let _ = self.process.kill(&mut tracker.child).await;
                    self.sentinel_tracker = None;
                }
            }

            // Check for FORGE startup timeout (no PLAN.md written)
            let plan_path = self.config.shared.join("PLAN.md");
            if !plan_path.exists()
                && self.forge_spawn_time.elapsed().as_secs() > FORGE_STARTUP_TIMEOUT_SECS
            {
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
                        // Only spawn SENTINEL for plan review if plan hasn't been approved yet
                        if !self.plan_approved && self.sentinel_tracker.is_none() {
                            info!("PLAN.md written - spawning SENTINEL for plan review");
                            self.spawn_sentinel_for_plan().await?;
                        } else if self.plan_approved {
                            debug!("PLAN.md written but plan already approved - ignoring");
                        } else {
                            warn!("SENTINEL already active - skipping duplicate spawn");
                        }
                    }

                    FsEvent::ContractWritten => {
                        self.sentinel_tracker = None;
                        let status = self.read_contract_status().await?;
                        if status == "AGREED" {
                            self.plan_approved = true;
                            info!("Contract agreed - respawning FORGE to begin implementation");
                            self.process.kill(forge).await?;
                            *forge = self.spawn_forge_resume().await?;
                            self.reset.increment_reset();
                        } else {
                            info!("Contract has issues - FORGE must revise plan");
                        }
                    }

                    FsEvent::WorklogUpdated => {
                        if self.all_segments_approved().await? {
                            info!("All segments complete - spawning SENTINEL for final review");
                            self.spawn_sentinel_for_final().await?;
                        } else if let Some(segment_n) = self.next_segment_to_eval().await? {
                            info!(
                                "Spawning SENTINEL for segment {} eval",
                                segment_n
                            );
                            self.spawn_sentinel_for_segment(segment_n).await?;
                        }
                        self.watchdog.reset();
                    }

                    FsEvent::SegmentEvalWritten(n) => {
                        self.sentinel_tracker = None;
                        info!("Segment {} evaluation complete", n);
                        
                        // Check if this was the last segment - if so, spawn final review
                        if self.all_segments_approved().await? {
                            info!("All segments approved - spawning SENTINEL for final review");
                            self.spawn_sentinel_for_final().await?;
                        }
                    }

                    FsEvent::FinalReviewWritten => {
                        self.sentinel_tracker = None;
                        let verdict = self.read_final_review_verdict().await?;
                        if verdict == "APPROVED" {
                            self.final_approved = true;
                            info!("Final review APPROVED - respawning FORGE to create PR");
                            self.process.kill(forge).await?;
                            *forge = self.spawn_forge_for_pr().await?;
                            self.reset.increment_reset();
                        } else {
                            info!("Final review REJECTED - FORGE must fix issues");
                        }
                    }

                    FsEvent::StatusJsonWritten => {
                        self.sentinel_tracker = None;
                        if let Some(status) = self.read_status().await? {
                            return Ok(status);
                        }
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
                // Drain any pending watcher events first - FORGE may have written
                // files just before exiting and the events may not have been
                // processed yet in the event-handling section above.
                while let Some(evt) = watcher.try_recv() {
                    match evt {
                        FsEvent::PlanWritten => {
                            if !self.plan_approved && self.sentinel_tracker.is_none() {
                                info!("PLAN.md written (drained after FORGE exit) - spawning SENTINEL for plan review");
                                self.spawn_sentinel_for_plan().await?;
                            }
                        }
                        FsEvent::ContractWritten => {
                            self.sentinel_tracker = None;
                            let status = self.read_contract_status().await?;
                            if status == "AGREED" {
                                self.plan_approved = true;
                                info!("Contract agreed (drained after FORGE exit) - respawning FORGE to begin implementation");
                                self.process.kill(forge).await?;
                                *forge = self.spawn_forge_resume().await?;
                                self.reset.increment_reset();
                            }
                        }
                        FsEvent::WorklogUpdated => {
                            if self.all_segments_approved().await? {
                                self.spawn_sentinel_for_final().await?;
                            } else if let Some(segment_n) = self.next_segment_to_eval().await? {
                                self.spawn_sentinel_for_segment(segment_n).await?;
                            }
                            self.watchdog.reset();
                        }
                        FsEvent::SegmentEvalWritten(n) => {
                            self.sentinel_tracker = None;
                            info!("Segment {} evaluation complete (drained)", n);
                            if self.all_segments_approved().await? {
                                self.spawn_sentinel_for_final().await?;
                            }
                        }
                        FsEvent::FinalReviewWritten => {
                            self.sentinel_tracker = None;
                            let verdict = self.read_final_review_verdict().await?;
                            if verdict == "APPROVED" {
                                self.final_approved = true;
                                info!("Final review APPROVED (drained) - respawning FORGE to create PR");
                                *forge = self.spawn_forge_for_pr().await?;
                                self.reset.increment_reset();
                            }
                        }
                        FsEvent::StatusJsonWritten => {
                            if let Some(status) = self.read_status().await? {
                                return Ok(status);
                            }
                        }
                        FsEvent::HandoffWritten => {
                            self.sentinel_tracker = None;
                        }
                    }
                }

                // After draining events, re-evaluate state based on filesystem
                if self.reset.has_handoff() {
                    info!("FORGE exited with handoff - respawning");
                    *forge = self.spawn_forge_resume().await?;
                    self.reset.increment_reset();
                } else if self.config.shared.join("STATUS.json").exists() {
                    if let Some(status) = self.read_status().await? {
                        return Ok(status);
                    }
                } else if self.has_progress_files().await {
                    // FORGE made progress - determine what SENTINEL action is needed
                    if self.sentinel_tracker.is_some() {
                        info!("FORGE exited but SENTINEL is active - waiting for completion");
                        tokio::time::sleep(Duration::from_secs(5)).await;
                    } else {
                        // Check the lifecycle phase and spawn SENTINEL if needed
                        let plan_exists = self.config.shared.join("PLAN.md").exists();
                        let contract_exists = self.config.shared.join("CONTRACT.md").exists();
                        let worklog_exists = self.config.shared.join("WORKLOG.md").exists();
                        let final_review_exists = self.config.shared.join("final-review.md").exists();

                        if plan_exists && !contract_exists && !self.plan_approved {
                            // Plan written but not reviewed - spawn SENTINEL
                            info!("FORGE exited after writing PLAN.md - spawning SENTINEL for plan review");
                            self.spawn_sentinel_for_plan().await?;
                        } else if contract_exists && self.plan_approved && !worklog_exists {
                            // Contract agreed but no implementation yet - respawn FORGE to implement
                            info!("FORGE exited, contract agreed - respawning FORGE to begin implementation");
                            *forge = self.spawn_forge_resume().await?;
                            self.reset.increment_reset();
                        } else if worklog_exists {
                            // Implementation in progress - check segment status
                            if self.all_segments_approved().await? {
                                if !final_review_exists {
                                    info!("FORGE exited, all segments approved - spawning SENTINEL for final review");
                                    self.spawn_sentinel_for_final().await?;
                                }
                            } else if let Some(segment_n) = self.next_segment_to_eval().await? {
                                info!("FORGE exited - spawning SENTINEL for segment {} eval", segment_n);
                                self.spawn_sentinel_for_segment(segment_n).await?;
                            } else {
                                info!("FORGE exited with partial worklog - respawning to continue implementation");
                                *forge = self.spawn_forge_resume().await?;
                                self.reset.increment_reset();
                            }
                        } else {
                            // No clear state - respawn
                            info!("FORGE exited after making progress - respawning to continue");
                            *forge = self.spawn_forge_resume().await?;
                            self.reset.increment_reset();
                        }
                    }
                } else {
                    // No progress files - check if FORGE just started and may not have had time
                    let forge_uptime = self.forge_spawn_time.elapsed().as_secs();
                    if forge_uptime < 30 {
                        // Very quick exit - likely a startup error, retry
                        warn!("FORGE exited quickly ({}s) without progress - retrying spawn", forge_uptime);
                        *forge = self.spawn_forge().await?;
                    } else {
                        // Ran for a while but produced nothing - synthesize handoff and respawn
                        warn!("FORGE exited unexpectedly after {}s without progress - synthesizing handoff", forge_uptime);
                        self.reset.synthesize_handoff().await?;
                        *forge = self.spawn_forge_resume().await?;
                        self.reset.increment_reset();
                    }
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
        self.worktree
            .create_worktree(&self.config.pair_id, &ticket.id)
            .context("Failed to create worktree")?;
        Ok(())
    }

    /// Provision configuration files.
    async fn provision_config(&self, _ticket: &Ticket) -> Result<()> {
        // Use project_root where .sprintless/plugin exists
        let provisioner = Provisioner::new(&self.config.project_root);

        provisioner
            .provision_pair(
                &self.config.pair_id,
                &self.config.worktree,
                &self.config.shared,
                &self.config.github_token,
                self.config.redis_url.as_deref(),
            )
            .await
    }

    /// Seed initial file locks for the ticket.
    async fn seed_locks(&self, ticket: &Ticket) -> Result<()> {
        self.locks
            .seed_locks(&ticket.touched_files, &self.config.pair_id)?;
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
        self.process
            .spawn_forge(
                &self.config.pair_id,
                &self.ticket_id,
                &self.config.worktree,
                &self.config.shared,
            )
            .await
    }

    /// Spawn FORGE process in resume mode.
    async fn spawn_forge_resume(&mut self) -> Result<Child> {
        self.forge_spawn_time = Instant::now();
        self.process
            .spawn_forge_resume(
                &self.config.pair_id,
                &self.ticket_id,
                &self.config.worktree,
                &self.config.shared,
            )
            .await
    }

    /// Spawn FORGE process for PR creation after final approval.
    async fn spawn_forge_for_pr(&mut self) -> Result<Child> {
        self.forge_spawn_time = Instant::now();
        self.process
            .spawn_forge_for_pr(
                &self.config.pair_id,
                &self.ticket_id,
                &self.config.worktree,
                &self.config.shared,
            )
            .await
    }

    /// Spawn SENTINEL for plan review.
    async fn spawn_sentinel_for_plan(&mut self) -> Result<()> {
        let child = self
            .process
            .spawn_sentinel(
                &self.config.pair_id,
                &self.ticket_id,
                SentinelMode::PlanReview,
                &self.config.worktree,
                &self.config.shared,
            )
            .await?;

        self.sentinel_tracker = Some(SentinelTracker {
            mode: SentinelMode::PlanReview,
            spawn_time: Instant::now(),
            child,
        });

        Ok(())
    }

    /// Spawn SENTINEL for segment evaluation.
    async fn spawn_sentinel_for_segment(&mut self, segment: u32) -> Result<()> {
        let child = self
            .process
            .spawn_sentinel(
                &self.config.pair_id,
                &self.ticket_id,
                SentinelMode::SegmentEval(segment),
                &self.config.worktree,
                &self.config.shared,
            )
            .await?;

        self.sentinel_tracker = Some(SentinelTracker {
            mode: SentinelMode::SegmentEval(segment),
            spawn_time: Instant::now(),
            child,
        });

        Ok(())
    }

    /// Spawn SENTINEL for final review.
    async fn spawn_sentinel_for_final(&mut self) -> Result<()> {
        // Don't spawn if final review already done
        if self.config.shared.join("final-review.md").exists() {
            debug!("Final review already exists - skipping spawn");
            return Ok(());
        }

        info!("Spawning SENTINEL for final review");
        let child = self
            .process
            .spawn_sentinel(
                &self.config.pair_id,
                &self.ticket_id,
                SentinelMode::FinalReview,
                &self.config.worktree,
                &self.config.shared,
            )
            .await?;

        self.sentinel_tracker = Some(SentinelTracker {
            mode: SentinelMode::FinalReview,
            spawn_time: Instant::now(),
            child,
        });

        Ok(())
    }

    /// Check if all segments from PLAN.md are approved.
    async fn all_segments_approved(&self) -> Result<bool> {
        let plan_path = self.config.shared.join("PLAN.md");
        if !plan_path.exists() {
            return Ok(false);
        }

        let content = tokio::fs::read_to_string(&plan_path).await?;

        // Count segments in PLAN.md
        let total_segments = content
            .lines()
            .filter(|line| line.starts_with("## Segment") || line.starts_with("### Segment"))
            .count();

        if total_segments == 0 {
            // If no segments defined, check if WORKLOG.md exists (implementation done)
            return Ok(self.config.shared.join("WORKLOG.md").exists());
        }

        // Count approved segment evaluations
        let mut approved_count = 0;
        for n in 1..=total_segments as u32 {
            let eval_path = self.config.shared.join(format!("segment-{}-eval.md", n));
            if eval_path.exists() {
                let eval_content = tokio::fs::read_to_string(&eval_path).await?;
                if eval_content.contains("APPROVED") {
                    approved_count += 1;
                }
            }
        }

        Ok(approved_count >= total_segments as u32)
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

        let mut latest = 0;
        for line in content.lines() {
            if line.starts_with("## Segment") || line.starts_with("### Segment") {
                if let Some(n) = line
                    .split_whitespace()
                    .nth(2)
                    .and_then(|s| s.trim_end_matches(':').parse::<u32>().ok())
                {
                    latest = n;
                }
            }
        }

        Ok(latest)
    }

    /// Find the next segment number that needs SENTINEL evaluation.
    /// Returns None if no segments need evaluation or if WORKLOG.md doesn't exist.
    async fn next_segment_to_eval(&self) -> Result<Option<u32>> {
        let worklog_path = self.config.shared.join("WORKLOG.md");
        if !worklog_path.exists() {
            return Ok(None);
        }

        let content = tokio::fs::read_to_string(&worklog_path).await?;

        let mut segments_in_worklog: Vec<u32> = Vec::new();
        for line in content.lines() {
            if line.starts_with("## Segment") || line.starts_with("### Segment") {
                if let Some(n) = line
                    .split_whitespace()
                    .nth(2)
                    .and_then(|s| s.trim_end_matches(':').parse::<u32>().ok())
                {
                    segments_in_worklog.push(n);
                }
            }
        }

        for n in &segments_in_worklog {
            let eval_path = self.config.shared.join(format!("segment-{}-eval.md", n));
            if !eval_path.exists() {
                return Ok(Some(*n));
            }
        }

        Ok(None)
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
    /// Returns `Ok(None)` if the file exists but is empty (race: inotify fires before flush).
    async fn read_status(&self) -> Result<Option<PairOutcome>> {
        let path = self.config.shared.join("STATUS.json");
        if !path.exists() {
            return Ok(None);
        }

        let content = tokio::fs::read_to_string(&path).await?;

        if content.trim().is_empty() {
            return Ok(None);
        }

        let status: StatusJson = serde_json::from_str(&content)?;

        Ok(Some(match status.status.as_str() {
            "PR_OPENED" => PairOutcome::PrOpened {
                pr_url: status.pr_url.clone().unwrap_or_default(),
                pr_number: status.pr_number.unwrap_or(0),
                branch: status.branch.clone().unwrap_or_default(),
            },
            "COMPLETED" | "complete" | "completed" => {
                if status.pr_url.is_some() && !status.pr_url.as_ref().unwrap().is_empty() {
                    PairOutcome::PrOpened {
                        pr_url: status.pr_url.clone().unwrap_or_default(),
                        pr_number: status.pr_number.unwrap_or(0),
                        branch: status.branch.clone().unwrap_or_default(),
                    }
                } else {
                    PairOutcome::Blocked {
                        reason: "Work complete but PR not created - needs push/PR creation"
                            .to_string(),
                        blockers: vec![],
                    }
                }
            }
            "IMPLEMENTATION_COMPLETE" => PairOutcome::Blocked {
                reason: "Implementation complete but PR not created - needs push/PR creation"
                    .to_string(),
                blockers: vec![],
            },
            "BLOCKED" => PairOutcome::Blocked {
                reason: "See blockers".to_string(),
                blockers: status.blockers,
            },
            "FUEL_EXHAUSTED" => PairOutcome::FuelExhausted {
                reason: "Fuel exhausted".to_string(),
                reset_count: status.context_resets,
            },
            _ => PairOutcome::FuelExhausted {
                reason: format!("Unknown status: {}", status.status),
                reset_count: self.reset.reset_count(),
            },
        }))
    }

    /// Check if FORGE has made progress (PLAN.md or WORKLOG.md exists).
    async fn has_progress_files(&self) -> bool {
        let plan_path = self.config.shared.join("PLAN.md");
        let worklog_path = self.config.shared.join("WORKLOG.md");

        plan_path.exists() || worklog_path.exists()
    }

    /// Check if we're waiting for SENTINEL output (plan reviewed but no contract).
    #[allow(dead_code)]
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
                    let eval_path = self
                        .config
                        .shared
                        .join(format!("segment-{}-eval.md", segment));
                    if !eval_path.exists() {
                        return true;
                    }
                }
            }
        }

        false
    }

    async fn materialize_sentinel_artifact(&self, mode: &SentinelMode) -> Result<()> {
        match mode {
            SentinelMode::PlanReview => {
                let output_path = self.config.shared.join("CONTRACT.md");
                if output_path.exists() {
                    return Ok(());
                }

                if let Some(content) = self.read_sentinel_result_payload(mode).await? {
                    tokio::fs::write(&output_path, content)
                        .await
                        .context("Failed to write CONTRACT.md from SENTINEL stdout")?;
                    info!(path = %output_path.display(), "Materialized CONTRACT.md from SENTINEL stdout");
                }
            }
            SentinelMode::SegmentEval(segment) => {
                let output_path = self
                    .config
                    .shared
                    .join(format!("segment-{}-eval.md", segment));
                if output_path.exists() {
                    return Ok(());
                }

                if let Some(content) = self.read_sentinel_result_payload(mode).await? {
                    tokio::fs::write(&output_path, content)
                        .await
                        .context("Failed to write segment eval from SENTINEL stdout")?;
                    info!(path = %output_path.display(), "Materialized segment eval from SENTINEL stdout");
                }
            }
            SentinelMode::FinalReview => {
                let output_path = self.config.shared.join("final-review.md");
                if output_path.exists() {
                    return Ok(());
                }

                if let Some(content) = self.read_sentinel_result_payload(mode).await? {
                    tokio::fs::write(&output_path, content)
                        .await
                        .context("Failed to write final-review.md from SENTINEL stdout")?;
                    info!(path = %output_path.display(), "Materialized final-review.md from SENTINEL stdout");
                }
            }
        }

        Ok(())
    }

    async fn read_sentinel_result_payload(&self, mode: &SentinelMode) -> Result<Option<String>> {
        let log_path = self
            .config
            .shared
            .join("logs")
            .join(format!("sentinel-{}-stdout.log", format!("{:?}", mode)));

        if !log_path.exists() {
            return Ok(None);
        }

        let content = tokio::fs::read_to_string(&log_path)
            .await
            .context("Failed to read SENTINEL stdout log")?;

        if content.trim().is_empty() {
            return Ok(None);
        }

        let last_line = content.lines().rev().find(|line| !line.trim().is_empty());

        let Some(last_line) = last_line else {
            return Ok(None);
        };

        let value: Value = match serde_json::from_str(last_line) {
            Ok(v) => v,
            Err(e) => {
                warn!(
                    mode = ?mode,
                    error = %e,
                    "Failed to parse SENTINEL stdout JSON - SENTINEL may not have produced structured output"
                );
                return Ok(None);
            }
        };
        let result_text = value
            .get("result")
            .and_then(Value::as_str)
            .unwrap_or("")
            .trim();
        if result_text.is_empty() {
            return Ok(None);
        }

        Ok(Self::extract_result_block(result_text).or_else(|| Some(result_text.to_string())))
    }

    fn extract_result_block(result_text: &str) -> Option<String> {
        let start = result_text.find("<result>")?;
        let end = result_text.rfind("</result>")?;
        let inner = &result_text[start + "<result>".len()..end];
        Some(inner.trim().to_string())
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
    use tempfile::tempdir;

    #[test]
    fn test_pair_config_creation() {
        let config = PairConfig::new("pair-1", std::path::Path::new("/project"), "ghp_test");

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

    #[test]
    fn test_extract_result_block() {
        let text = "<result>\nstatus: AGREED\nsummary: ok\n</result>";
        let extracted = ForgeSentinelPair::extract_result_block(text).unwrap();
        assert_eq!(extracted, "status: AGREED\nsummary: ok");
    }

    #[tokio::test]
    async fn test_read_sentinel_result_payload_from_stdout_log() {
        let dir = tempdir().unwrap();
        let config = PairConfig::new("pair-1", dir.path(), "ghp_test");
        let pair = ForgeSentinelPair::new(config.clone());

        let logs_dir = config.shared.join("logs");
        std::fs::create_dir_all(&logs_dir).unwrap();
        std::fs::write(
            logs_dir.join("sentinel-PlanReview-stdout.log"),
            "{\"result\":\"<result>\\nstatus: AGREED\\nsummary: ok\\n</result>\"}\n",
        )
        .unwrap();

        let payload = pair
            .read_sentinel_result_payload(&SentinelMode::PlanReview)
            .await
            .unwrap()
            .unwrap();

        assert_eq!(payload, "status: AGREED\nsummary: ok");
    }
}
