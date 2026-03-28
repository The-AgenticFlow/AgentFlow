/// FORGE-SENTINEL Pair Orchestration
///
/// Core struct that manages the complete lifecycle of a pair:
/// - Worktree provisioning
/// - Process spawning (FORGE long-running, SENTINEL ephemeral)
/// - Event-driven watcher
/// - Context resets
/// - Terminal condition detection
use anyhow::{Context, Result};
use git2::Repository;
use std::env;
use std::path::{Path, PathBuf};
use std::time::Instant;
use tokio::sync::mpsc;
use tracing::{error, info, warn};

use crate::artifacts::{Status, StatusJson, Ticket};
use crate::isolation::IsolationManager;
use crate::mcp_config::{setup_mcp_directories, McpConfigGenerator};
use crate::process::{AgentProcess, AgentType, ProcessManager};
use crate::watcher::{PairWatcher, WatchEvent};
use crate::worktree::WorktreeManager;

/// Configuration for a pair
#[derive(Debug, Clone)]
pub struct PairConfig {
    /// Pair identifier (e.g., "pair-1")
    pub pair_id: String,
    /// Main repository path
    pub repo_path: PathBuf,
    /// Redis URL for file locking
    pub redis_url: String,
    /// GitHub token for MCP
    pub github_token: String,
    /// Maximum number of context resets
    pub max_resets: u32,
}

const DEFAULT_REDIS_URL: &str = "redis://localhost:6379";

impl PairConfig {
    /// Builds a pair configuration from values loaded from `.env` and the process environment.
    ///
    /// Resolution order:
    /// - `AGENT_TEST_WORKDIR` (preferred for real/E2E test runs)
    /// - `REDIS_URL` (default: `redis://localhost:6379`)
    /// - `REPO_PATH` (falls back to current working directory, then resolves to the git root)
    /// - `GITHUB_TOKEN`, then `GITHUB_PERSONAL_ACCESS_TOKEN`
    pub fn from_env(pair_id: impl Into<String>, max_resets: u32) -> Result<Self> {
        let _ = dotenvy::dotenv();

        let repo_path = env_var("AGENT_TEST_WORKDIR")
            .or_else(|| env_var("REPO_PATH"))
            .map(PathBuf::from)
            .map_or_else(
                || env::current_dir().context("Failed to determine current directory"),
                Ok,
            )?;

        Ok(Self {
            pair_id: pair_id.into(),
            repo_path: normalize_repo_path(&repo_path)?,
            redis_url: env_var("REDIS_URL").unwrap_or_else(|| DEFAULT_REDIS_URL.to_string()),
            github_token: required_env_var(&["GITHUB_TOKEN", "GITHUB_PERSONAL_ACCESS_TOKEN"])?,
            max_resets,
        })
    }
}

fn env_var(name: &str) -> Option<String> {
    env::var(name)
        .ok()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
}

fn required_env_var(names: &[&str]) -> Result<String> {
    names
        .iter()
        .find_map(|name| env_var(name))
        .with_context(|| format!("Expected one of {} to be set", names.join(", ")))
}

fn normalize_repo_path(repo_path: &Path) -> Result<PathBuf> {
    let repo = Repository::discover(repo_path).with_context(|| {
        format!(
            "Failed to discover git repository from {}",
            repo_path.display()
        )
    })?;

    let root = repo
        .workdir()
        .map(Path::to_path_buf)
        .or_else(|| repo.path().parent().map(Path::to_path_buf))
        .context("Failed to determine repository root")?;

    Ok(root.canonicalize().unwrap_or(root))
}

/// Outcome of a pair execution
#[derive(Debug)]
pub enum PairOutcome {
    /// PR successfully opened
    Success {
        pr_url: String,
        pr_number: u32,
        files_changed: Vec<String>,
        elapsed_ms: u64,
    },
    /// Task blocked, needs human intervention
    Blocked { reason: String, details: String },
    /// Exceeded max context resets
    FuelExhausted { resets: u32, elapsed_ms: u64 },
}

