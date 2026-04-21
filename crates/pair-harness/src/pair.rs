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
use crate::types::{
    Complexity, FsEvent, PairConfig, PairOutcome, StatusJson, Ticket, TimeoutProfile,
};
use crate::watchdog::Watchdog;
use crate::watcher::SharedDirWatcher;
use crate::worktree::{MergeMainResult, WorktreeManager};

const DEFAULT_SENTINEL_TIMEOUT_SECS: u64 = 120;
const FORGE_STARTUP_TIMEOUT_SECS: u64 = 300; // 5 minutes to write PLAN.md
const MAX_SENTINEL_RETRIES: u32 = 2;

const ENV_OVERHEAD_NETWORK_SECS: u64 = 15;
const ENV_OVERHEAD_STREAMING_SECS: u64 = 10;
const ENV_OVERHEAD_BUILD_SECS: u64 = 30;
const ENV_OVERHEAD_BUFFER_SECS: u64 = 20;

fn compute_effective_timeout(base_secs: u64, complexity: &Complexity) -> u64 {
    let overhead =
        ENV_OVERHEAD_NETWORK_SECS + ENV_OVERHEAD_STREAMING_SECS + ENV_OVERHEAD_BUFFER_SECS;
    let build_overhead = match complexity {
        Complexity::Low => ENV_OVERHEAD_BUILD_SECS / 2,
        Complexity::Medium => ENV_OVERHEAD_BUILD_SECS,
        Complexity::High => ENV_OVERHEAD_BUILD_SECS * 2,
    };
    base_secs + overhead + build_overhead
}

struct SentinelTracker {
    mode: SentinelMode,
    spawn_time: Instant,
    child: Child,
    timeout_secs: u64,
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
    sentinel_retries: u32,
    ticket_id: String,
    plan_approved: bool,
    final_approved: bool,
    contract_timeout: Option<TimeoutProfile>,
}

