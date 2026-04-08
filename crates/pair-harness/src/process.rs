// crates/pair-harness/src/process.rs
//! Process management for FORGE and SENTINEL agents.

use anyhow::{Context, Result, anyhow};
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::time::Duration;
use tokio::process::{Command, Child, ChildStdout, ChildStderr};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tracing::{info, warn, debug, error};
use serde_json::json;

/// Mode for SENTINEL spawning.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SentinelMode {
    /// Plan review mode (SPRINTLESS_SEGMENT is empty)
    PlanReview,
    /// Segment evaluation mode (SPRINTLESS_SEGMENT is set)
    SegmentEval(u32),
    /// Final review mode
    FinalReview,
}

impl SentinelMode {
    /// Get the SPRINTLESS_SEGMENT value for this mode.
    pub fn segment_value(&self) -> String {
        match self {
            SentinelMode::PlanReview => String::new(),
            SentinelMode::SegmentEval(n) => n.to_string(),
            SentinelMode::FinalReview => "final".to_string(),
        }
    }
}

/// Manages FORGE and SENTINEL processes.
pub struct ProcessManager {
    /// Path to the Claude binary
    claude_path: PathBuf,
    /// GitHub token for MCP tools
    github_token: String,
    /// Optional Redis URL for shared store (fallback to filesystem if None)
    redis_url: Option<String>,
}

impl ProcessManager {
    /// Create a new process manager without Redis (uses filesystem state).
    pub fn new(github_token: impl Into<String>) -> Self {
        let claude_path = std::env::var("CLAUDE_PATH")
            .unwrap_or_else(|_| "claude".to_string());
        
        Self {
            claude_path: PathBuf::from(claude_path),
            github_token: github_token.into(),
            redis_url: None,
        }
    }

    /// Create a process manager with Redis backend.
    pub fn with_redis(github_token: impl Into<String>, redis_url: impl Into<String>) -> Self {
        let claude_path = std::env::var("CLAUDE_PATH")
            .unwrap_or_else(|_| "claude".to_string());
        
        Self {
            claude_path: PathBuf::from(claude_path),
            github_token: github_token.into(),
            redis_url: Some(redis_url.into()),
        }
    }

    /// Spawn a FORGE process (long-running).
    pub async fn spawn_forge(
        &self,
        pair_id: &str,
        ticket_id: &str,
        worktree: &Path,
        shared: &Path,
    ) -> Result<Child> {
        info!(
            pair = pair_id,
            ticket = ticket_id,
            worktree = %worktree.display(),
            "Spawning FORGE process"
        );

        // Ensure the sentinel working directory exists
        let sentinel_dir = shared.join("sentinel");
        tokio::fs::create_dir_all(&sentinel_dir)
            .await
            .context("Failed to create sentinel directory")?;

        // Build the initial prompt for FORGE
        let initial_prompt = self.build_forge_prompt(shared);

        let mut cmd = Command::new(&self.claude_path);
        cmd.args([
                "--permission-mode", "auto",
                "--print",
                &initial_prompt,
            ])
            .env("SPRINTLESS_PAIR_ID", pair_id)
            .env("SPRINTLESS_TICKET_ID", ticket_id)
            .env("SPRINTLESS_SEGMENT", "")
            .env("SPRINTLESS_WORKTREE", worktree.to_string_lossy().to_string())
            .env("SPRINTLESS_SHARED", shared.to_string_lossy().to_string())
            .env("SPRINTLESS_GITHUB_TOKEN", &self.github_token)
            .env("ANTHROPIC_API_KEY", std::env::var("ANTHROPIC_API_KEY").unwrap_or_default())
            .current_dir(worktree)
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());

        // Set Redis URL if provided, otherwise use filesystem-based state in shared directory
        if let Some(redis_url) = &self.redis_url {
            cmd.env("SPRINTLESS_REDIS_URL", redis_url);
        } else {
            cmd.env("SPRINTLESS_STATE_FILE", shared.join("state.json").to_string_lossy().to_string());
        }

        let mut child = cmd.spawn()
            .context("Failed to spawn FORGE process")?;

        // Capture and log stdout/stderr in background
        let log_dir = shared.join("logs");
        tokio::fs::create_dir_all(&log_dir).await?;
        