impl PairOutcome {
    pub fn from_status(status: StatusJson, elapsed_ms: u64) -> Self {
        match status.status {
            Status::PrOpened => Self::Success {
                pr_url: status.pr_url.unwrap_or_default(),
                pr_number: status.pr_number.unwrap_or(0),
                files_changed: status.files_changed,
                elapsed_ms,
            },
            Status::Blocked => {
                let reason = status
                    .blockers
                    .first()
                    .map(|b| b.blocker_type.clone())
                    .unwrap_or_else(|| "UNKNOWN".to_string());
                let details = status
                    .blockers
                    .first()
                    .map(|b| b.description.clone())
                    .unwrap_or_default();
                Self::Blocked { reason, details }
            }
            Status::FuelExhausted => Self::FuelExhausted {
                resets: status.context_resets.unwrap_or(0),
                elapsed_ms,
            },
        }
    }
}

/// The FORGE-SENTINEL pair harness
pub struct ForgeSentinelPair {
    config: PairConfig,
    ticket: Ticket,
    worktree_mgr: WorktreeManager,
    isolation_mgr: IsolationManager,
    start_time: Instant,
    reset_count: u32,
}

impl ForgeSentinelPair {
    pub fn new(mut config: PairConfig, ticket: Ticket) -> Result<Self> {
        config.repo_path = normalize_repo_path(&config.repo_path)?;

        let worktree_mgr = WorktreeManager::new(config.repo_path.clone(), config.pair_id.clone());

        let isolation_mgr = IsolationManager::new(&config.redis_url, config.pair_id.clone())?;

        Ok(Self {
            config,
            ticket,
            worktree_mgr,
            isolation_mgr,
            start_time: Instant::now(),
            reset_count: 0,
        })
    }

    /// Runs the complete pair lifecycle
    pub async fn run(mut self) -> Result<PairOutcome> {
        info!(
            pair_id = %self.config.pair_id,
            ticket_id = %self.ticket.id,
            "Starting FORGE-SENTINEL pair"
        );

        // 1. Provision worktree
        let worktree_path = self.worktree_mgr.create_worktree(&self.ticket.id).await?;
        info!(
            pair_id = %self.config.pair_id,
            worktree = %worktree_path.display(),
            "Worktree provisioned"
        );

        // 2. Setup shared directory
        let shared_path = self.shared_path();
        tokio::fs::create_dir_all(&shared_path)
            .await
            .context("Failed to create shared directory")?;

        // 3. Write ticket artifacts
        self.write_ticket_artifacts(&shared_path).await?;

        // 4. Setup MCP configuration
        self.setup_mcp(&worktree_path, &shared_path).await?;

        // 5. Acquire initial file locks
        self.acquire_initial_locks().await?;

        // 6. Spawn FORGE process
        let mut forge = self.spawn_forge(&worktree_path, &shared_path).await?;

        // 7. Start file watcher
        let (watcher, mut event_rx) =
            PairWatcher::new(shared_path.clone(), self.config.pair_id.clone());

        // Spawn watcher in background
        tokio::spawn(async move {
            if let Err(e) = watcher.watch().await {
                error!(error = %e, "Watcher failed");
            }
        });

        // 8. Main event loop
        let outcome = self
            .event_loop(&mut forge, &mut event_rx, &worktree_path, &shared_path)
            .await?;

        // 9. Cleanup
        self.cleanup(&worktree_path, &mut forge).await?;

        let elapsed_ms = self.start_time.elapsed().as_millis() as u64;
        info!(
            pair_id = %self.config.pair_id,
            ticket_id = %self.ticket.id,
            elapsed_ms = elapsed_ms,
            outcome = ?outcome,
            "Pair execution completed"
        );

        Ok(outcome)
    }