impl ForgeSentinelPair {
    /// Create a new ForgeSentinelPair.
    pub fn new(config: PairConfig) -> Self {
        // Use the project_root from config (contains .git)
        let project_root = config.project_root.clone();

        Self {
            worktree: WorktreeManager::new(&project_root),
            locks: FileLockManager::new(&project_root),
            process: match (&config.redis_url, &config.proxy_url) {
                (Some(redis_url), Some(proxy_url)) => ProcessManager::with_proxy(
                    &config.github_token,
                    Some(redis_url.clone()),
                    proxy_url,
                ),
                (Some(redis_url), None) => {
                    ProcessManager::with_redis(&config.github_token, redis_url)
                }
                (None, Some(proxy_url)) => {
                    ProcessManager::with_proxy(&config.github_token, None, proxy_url)
                }
                (None, None) => ProcessManager::new(&config.github_token),
            },
            reset: ResetManager::new(config.shared.clone(), config.max_resets),
            watchdog: Watchdog::new(config.shared.clone(), config.watchdog_timeout_secs),
            config,
            start_time: Instant::now(),
            sentinel_tracker: None,
            forge_spawn_time: Instant::now(),
            sentinel_retries: 0,
            ticket_id: String::new(),
            plan_approved: false,
            final_approved: false,
            contract_timeout: None,
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
                    self.contract_timeout = Self::parse_timeout_profile(&content);
                    info!(timeout = ?self.contract_timeout, "Resuming with approved plan - skipping plan review phase");
                }
            }
        }

        // Check if this is a conflict rework — skip plan review, go straight to implementation
        let conflict_resolution_path = self.config.shared.join("CONFLICT_RESOLUTION.md");
        if conflict_resolution_path.exists() {
            self.plan_approved = true;
            self.final_approved = true;
            info!(
                pair = %self.config.pair_id,
                "CONFLICT_RESOLUTION.md detected — skipping plan/final review, forge will resolve conflicts"
            );
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

        // Check if all segments are already approved on resume
        if self.plan_approved && self.all_segments_approved().await? {
            if final_review_path.exists() {
                self.final_approved = true;
                info!("All segments approved and final review exists on resume");
            } else {
                info!("All segments approved on resume - will proceed to final review");
            }
        }

        // 1. Provision worktree (reuses existing if on correct branch)
        self.provision_worktree(ticket).await?;

        // 1b. If conflict rework: merge origin/main into worktree so conflicts are visible to FORGE
        if conflict_resolution_path.exists() {
            match self.worktree.merge_origin_main(&self.config.worktree) {
                Ok(MergeMainResult::Clean) => {
                    info!(
                        pair = %self.config.pair_id,
                        "origin/main merged cleanly during conflict rework — force-pushing updated branch to remote"
                    );
                    if let Err(e) = self.worktree.force_push_branch(&self.config.worktree) {
                        warn!(
                            pair = %self.config.pair_id,
                            error = %e,
                            "Failed to force-push after clean merge — GitHub PR may still show conflicts"
                        );
                    }
                }
                Ok(MergeMainResult::Conflict { conflicted_files }) => {
                    info!(
                        pair = %self.config.pair_id,
                        files = conflicted_files.len(),
                        "Conflict markers materialized in worktree — FORGE will resolve"
                    );
                }
                Err(e) => {
                    warn!(
                        pair = %self.config.pair_id,
                        error = %e,
                        "Failed to merge origin/main into worktree for conflict rework — FORGE may not see conflicts"
                    );
                }
            }
        }

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
                            self.sentinel_retries = 0;
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
                if tracker.spawn_time.elapsed().as_secs() > tracker.timeout_secs {
                    warn!(
                        mode = ?tracker.mode,
                        "SENTINEL timed out after {}s",
                        tracker.timeout_secs
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
                            self.read_contract_timeout_profile().await?;
                            info!(
                                timeout = ?self.contract_timeout,
                                "Contract agreed - respawning FORGE to begin implementation"
                            );
                            self.process.kill(forge).await?;
                            *forge = self.spawn_forge_resume().await?;
                        } else {
                            info!("Contract has issues - FORGE must revise plan");
                        }
                    }

                    FsEvent::WorklogUpdated => {
                        if self.all_segments_approved().await? {
                            info!("All segments complete - spawning SENTINEL for final review");
                            self.spawn_sentinel_for_final().await?;
                        } else if let Some(segment_n) = self.next_segment_to_eval().await? {
                            info!("Spawning SENTINEL for segment {} eval", segment_n);
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
            if self.start_time.elapsed().as_secs().wrapping_rem(60) == 0 {
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
                                self.read_contract_timeout_profile().await?;
                                info!(timeout = ?self.contract_timeout, "Contract agreed (drained after FORGE exit) - respawning FORGE to begin implementation");
                                self.process.kill(forge).await?;
                                *forge = self.spawn_forge_resume().await?;
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
                        let final_review_exists =
                            self.config.shared.join("final-review.md").exists();

                        if plan_exists && !contract_exists && !self.plan_approved {
                            // Plan written but not reviewed - spawn SENTINEL
                            info!("FORGE exited after writing PLAN.md - spawning SENTINEL for plan review");
                            self.spawn_sentinel_for_plan().await?;
                        } else if contract_exists && self.plan_approved && !worklog_exists {
                            // Contract agreed but no implementation yet - respawn FORGE to implement
                            info!("FORGE exited, contract agreed - respawning FORGE to begin implementation");
                            *forge = self.spawn_forge_resume().await?;
                        } else if worklog_exists {
                            // Implementation in progress - check segment status
                            if self.all_segments_approved().await? {
                                if !final_review_exists {
                                    info!("FORGE exited, all segments approved - spawning SENTINEL for final review");
                                    self.spawn_sentinel_for_final().await?;
                                }
                            } else if let Some(segment_n) = self.next_segment_to_eval().await? {
                                info!(
                                    "FORGE exited - spawning SENTINEL for segment {} eval",
                                    segment_n
                                );
                                self.spawn_sentinel_for_segment(segment_n).await?;
                            } else if self.segments_remaining_in_plan().await?.is_some() {
                                // Written segments have evals, but PLAN has more segments to implement.
                                // This is expected with --print mode: FORGE exits after each segment,
                                // and we just respawn to continue the next one.
                                info!("FORGE exited after segment work - respawning to continue implementation");
                                *forge = self.spawn_forge_resume().await?;
                            } else {
                                // No segments remaining in plan, no eval needed, but not all approved.
                                // This is a genuine stale state - count as a reset.
                                info!("FORGE exited with partial worklog - respawning to continue implementation");
                                *forge = self.spawn_forge_resume().await?;
                                self.reset.increment_reset();
                            }
                        } else {
                            // No clear state - respawn
                            info!("FORGE exited after making progress - respawning to continue");
                            *forge = self.spawn_forge_resume().await?;
                        }
                    }
                } else {
                    // No progress files - check if FORGE just started and may not have had time
                    let forge_uptime = self.forge_spawn_time.elapsed().as_secs();
                    if forge_uptime < 30 {
                        // Very quick exit - likely a startup error, retry
                        warn!(
                            "FORGE exited quickly ({}s) without progress - retrying spawn",
                            forge_uptime
                        );
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
    async fn provision_worktree(&mut self, ticket: &Ticket) -> Result<()> {
        let worktree_path = self
            .worktree
            .create_worktree(&self.config.pair_id, &ticket.id)
            .context("Failed to create worktree")?;
        self.config.worktree = worktree_path;
        Ok(())
    }

    /// Provision configuration files.
    async fn provision_config(&self, _ticket: &Ticket) -> Result<()> {
        // Use project_root where orchestration/plugin exists
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

        let conflict_path = self.config.shared.join("CONFLICT_RESOLUTION.md");
        let task = if conflict_path.exists() {
            format!(
                "Resolve merge conflicts for ticket {}.\n\n\
                 Branch: {}\n\n\
                 CONFLICT_RESOLUTION.md in this directory contains detailed instructions.\n\
                 Resolve all conflict markers, commit, then force-push with 'git push --force-with-lease origin HEAD' (the branch has diverged due to the merge of origin/main).\n\
                 If a PR already exists for this branch, do NOT create a new one — just push and update STATUS.json.\n\
                 Write STATUS.json with status PR_OPENED, the existing PR URL if known, or create a new PR only if none exists.",
                ticket.id,
                WorktreeManager::branch_name(&self.config.pair_id, &ticket.id)
            )
        } else {
            format!(
                "Implement ticket {}.\n\nBranch: {}\n\nWhen done, open a PR and write STATUS.json.",
                ticket.id,
                WorktreeManager::branch_name(&self.config.pair_id, &ticket.id)
            )
        };
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

    /// Resolve the effective timeout for a SENTINEL evaluation based on mode and contract.
    fn resolve_sentinel_timeout(&self, mode: &SentinelMode) -> u64 {
        let (base_secs, complexity) = match &self.contract_timeout {
            Some(profile) => {
                let base = match mode {
                    SentinelMode::PlanReview => profile.plan_review_secs,
                    SentinelMode::SegmentEval(_) => profile.segment_eval_secs,
                    SentinelMode::FinalReview => profile.final_review_secs,
                };
                (base, profile.complexity.clone())
            }
            None => (DEFAULT_SENTINEL_TIMEOUT_SECS, Complexity::Medium),
        };
        compute_effective_timeout(base_secs, &complexity)
    }

    /// Spawn SENTINEL for plan review.
    async fn spawn_sentinel_for_plan(&mut self) -> Result<()> {
        if self.sentinel_retries >= MAX_SENTINEL_RETRIES {
            warn!(
                retries = self.sentinel_retries,
                "SENTINEL plan review exceeded max retries — forcing changes_requested and reset"
            );
            self.reset.increment_reset();
            return Ok(());
        }
        self.sentinel_retries += 1;
        let timeout_secs = self.resolve_sentinel_timeout(&SentinelMode::PlanReview);
        let child = self
            .process
            .spawn_sentinel_with_timeout(
                &self.config.pair_id,
                &self.ticket_id,
                SentinelMode::PlanReview,
                &self.config.worktree,
                &self.config.shared,
                timeout_secs,
            )
            .await?;

        self.sentinel_tracker = Some(SentinelTracker {
            mode: SentinelMode::PlanReview,
            spawn_time: Instant::now(),
            child,
            timeout_secs,
        });

        Ok(())
    }

    /// Spawn SENTINEL for segment evaluation.
    async fn spawn_sentinel_for_segment(&mut self, segment: u32) -> Result<()> {
        if self.sentinel_retries >= MAX_SENTINEL_RETRIES {
            warn!(
                retries = self.sentinel_retries,
                segment,
                "SENTINEL segment eval exceeded max retries — forcing changes_requested and reset"
            );
            self.reset.increment_reset();
            return Ok(());
        }
        self.sentinel_retries += 1;
        let timeout_secs = self.resolve_sentinel_timeout(&SentinelMode::SegmentEval(segment));
        let child = self
            .process
            .spawn_sentinel_with_timeout(
                &self.config.pair_id,
                &self.ticket_id,
                SentinelMode::SegmentEval(segment),
                &self.config.worktree,
                &self.config.shared,
                timeout_secs,
            )
            .await?;

        self.sentinel_tracker = Some(SentinelTracker {
            mode: SentinelMode::SegmentEval(segment),
            spawn_time: Instant::now(),
            child,
            timeout_secs,
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

        if self.sentinel_retries >= MAX_SENTINEL_RETRIES {
            warn!(
                retries = self.sentinel_retries,
                "SENTINEL final review exceeded max retries — forcing changes_requested and reset"
            );
            self.reset.increment_reset();
            return Ok(());
        }
        self.sentinel_retries += 1;
        info!("Spawning SENTINEL for final review");
        let timeout_secs = self.resolve_sentinel_timeout(&SentinelMode::FinalReview);
        let child = self
            .process
            .spawn_sentinel_with_timeout(
                &self.config.pair_id,
                &self.ticket_id,
                SentinelMode::FinalReview,
                &self.config.worktree,
                &self.config.shared,
                timeout_secs,
            )
            .await?;

        self.sentinel_tracker = Some(SentinelTracker {
            mode: SentinelMode::FinalReview,
            spawn_time: Instant::now(),
            child,
            timeout_secs,
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

    /// Parse the timeout_profile section from CONTRACT.md and store it.
    async fn read_contract_timeout_profile(&mut self) -> Result<()> {
        let path = self.config.shared.join("CONTRACT.md");
        if !path.exists() {
            return Ok(());
        }

        let content = tokio::fs::read_to_string(&path).await?;
        self.contract_timeout = Self::parse_timeout_profile(&content);
        Ok(())
    }

    /// Parse timeout_profile from CONTRACT.md markdown content.
    fn parse_timeout_profile(content: &str) -> Option<TimeoutProfile> {
        if !content.contains("timeout_profile:") {
            return None;
        }

        let mut plan_review_secs: Option<u64> = None;
        let mut segment_eval_secs: Option<u64> = None;
        let mut final_review_secs: Option<u64> = None;
        let mut complexity: Option<Complexity> = None;

        let mut in_profile = false;
        for line in content.lines() {
            let trimmed = line.trim();
            if trimmed.starts_with("timeout_profile:") {
                in_profile = true;
                continue;
            }
            if in_profile {
                if !trimmed.starts_with('-')
                    && !trimmed.is_empty()
                    && !trimmed
                        .chars()
                        .next()
                        .map(|c| c.is_whitespace())
                        .unwrap_or(false)
                    && !trimmed.starts_with("plan_review")
                    && !trimmed.starts_with("segment_eval")
                    && !trimmed.starts_with("final_review")
                    && !trimmed.starts_with("complexity")
                {
                    in_profile = false;
                    continue;
                }
                if let Some(rest) = trimmed.strip_prefix("plan_review_secs:") {
                    plan_review_secs = rest.trim().trim_end_matches(',').parse().ok();
                } else if let Some(rest) = trimmed.strip_prefix("segment_eval_secs:") {
                    segment_eval_secs = rest.trim().trim_end_matches(',').parse().ok();
                } else if let Some(rest) = trimmed.strip_prefix("final_review_secs:") {
                    final_review_secs = rest.trim().trim_end_matches(',').parse().ok();
                } else if let Some(rest) = trimmed.strip_prefix("complexity:") {
                    let val = rest.trim().trim_end_matches(',').to_lowercase();
                    complexity = match val.as_str() {
                        "low" => Some(Complexity::Low),
                        "medium" => Some(Complexity::Medium),
                        "high" => Some(Complexity::High),
                        _ => None,
                    };
                }
            }
        }

        match (
            plan_review_secs,
            segment_eval_secs,
            final_review_secs,
            complexity,
        ) {
            (Some(pr), Some(se), Some(fr), Some(cx)) => Some(TimeoutProfile {
                plan_review_secs: pr,
                segment_eval_secs: se,
                final_review_secs: fr,
                complexity: cx,
            }),
            _ => None,
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

    /// Check if PLAN.md has more segments than WORKLOG.md has written.
    /// Returns Some(count) of remaining segments, or None if PLAN.md doesn't exist
    /// or has no segment headers.
    ///
    /// This is used to distinguish between:
    /// - FORGE exited after completing a segment (--print mode, normal exit)
    ///   where more segments remain to be implemented
    /// - FORGE exited with a genuinely incomplete/partial worklog
    async fn segments_remaining_in_plan(&self) -> Result<Option<u32>> {
        let plan_path = self.config.shared.join("PLAN.md");
        if !plan_path.exists() {
            return Ok(None);
        }

        let plan_content = tokio::fs::read_to_string(&plan_path).await?;
        let total_in_plan: Vec<u32> = plan_content
            .lines()
            .filter(|line| line.starts_with("## Segment") || line.starts_with("### Segment"))
            .filter_map(|line| {
                line.split_whitespace()
                    .nth(2)
                    .and_then(|s| s.trim_end_matches(':').parse::<u32>().ok())
            })
            .collect();

        if total_in_plan.is_empty() {
            return Ok(None);
        }

        let worklog_path = self.config.shared.join("WORKLOG.md");
        if !worklog_path.exists() {
            return Ok(Some(total_in_plan.len() as u32));
        }

        let worklog_content = tokio::fs::read_to_string(&worklog_path).await?;
        let written_segments: std::collections::HashSet<u32> = worklog_content
            .lines()
            .filter(|line| line.starts_with("## Segment") || line.starts_with("### Segment"))
            .filter_map(|line| {
                line.split_whitespace()
                    .nth(2)
                    .and_then(|s| s.trim_end_matches(':').parse::<u32>().ok())
            })
            .collect();

        let remaining = total_in_plan
            .iter()
            .filter(|n| !written_segments.contains(n))
            .count() as u32;

        if remaining > 0 {
            Ok(Some(remaining))
        } else {
            Ok(None)
        }
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
    /// Handles deserialization errors gracefully by logging a warning and returning None,
    /// rather than crashing the entire pair lifecycle.
    async fn read_status(&self) -> Result<Option<PairOutcome>> {
        let path = self.config.shared.join("STATUS.json");
        if !path.exists() {
            return Ok(None);
        }

        let content = tokio::fs::read_to_string(&path).await?;

        if content.trim().is_empty() {
            return Ok(None);
        }

        let status: StatusJson = match serde_json::from_str(&content) {
            Ok(s) => s,
            Err(e) => {
                warn!(
                    error = %e,
                    path = %path.display(),
                    "Failed to parse STATUS.json — renaming to .broken to break respawn loop"
                );
                let broken_path = self.config.shared.join("STATUS.json.broken");
                let _ = tokio::fs::rename(&path, &broken_path).await;
                return Ok(None);
            }
        };

        Ok(Some(match status.status.as_str() {
            "PR_OPENED"
            | "COMPLETE"
            | "complete"
            | "completed"
            | "COMPLETED"
            | "SEGMENTS_COMPLETE"
            | "SEGMENT_COMPLETE_AWAITING_REVIEW"
            | "ALL_SEGMENTS_DONE" => {
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
            "PENDING_REVIEW" => {
                debug!(
                    status = "PENDING_REVIEW",
                    "FORGE requests review — treating as non-terminal, continuing event loop"
                );
                return Ok(None);
            }
            _ => {
                let s = status.status.as_str();
                if s.starts_with("SEGMENT_") && s.ends_with("_DONE") {
                    debug!(status = s, "Intermediate segment status in STATUS.json — treating as non-terminal, continuing event loop");
                    return Ok(None);
                }
                warn!(
                    status = s,
                    "Unrecognized STATUS.json status — treating as fuel exhausted"
                );
                PairOutcome::FuelExhausted {
                    reason: format!("Unknown status: {}", s),
                    reset_count: self.reset.reset_count(),
                }
            }
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
            .join(format!("sentinel-{:?}-stdout.log", mode));

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
        self.locks.release_all_for_pair(&self.config.pair_id)?;

        let conflict_path = self.config.shared.join("CONFLICT_RESOLUTION.md");
        if conflict_path.exists() {
            let _ = tokio::fs::remove_file(&conflict_path).await;
            debug!("Removed CONFLICT_RESOLUTION.md after pair completion");
        }

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
        assert!(config.worktree.starts_with("/project/worktrees/"));
        assert!(config.shared.ends_with("orchestration/pairs/pair-1/shared"));
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

    #[test]
    fn test_parse_timeout_profile_medium() {
        let content = "\
status: AGREED
summary: Add authentication module
definition_of_done:
- Auth middleware implemented
- Tests passing
objections:
- None
timeout_profile:
  plan_review_secs: 120
  segment_eval_secs: 300
  final_review_secs: 480
  complexity: medium";
        let profile = ForgeSentinelPair::parse_timeout_profile(content).unwrap();
        assert_eq!(profile.plan_review_secs, 120);
        assert_eq!(profile.segment_eval_secs, 300);
        assert_eq!(profile.final_review_secs, 480);
        assert_eq!(profile.complexity, Complexity::Medium);
    }

    #[test]
    fn test_parse_timeout_profile_high() {
        let content = "\
status: AGREED
summary: Refactor API layer
timeout_profile:
  plan_review_secs: 180
  segment_eval_secs: 480
  final_review_secs: 720
  complexity: high";
        let profile = ForgeSentinelPair::parse_timeout_profile(content).unwrap();
        assert_eq!(profile.plan_review_secs, 180);
        assert_eq!(profile.segment_eval_secs, 480);
        assert_eq!(profile.final_review_secs, 720);
        assert_eq!(profile.complexity, Complexity::High);
    }

    #[test]
    fn test_parse_timeout_profile_missing() {
        let content = "status: AGREED\nsummary: Simple fix";
        assert!(ForgeSentinelPair::parse_timeout_profile(content).is_none());
    }

    #[test]
    fn test_parse_timeout_profile_partial() {
        let content = "\
status: AGREED
timeout_profile:
  plan_review_secs: 90
  complexity: low";
        assert!(ForgeSentinelPair::parse_timeout_profile(content).is_none());
    }

    #[test]
    fn test_compute_effective_timeout_low() {
        let timeout = compute_effective_timeout(90, &Complexity::Low);
        let expected = 90 + 15 + 10 + 20 + 15; // base + network + streaming + buffer + build_low
        assert_eq!(timeout, expected);
    }

    #[test]
    fn test_compute_effective_timeout_high() {
        let timeout = compute_effective_timeout(480, &Complexity::High);
        let expected = 480 + 15 + 10 + 20 + 60; // base + network + streaming + buffer + build_high
        assert_eq!(timeout, expected);
    }

    #[test]
    fn test_resolve_sentinel_timeout_with_profile() {
        let dir = tempdir().unwrap();
        let config = PairConfig::new("pair-1", dir.path(), "ghp_test");
        let mut pair = ForgeSentinelPair::new(config);
        pair.contract_timeout = Some(TimeoutProfile {
            plan_review_secs: 180,
            segment_eval_secs: 480,
            final_review_secs: 720,
            complexity: Complexity::High,
        });

        let pr_timeout = pair.resolve_sentinel_timeout(&SentinelMode::PlanReview);
        let se_timeout = pair.resolve_sentinel_timeout(&SentinelMode::SegmentEval(1));
        let fr_timeout = pair.resolve_sentinel_timeout(&SentinelMode::FinalReview);

        assert!(pr_timeout > 180);
        assert!(se_timeout > 480);
        assert!(fr_timeout > 720);
    }

    #[test]
    fn test_resolve_sentinel_timeout_fallback() {
        let dir = tempdir().unwrap();
        let config = PairConfig::new("pair-1", dir.path(), "ghp_test");
        let pair = ForgeSentinelPair::new(config);

        let timeout = pair.resolve_sentinel_timeout(&SentinelMode::PlanReview);
        assert!(timeout > DEFAULT_SENTINEL_TIMEOUT_SECS);
    }
}