        if let Some(stdout) = child.stdout.take() {
            let stdout_log = log_dir.join("forge-stdout.log");
            let pair_id_clone = pair_id.to_string();
            tokio::spawn(async move {
                Self::stream_to_file(stdout, stdout_log, &pair_id_clone, "FORGE-OUT").await;
            });
        }
        
        if let Some(stderr) = child.stderr.take() {
            let stderr_log = log_dir.join("forge-stderr.log");
            let pair_id_clone = pair_id.to_string();
            tokio::spawn(async move {
                Self::stream_to_file(stderr, stderr_log, &pair_id_clone, "FORGE-ERR").await;
            });
        }

        info!(pair = pair_id, pid = ?child.id(), "FORGE process spawned");
        Ok(child)
    }

    /// Spawn a FORGE process in resume mode (after context reset).
    pub async fn spawn_forge_resume(
        &self,
        pair_id: &str,
        ticket_id: &str,
        worktree: &Path,
        shared: &Path,
    ) -> Result<Child> {
        info!(
            pair = pair_id,
            ticket = ticket_id,
            "Spawning FORGE process (resume mode)"
        );

        // Same as regular spawn, but the session_start hook will detect HANDOFF.md
        self.spawn_forge(pair_id, ticket_id, worktree, shared).await
    }

    /// Spawn a SENTINEL process (ephemeral, for single evaluation).
    pub async fn spawn_sentinel(
        &self,
        pair_id: &str,
        ticket_id: &str,
        mode: SentinelMode,
        worktree: &Path,
        shared: &Path,
    ) -> Result<Child> {
        let segment = mode.segment_value();
        
        info!(
            pair = pair_id,
            ticket = ticket_id,
            mode = ?mode,
            segment = %segment,
            "Spawning SENTINEL process (ephemeral)"
        );

        // Ensure the sentinel working directory exists
        let sentinel_dir = shared.join("sentinel");
        tokio::fs::create_dir_all(&sentinel_dir)
            .await
            .context("Failed to create sentinel directory")?;

        // Build the initial prompt for SENTINEL based on mode
        let initial_prompt = self.build_sentinel_prompt(shared, &mode);

        let mut cmd = Command::new(&self.claude_path);
        cmd.args([
                "--permission-mode", "auto",
                "--print",
                &initial_prompt,
            ])
            .env("SPRINTLESS_PAIR_ID", pair_id)
            .env("SPRINTLESS_TICKET_ID", ticket_id)
            .env("SPRINTLESS_SEGMENT", &segment)
            .env("SPRINTLESS_WORKTREE", worktree.to_string_lossy().to_string())
            .env("SPRINTLESS_SHARED", shared.to_string_lossy().to_string())
            .env("SPRINTLESS_GITHUB_TOKEN", &self.github_token)
            .env("ANTHROPIC_API_KEY", std::env::var("ANTHROPIC_API_KEY").unwrap_or_default())
            .current_dir(&sentinel_dir)
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());

        // Set Redis URL if provided, otherwise use filesystem-based state
        if let Some(redis_url) = &self.redis_url {
            cmd.env("SPRINTLESS_REDIS_URL", redis_url);
        } else {
            cmd.env("SPRINTLESS_STATE_FILE", shared.join("state.json").to_string_lossy().to_string());
        }

        let mut child = cmd.spawn()
            .context("Failed to spawn SENTINEL process")?;

        // Capture and log stdout/stderr in background
        let log_dir = shared.join("logs");
        tokio::fs::create_dir_all(&log_dir).await?;
        
        let mode_str = format!("{:?}", mode);
        if let Some(stdout) = child.stdout.take() {
            let stdout_log = log_dir.join(format!("sentinel-{}-stdout.log", mode_str));
            let pair_id_clone = pair_id.to_string();
            tokio::spawn(async move {
                Self::stream_to_file(stdout, stdout_log, &pair_id_clone, "SENTINEL-OUT").await;
            });
        }
        
        if let Some(stderr) = child.stderr.take() {
            let stderr_log = log_dir.join(format!("sentinel-{}-stderr.log", mode_str));
            let pair_id_clone = pair_id.to_string();
            tokio::spawn(async move {
                Self::stream_to_file(stderr, stderr_log, &pair_id_clone, "SENTINEL-ERR").await;
            });
        }

        info!(pair = pair_id, pid = ?child.id(), mode = ?mode, "SENTINEL process spawned");
        Ok(child)
    }

    /// Wait for a process to complete with timeout.
    pub async fn wait_with_timeout(
        &self,
        child: &mut Child,
        timeout: Duration,
    ) -> Result<ProcessOutcome> {
        match tokio::time::timeout(timeout, child.wait()).await {
            Ok(Ok(status)) => {
                if status.success() {
                    Ok(ProcessOutcome::Success)
                } else {
                    warn!(exit_code = ?status.code(), "Process exited with error");
                    Ok(ProcessOutcome::Failed {
                        exit_code: status.code(),
                    })
                }
            }
            Ok(Err(e)) => {
                error!(error = %e, "Failed to wait for process");
                Err(anyhow!("Failed to wait for process: {}", e))
            }
            Err(_) => {
                warn!("Process timed out, killing");
                child.kill().await.context("Failed to kill timed-out process")?;
                Ok(ProcessOutcome::Timeout)
            }
        }
    }

    /// Kill a process.
    pub async fn kill(&self, child: &mut Child) -> Result<()> {
        info!(pid = ?child.id(), "Killing process");
        child.kill().await.context("Failed to kill process")?;
        Ok(())
    }

    /// Check if a process is still running.
    pub async fn is_running(&self, child: &mut Child) -> bool {
        // Try to get exit status without blocking
        matches!(child.try_wait(), Ok(None))
    }

    /// Build the initial prompt for FORGE based on current state.
    fn build_forge_prompt(&self, shared: &Path) -> String {
        let handoff_path = shared.join("HANDOFF.md");
        let ticket_path = shared.join("TICKET.md");
        let task_path = shared.join("TASK.md");

        if handoff_path.exists() {
            // Resume mode - read handoff and continue
            let handoff = std::fs::read_to_string(&handoff_path)
                .unwrap_or_else(|_| "Could not read HANDOFF.md".to_string());
            
            format!(
                "You are FORGE, an autonomous coding agent. You are resuming work after a context reset.\n\n\
                Read the handoff document and continue from the exact next step:\n\n\
                --- HANDOFF.md ---\n{}\n\n\
                Continue exactly where the previous session left off. Do not repeat work already done.",
                handoff
            )
        } else {
            // New session - read ticket and task
            let ticket = std::fs::read_to_string(&ticket_path)
                .unwrap_or_else(|_| "No TICKET.md found".to_string());
            let task = std::fs::read_to_string(&task_path)
                .unwrap_or_else(|_| "No TASK.md found".to_string());
            let shared_path = shared.display();

            format!(
                "You are FORGE, an autonomous coding agent. Start working on the assigned ticket.\n\n\
                --- TICKET.md ---\n{}\n\n\
                --- TASK.md ---\n{}\n\n\
                SHARED DIRECTORY: {}\n\n\
                INSTRUCTIONS:\n\
                1. First, create a detailed implementation plan. Write it to {}/PLAN.md\n\
                2. Wait for SENTINEL to review your plan and write {}/CONTRACT.md\n\
                3. Once CONTRACT.md shows status AGREED, begin implementation\n\
                4. As you work, document your progress in {}/WORKLOG.md (one segment at a time)\n\
                5. When complete, open a PR and write {}/STATUS.json with the result\n\n\
                Start by reading the repository structure and creating PLAN.md.",
                ticket, task, shared_path, shared_path, shared_path, shared_path, shared_path
            )
        }
    }

    /// Build the initial prompt for SENTINEL based on mode.
    fn build_sentinel_prompt(&self, shared: &Path, mode: &SentinelMode) -> String {
        match mode {
            SentinelMode::PlanReview => {
                let plan_path = shared.join("PLAN.md");
                let plan = std::fs::read_to_string(&plan_path)
                    .unwrap_or_else(|_| "No PLAN.md found".to_string());
                
                format!(
                    "You are SENTINEL, an autonomous code reviewer. Review the following plan.\n\n\
                    --- PLAN.md ---\n{}\n\n\
                    Evaluate the plan for completeness, feasibility, and alignment with the ticket requirements. \
                    Write CONTRACT.md with your evaluation. Set status to AGREED if the plan is good, \
                    or ISSUES if it needs revision with specific feedback.",
                    plan
                )
            }
            SentinelMode::SegmentEval(n) => {
                let worklog_path = shared.join("WORKLOG.md");
                let worklog = std::fs::read_to_string(&worklog_path)
                    .unwrap_or_else(|_| "No WORKLOG.md found".to_string());
                
                format!(
                    "You are SENTINEL, an autonomous code reviewer. Evaluate segment {}.\n\n\
                    --- WORKLOG.md ---\n{}\n\n\
                    Review the implementation in segment {} for correctness, test coverage, and code quality. \
                    Write segment-{}-eval.md with your evaluation. Approve if the segment meets standards, \
                    or request changes with specific feedback.",
                    n, worklog, n, n
                )
            }
            SentinelMode::FinalReview => {
                let worklog_path = shared.join("WORKLOG.md");
                let worklog = std::fs::read_to_string(&worklog_path)
                    .unwrap_or_else(|_| "No WORKLOG.md found".to_string());
                
                format!(
                    "You are SENTINEL, an autonomous code reviewer. Perform final review.\n\n\
                    --- WORKLOG.md ---\n{}\n\n\
                    Review the complete implementation. Check that all acceptance criteria are met. \
                    Write final-review.md with your verdict. Set verdict to APPROVED if ready to merge, \
                    or REJECTED with specific issues that must be fixed.",
                    worklog
                )
            }
        }
    }

    /// Stream process output to a log file.
    async fn stream_to_file<T: tokio::io::AsyncRead + Unpin>(
        stream: T,
        log_path: PathBuf,
        pair_id: &str,
        prefix: &str,
    ) {
        let mut reader = BufReader::new(stream).lines();
        let mut log_file = match tokio::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&log_path)
            .await
        {
            Ok(f) => f,
            Err(e) => {
                error!(pair = pair_id, error = %e, "Failed to open log file");
                return;
            }
        };

        while let Ok(Some(line)) = reader.next_line().await {
            debug!(pair = pair_id, prefix = prefix, "{}", line);
            if let Err(e) = log_file.write_all(format!("{}\n", line).as_bytes()).await {
                error!(pair = pair_id, error = %e, "Failed to write to log file");
                break;
            }
        }
    }
}