    /// Main event loop - handles file events and spawns SENTINEL
    // Validation: C2-04 Crash Recovery - monitors FORGE exit
    async fn event_loop(
        &mut self,
        forge: &mut AgentProcess,
        event_rx: &mut mpsc::UnboundedReceiver<WatchEvent>,
        worktree_path: &Path,
        shared_path: &Path,
    ) -> Result<PairOutcome> {
        loop {
            tokio::select! {
                // Watch for FORGE process exit
                exit_result = forge.child.wait() => {
                    match exit_result {
                        Ok(status) => {
                            warn!(
                                pair_id = %self.config.pair_id,
                                exit_code = status.code(),
                                "FORGE exited unexpectedly"
                            );

                            // Check if clean artifacts exist
                            let status_exists = shared_path.join("STATUS.json").exists();
                            let handoff_exists = shared_path.join("HANDOFF.md").exists();

                            if status_exists {
                                // Clean exit with STATUS.json - check final status
                                info!("STATUS.json found - checking terminal status");
                                return self.check_terminal_status(shared_path).await?
                                    .ok_or_else(|| anyhow::anyhow!("Expected terminal status after FORGE exit"));
                            } else if handoff_exists {
                                // Clean handoff - handle context reset
                                info!("HANDOFF.md found - performing context reset");
                                if let Some(outcome) = self.handle_context_reset(forge, worktree_path, shared_path).await? {
                                    return Ok(outcome);
                                }
                                // Context reset succeeded, respawn FORGE and continue
                                *forge = self.spawn_forge(worktree_path, shared_path).await?;
                            } else {
                                // Crash recovery: synthesize HANDOFF from WORKLOG
                                warn!("No clean exit artifacts - synthesizing HANDOFF from WORKLOG");
                                self.synthesize_handoff_from_worklog(shared_path).await?;

                                // Validation: C2-04 - Autonomous crash recovery
                                self.reset_count += 1;

                                if self.reset_count >= self.config.max_resets {
                                    return Ok(PairOutcome::FuelExhausted {
                                        resets: self.reset_count,
                                        elapsed_ms: self.start_time.elapsed().as_millis() as u64,
                                    });
                                }

                                info!(
                                    pair_id = %self.config.pair_id,
                                    reset_count = self.reset_count,
                                    "Respawning FORGE after crash"
                                );

                                // Respawn FORGE and continue the loop
                                *forge = self.spawn_forge(worktree_path, shared_path).await?;
                            }
                        }
                        Err(e) => {
                            return Err(anyhow::anyhow!("Failed to wait on FORGE process: {}", e));
                        }
                    }
                }

                // Watch for file events
                Some(event) = event_rx.recv() => {
                    match event {
                        WatchEvent::WorklogModified => {
                            info!(
                                pair_id = %self.config.pair_id,
                                "WORKLOG.md modified - spawning SENTINEL"
                            );
                            self.spawn_and_wait_sentinel(worktree_path, shared_path).await?;
                        }
                        WatchEvent::StatusChanged => {
                            // Check for terminal status
                            if let Some(outcome) = self.check_terminal_status(shared_path).await? {
                                return Ok(outcome);
                            }
                        }
                        WatchEvent::HandoffCreated => {
                            info!(
                                pair_id = %self.config.pair_id,
                                "HANDOFF.md detected - performing context reset"
                            );
                            if let Some(outcome) = self.handle_context_reset(forge, worktree_path, shared_path).await? {
                                return Ok(outcome);
                            }
                        }
                        WatchEvent::PlanModified => {
                            info!(
                                pair_id = %self.config.pair_id,
                                "PLAN.md modified - spawning SENTINEL for review"
                            );
                            self.spawn_and_wait_sentinel(worktree_path, shared_path).await?;
                        }
                    }
                }
            }
        }
    }

    /// Spawns SENTINEL and waits for it to complete
    ///
    /// # Validation: C2-01 SENTINEL is Ephemeral
    async fn spawn_and_wait_sentinel(
        &self,
        worktree_path: &Path,
        shared_path: &Path,
    ) -> Result<()> {
        let process_mgr = self.create_process_manager(worktree_path, shared_path);

        // Validation: C2-01 - SENTINEL spawned on-demand
        let sentinel = process_mgr.spawn_sentinel().await?;

        // Validation: C2-01 - Wait for SENTINEL to exit immediately after eval
        process_mgr.wait_sentinel_completion(sentinel).await?;

        Ok(())
    }

    /// Checks for terminal status (STATUS.json)
    async fn check_terminal_status(&self, shared_path: &Path) -> Result<Option<PairOutcome>> {
        let status_path = shared_path.join("STATUS.json");

        if !status_path.exists() {
            return Ok(None);
        }

        let status = StatusJson::read_from_file(&status_path).await?;
        status.validate()?;

        let elapsed_ms = self.start_time.elapsed().as_millis() as u64;
        let outcome = PairOutcome::from_status(status, elapsed_ms);

        Ok(Some(outcome))
    }

    /// Handles context reset (HANDOFF.md created)
    async fn handle_context_reset(
        &mut self,
        forge: &mut AgentProcess,
        worktree_path: &Path,
        shared_path: &Path,
    ) -> Result<Option<PairOutcome>> {
        self.reset_count += 1;

        info!(
            pair_id = %self.config.pair_id,
            reset_count = self.reset_count,
            max_resets = self.config.max_resets,
            "Context reset in progress"
        );

        if self.reset_count >= self.config.max_resets {
            warn!(
                pair_id = %self.config.pair_id,
                "Maximum context resets exceeded"
            );
            return Ok(Some(PairOutcome::FuelExhausted {
                resets: self.reset_count,
                elapsed_ms: self.start_time.elapsed().as_millis() as u64,
            }));
        }

        // Terminate old FORGE
        let process_mgr = self.create_process_manager(worktree_path, shared_path);

        // Try to create dummy process for swap during termination
        match tokio::process::Command::new("true").spawn() {
            Ok(dummy_child) => {
                process_mgr
                    .terminate_process(std::mem::replace(
                        forge,
                        AgentProcess {
                            child: dummy_child,
                            agent_type: AgentType::Forge,
                            start_time: Instant::now(),
                        },
                    ))
                    .await?;
            }
            Err(e) => {
                warn!(error = %e, "Could not spawn dummy process for cleanup, killing FORGE directly");
                if let Err(kill_err) = forge.child.kill().await {
                    warn!(error = %kill_err, "Failed to kill FORGE process");
                }
            }
        }

        // Spawn fresh FORGE
        *forge = self.spawn_forge(worktree_path, shared_path).await?;

        info!(
            pair_id = %self.config.pair_id,
            "Fresh FORGE spawned after context reset"
        );

        Ok(None)
    }

    /// Spawns FORGE process
    async fn spawn_forge(&self, worktree_path: &Path, shared_path: &Path) -> Result<AgentProcess> {
        let process_mgr = self.create_process_manager(worktree_path, shared_path);
        process_mgr.spawn_forge().await
    }

    /// Checks if FORGE is still alive
    async fn check_forge_alive(&self, forge: &mut AgentProcess) -> Result<bool> {
        let process_mgr =
            self.create_process_manager(&self.worktree_mgr.worktree_path(), &self.shared_path());
        process_mgr.check_forge_alive(forge).await
    }

    /// Creates a ProcessManager instance
    fn create_process_manager(&self, worktree_path: &Path, shared_path: &Path) -> ProcessManager {
        ProcessManager::new(
            self.config.pair_id.clone(),
            self.ticket.id.clone(),
            worktree_path.to_path_buf(),
            shared_path.to_path_buf(),
            self.config.redis_url.clone(),
            self.config.github_token.clone(),
        )
    }

    /// Returns the shared directory path
    fn shared_path(&self) -> PathBuf {
        self.config
            .repo_path
            .join(".sprintless")
            .join("pairs")
            .join(&self.config.pair_id)
            .join("shared")
    }

    /// Writes ticket artifacts to shared directory
    async fn write_ticket_artifacts(&self, shared_path: &Path) -> Result<()> {
        let ticket_path = shared_path.join("TICKET.md");
        self.ticket.write_ticket_md(&ticket_path).await?;

        info!(
            pair_id = %self.config.pair_id,
            "Ticket artifacts written"
        );

        Ok(())
    }

    /// Sets up MCP configuration and installs hooks
    ///
    /// # Validation: C4-02 Hook Enforcement
    async fn setup_mcp(&self, worktree_path: &Path, shared_path: &Path) -> Result<()> {
        // Create MCP directories
        setup_mcp_directories(worktree_path, shared_path).await?;

        // Validation: C4-02 - Install plugin hooks into runtime
        crate::mcp_config::install_plugin_hooks(worktree_path, shared_path, &self.config.repo_path)
            .await?;

        // Generate MCP config
        let mcp_gen = McpConfigGenerator::new(
            self.config.pair_id.clone(),
            self.ticket.id.clone(),
            worktree_path.to_path_buf(),
            shared_path.to_path_buf(),
            self.config.redis_url.clone(),
            self.config.github_token.clone(),
        );

        // Write config for FORGE
        let forge_config_path = worktree_path.join(".claude").join("mcp.json");
        if let Some(parent) = forge_config_path.parent() {
            tokio::fs::create_dir_all(parent).await?;
        }
        mcp_gen.write_config(&forge_config_path).await?;

        // Write config for SENTINEL
        let sentinel_config_path = shared_path.join(".claude").join("mcp.json");
        if let Some(parent) = sentinel_config_path.parent() {
            tokio::fs::create_dir_all(parent).await?;
        }
        mcp_gen.write_config(&sentinel_config_path).await?;

        info!(
            pair_id = %self.config.pair_id,
            "MCP configuration and hooks installed"
        );

        Ok(())
    }