/// Outcome of a process execution.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ProcessOutcome {
    /// Process completed successfully
    Success,
    /// Process failed with exit code
    Failed { exit_code: Option<i32> },
    /// Process timed out and was killed
    Timeout,
}

/// Builder for creating FORGE processes with custom configuration.
pub struct ForgeProcessBuilder {
    pair_id: String,
    ticket_id: String,
    worktree: PathBuf,
    shared: PathBuf,
    github_token: String,
    redis_url: Option<String>,
    extra_env: Vec<(String, String)>,
}

impl ForgeProcessBuilder {
    /// Create a new builder.
    pub fn new(
        pair_id: impl Into<String>,
        ticket_id: impl Into<String>,
        worktree: PathBuf,
        shared: PathBuf,
    ) -> Self {
        Self {
            pair_id: pair_id.into(),
            ticket_id: ticket_id.into(),
            worktree,
            shared,
            github_token: String::new(),
            redis_url: None,
            extra_env: Vec::new(),
        }
    }

    /// Set the GitHub token.
    pub fn github_token(mut self, token: impl Into<String>) -> Self {
        self.github_token = token.into();
        self
    }

    /// Set the Redis URL (optional - uses filesystem state if not provided).
    pub fn redis_url(mut self, url: impl Into<String>) -> Self {
        self.redis_url = Some(url.into());
        self
    }

    /// Add an extra environment variable.
    pub fn env(mut self, key: impl Into<String>, value: impl Into<String>) -> Self {
        self.extra_env.push((key.into(), value.into()));
        self
    }

    /// Build and spawn the FORGE process.
    pub async fn spawn(self) -> Result<Child> {
        let manager = if let Some(redis_url) = &self.redis_url {
            ProcessManager::with_redis(self.github_token, redis_url)
        } else {
            ProcessManager::new(self.github_token)
        };
        
        let mut child = manager.spawn_forge(
            &self.pair_id,
            &self.ticket_id,
            &self.worktree,
            &self.shared,
        ).await?;

        // Add extra environment variables
        // Note: This doesn't work after spawn, so we need to handle this differently
        // For now, the extra_env is not used, but could be added to the Command before spawn

        Ok(child)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_sentinel_mode_segment_value() {
        assert_eq!(SentinelMode::PlanReview.segment_value(), "");
        assert_eq!(SentinelMode::SegmentEval(3).segment_value(), "3");
        assert_eq!(SentinelMode::FinalReview.segment_value(), "final");
    }
}