    /// Acquires initial file locks
    async fn acquire_initial_locks(&self) -> Result<()> {
        let failed = self
            .isolation_mgr
            .acquire_locks_batch(&self.ticket.touched_files)
            .await?;

        if !failed.is_empty() {
            warn!(
                pair_id = %self.config.pair_id,
                failed_locks = ?failed,
                "Some files are already locked"
            );
            anyhow::bail!("Cannot acquire locks for initial files");
        }

        info!(
            pair_id = %self.config.pair_id,
            locked_files = self.ticket.touched_files.len(),
            "Initial file locks acquired"
        );

        Ok(())
    }

    /// Synthesize HANDOFF.md from WORKLOG.md after crash
    /// Validation: C2-04 Crash Recovery
    async fn synthesize_handoff_from_worklog(&self, shared_path: &Path) -> Result<()> {
        let worklog_path = shared_path.join("WORKLOG.md");
        let handoff_path = shared_path.join("HANDOFF.md");

        let worklog_content = tokio::fs::read_to_string(&worklog_path)
            .await
            .unwrap_or_else(|_| "# Worklog\n\n(No worklog found)".to_string());

        let handoff_content = format!(
            r#"# Handoff Document (Auto-generated from Crash)

## Context
FORGE exited unexpectedly without producing STATUS.json or HANDOFF.md.
This document was synthesized from the WORKLOG to preserve progress.

## Work Completed So Far
{}

## Exact next step
Review the WORKLOG.md above and continue from where the previous session stopped.
Check for any in-progress work and complete the current segment before moving forward.

## Artifacts
See WORKLOG.md for detailed activity log.
Ticket: {}
"#,
            worklog_content, self.ticket.id
        );

        tokio::fs::write(&handoff_path, handoff_content).await?;

        warn!(
            pair_id = %self.config.pair_id,
            "Synthesized HANDOFF.md from WORKLOG.md due to crash"
        );

        Ok(())
    }

    /// Cleanup resources
    async fn cleanup(&self, worktree_path: &Path, forge: &mut AgentProcess) -> Result<()> {
        info!(
            pair_id = %self.config.pair_id,
            "Starting cleanup"
        );

        // Terminate FORGE if still running
        if self.check_forge_alive(forge).await.unwrap_or(false) {
            let process_mgr = self.create_process_manager(worktree_path, &self.shared_path());

            // Try to create dummy process for swap during termination
            match tokio::process::Command::new("true").spawn() {
                Ok(dummy_child) => {
                    process_mgr
                        .terminate_process(std::mem::replace(
                            forge,
                            AgentProcess {
                                child: dummy_child,
                                agent_type: AgentType::Forge,
                                start_time: Instant::now(),
                            },
                        ))
                        .await?;
                }
                Err(e) => {
                    warn!(error = %e, "Could not spawn dummy process for cleanup, killing FORGE directly");
                    if let Err(kill_err) = forge.child.kill().await {
                        warn!(error = %kill_err, "Failed to kill FORGE process during cleanup");
                    }
                }
            }
        }

        // Release all file locks
        self.isolation_mgr.release_all_locks().await?;

        info!(
            pair_id = %self.config.pair_id,
            "Cleanup completed"
        );

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalizes_repo_root_from_nested_path() {
        let temp = tempfile::tempdir().unwrap();
        Repository::init(temp.path()).unwrap();

        let nested = temp.path().join("crates").join("pair-harness");
        std::fs::create_dir_all(&nested).unwrap();

        let resolved = normalize_repo_path(&nested).unwrap();

        assert_eq!(
            resolved.canonicalize().unwrap(),
            temp.path().canonicalize().unwrap(),
        );
    }
}
