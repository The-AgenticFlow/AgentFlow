// crates/pair-harness/src/process.rs
//! Process management for FORGE and SENTINEL agents.

use anyhow::{anyhow, Context, Result};
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::time::Duration;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::process::{Child, Command};
use tracing::{debug, error, info, warn};

#[cfg(unix)]
fn is_executable(path: &Path) -> bool {
    use std::os::unix::fs::MetadataExt;
    path.metadata()
        .map(|m| m.mode() & 0o111 != 0)
        .unwrap_or(false)
}

#[cfg(not(unix))]
fn is_executable(_path: &Path) -> bool {
    true
}

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
        let claude_path = std::env::var("CLAUDE_PATH").unwrap_or_else(|_| "claude".to_string());
        let claude_path = PathBuf::from(claude_path);

        Self::validate_claude_binary(&claude_path);

        Self {
            claude_path,
            github_token: github_token.into(),
            redis_url: None,
        }
    }

    /// Create a process manager with Redis backend.
    pub fn with_redis(github_token: impl Into<String>, redis_url: impl Into<String>) -> Self {
        let claude_path = std::env::var("CLAUDE_PATH").unwrap_or_else(|_| "claude".to_string());
        let claude_path = PathBuf::from(claude_path);

        Self::validate_claude_binary(&claude_path);

        Self {
            claude_path,
            github_token: github_token.into(),
            redis_url: Some(redis_url.into()),
        }
    }

    fn validate_claude_binary(claude_path: &Path) {
        if claude_path.is_absolute() {
            if !claude_path.exists() {
                error!(
                    path = %claude_path.display(),
                    "CLAUDE_PATH binary not found. Install Claude CLI or set CLAUDE_PATH in .env"
                );
            } else if !is_executable(claude_path) {
                error!(
                    path = %claude_path.display(),
                    "CLAUDE_PATH binary exists but is not executable. Run: chmod +x {}",
                    claude_path.display()
                );
            }
        } else {
            match which::which(claude_path) {
                Ok(found) => {
                    debug!(path = %found.display(), "Claude CLI binary found");
                }
                Err(_) => {
                    error!(
                        binary = %claude_path.display(),
                        "Claude CLI binary not found on PATH. Install it from https://claude.ai/download or set CLAUDE_PATH in .env to an absolute path"
                    );
                }
            }
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

        // Build the initial prompt for FORGE
        let initial_prompt = self.build_forge_prompt(shared);
        let settings_path = worktree.join(".claude").join("settings.json");

        let mut cmd = Command::new(&self.claude_path);
        cmd.arg("--bare")
            .arg("--print")
            .arg("--dangerously-skip-permissions")
            .arg("--settings")
            .arg(&settings_path)
            .arg("--add-dir")
            .arg(shared)
            .env("SPRINTLESS_PAIR_ID", pair_id)
            .env("SPRINTLESS_TICKET_ID", ticket_id)
            .env("SPRINTLESS_SEGMENT", "")
            .env(
                "SPRINTLESS_WORKTREE",
                worktree.to_string_lossy().to_string(),
            )
            .env("SPRINTLESS_SHARED", shared.to_string_lossy().to_string())
            .env("SPRINTLESS_GITHUB_TOKEN", &self.github_token)
            // Pass all LLM provider environment variables for fallback support
            .env(
                "LLM_PROVIDER",
                std::env::var("LLM_PROVIDER").unwrap_or_else(|_| "fallback".to_string()),
            )
            .env(
                "LLM_FALLBACK",
                std::env::var("LLM_FALLBACK").unwrap_or_default(),
            )
            .env(
                "ANTHROPIC_API_KEY",
                std::env::var("ANTHROPIC_API_KEY").unwrap_or_default(),
            )
            .env(
                "ANTHROPIC_MODEL",
                std::env::var("ANTHROPIC_MODEL").unwrap_or_default(),
            )
            .env(
                "OPENAI_API_KEY",
                std::env::var("OPENAI_API_KEY").unwrap_or_default(),
            )
            .env(
                "OPENAI_MODEL",
                std::env::var("OPENAI_MODEL").unwrap_or_default(),
            )
            .env(
                "GEMINI_API_KEY",
                std::env::var("GEMINI_API_KEY").unwrap_or_default(),
            )
            .env(
                "GEMINI_MODEL",
                std::env::var("GEMINI_MODEL").unwrap_or_default(),
            )
            .current_dir(worktree)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());

        // Set Redis URL if provided, otherwise use filesystem-based state in shared directory
        if let Some(redis_url) = &self.redis_url {
            cmd.env("SPRINTLESS_REDIS_URL", redis_url);
        } else {
            cmd.env(
                "SPRINTLESS_STATE_FILE",
                shared.join("state.json").to_string_lossy().to_string(),
            );
        }

        let mut child = cmd.spawn().context("Failed to spawn FORGE process")?;

        if let Some(mut stdin) = child.stdin.take() {
            stdin
                .write_all(initial_prompt.as_bytes())
                .await
                .context("Failed to write FORGE prompt to stdin")?;
            stdin
                .shutdown()
                .await
                .context("Failed to close FORGE stdin")?;
        }

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

    /// Spawn a FORGE process for PR creation after final SENTINEL approval.
    pub async fn spawn_forge_for_pr(
        &self,
        pair_id: &str,
        ticket_id: &str,
        worktree: &Path,
        shared: &Path,
    ) -> Result<Child> {
        info!(
            pair = pair_id,
            ticket = ticket_id,
            "Spawning FORGE process (PR creation mode)"
        );

        // Build prompt for PR creation
        let initial_prompt = self.build_forge_pr_prompt(shared);
        let settings_path = worktree.join(".claude").join("settings.json");

        let mut cmd = Command::new(&self.claude_path);
        cmd.arg("--bare")
            .arg("--print")
            .arg("--dangerously-skip-permissions")
            .arg("--settings")
            .arg(&settings_path)
            .arg("--add-dir")
            .arg(shared)
            .env("SPRINTLESS_PAIR_ID", pair_id)
            .env("SPRINTLESS_TICKET_ID", ticket_id)
            .env("SPRINTLESS_SEGMENT", "")
            .env(
                "SPRINTLESS_WORKTREE",
                worktree.to_string_lossy().to_string(),
            )
            .env("SPRINTLESS_SHARED", shared.to_string_lossy().to_string())
            .env("SPRINTLESS_GITHUB_TOKEN", &self.github_token)
            // Pass all LLM provider environment variables for fallback support
            .env(
                "LLM_PROVIDER",
                std::env::var("LLM_PROVIDER").unwrap_or_else(|_| "fallback".to_string()),
            )
            .env(
                "LLM_FALLBACK",
                std::env::var("LLM_FALLBACK").unwrap_or_default(),
            )
            .env(
                "ANTHROPIC_API_KEY",
                std::env::var("ANTHROPIC_API_KEY").unwrap_or_default(),
            )
            .env(
                "ANTHROPIC_MODEL",
                std::env::var("ANTHROPIC_MODEL").unwrap_or_default(),
            )
            .env(
                "OPENAI_API_KEY",
                std::env::var("OPENAI_API_KEY").unwrap_or_default(),
            )
            .env(
                "OPENAI_MODEL",
                std::env::var("OPENAI_MODEL").unwrap_or_default(),
            )
            .env(
                "GEMINI_API_KEY",
                std::env::var("GEMINI_API_KEY").unwrap_or_default(),
            )
            .env(
                "GEMINI_MODEL",
                std::env::var("GEMINI_MODEL").unwrap_or_default(),
            )
            .current_dir(worktree)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());

        // Set Redis URL if provided, otherwise use filesystem-based state
        if let Some(redis_url) = &self.redis_url {
            cmd.env("SPRINTLESS_REDIS_URL", redis_url);
        } else {
            cmd.env(
                "SPRINTLESS_STATE_FILE",
                shared.join("state.json").to_string_lossy().to_string(),
            );
        }

        let mut child = cmd
            .spawn()
            .context("Failed to spawn FORGE process (PR mode)")?;

        if let Some(mut stdin) = child.stdin.take() {
            stdin
                .write_all(initial_prompt.as_bytes())
                .await
                .context("Failed to write FORGE PR prompt to stdin")?;
            stdin
                .shutdown()
                .await
                .context("Failed to close FORGE stdin")?;
        }

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

        info!(pair = pair_id, pid = ?child.id(), "FORGE process (PR mode) spawned");
        Ok(child)
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

        // Build the initial prompt for SENTINEL based on mode
        let initial_prompt = self.build_sentinel_prompt(shared, &mode);
        let settings_path = shared.join(".claude").join("settings.json");

        let mut cmd = Command::new(&self.claude_path);
        cmd.arg("--bare")
            .arg("--print")
            .arg("--output-format")
            .arg("json")
            .arg("--dangerously-skip-permissions")
            .arg("--settings")
            .arg(&settings_path)
            .arg("--add-dir")
            .arg(worktree)
            .args(["--no-session-persistence"])
            .env("SPRINTLESS_PAIR_ID", pair_id)
            .env("SPRINTLESS_TICKET_ID", ticket_id)
            .env("SPRINTLESS_SEGMENT", &segment)
            .env(
                "SPRINTLESS_WORKTREE",
                worktree.to_string_lossy().to_string(),
            )
            .env("SPRINTLESS_SHARED", shared.to_string_lossy().to_string())
            .env("SPRINTLESS_GITHUB_TOKEN", &self.github_token)
            // Pass all LLM provider environment variables for fallback support
            .env(
                "LLM_PROVIDER",
                std::env::var("LLM_PROVIDER").unwrap_or_else(|_| "fallback".to_string()),
            )
            .env(
                "LLM_FALLBACK",
                std::env::var("LLM_FALLBACK").unwrap_or_default(),
            )
            .env(
                "ANTHROPIC_API_KEY",
                std::env::var("ANTHROPIC_API_KEY").unwrap_or_default(),
            )
            .env(
                "ANTHROPIC_MODEL",
                std::env::var("ANTHROPIC_MODEL").unwrap_or_default(),
            )
            .env(
                "OPENAI_API_KEY",
                std::env::var("OPENAI_API_KEY").unwrap_or_default(),
            )
            .env(
                "OPENAI_MODEL",
                std::env::var("OPENAI_MODEL").unwrap_or_default(),
            )
            .env(
                "GEMINI_API_KEY",
                std::env::var("GEMINI_API_KEY").unwrap_or_default(),
            )
            .env(
                "GEMINI_MODEL",
                std::env::var("GEMINI_MODEL").unwrap_or_default(),
            )
            .current_dir(shared)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());

        // Set Redis URL if provided, otherwise use filesystem-based state
        if let Some(redis_url) = &self.redis_url {
            cmd.env("SPRINTLESS_REDIS_URL", redis_url);
        } else {
            cmd.env(
                "SPRINTLESS_STATE_FILE",
                shared.join("state.json").to_string_lossy().to_string(),
            );
        }

        let mut child = cmd.spawn().context("Failed to spawn SENTINEL process")?;

        if let Some(mut stdin) = child.stdin.take() {
            stdin
                .write_all(initial_prompt.as_bytes())
                .await
                .context("Failed to write SENTINEL prompt to stdin")?;
            stdin
                .shutdown()
                .await
                .context("Failed to close SENTINEL stdin")?;
        }

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
                child
                    .kill()
                    .await
                    .context("Failed to kill timed-out process")?;
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
        let contract_path = shared.join("CONTRACT.md");
        let plan_path = shared.join("PLAN.md");
        let shared_path = shared.display();

        if handoff_path.exists() {
            // Resume mode - read handoff and continue
            let handoff = std::fs::read_to_string(&handoff_path)
                .unwrap_or_else(|_| "Could not read HANDOFF.md".to_string());

            format!(
                "You are FORGE, an autonomous coding agent. You are resuming work after a context reset.\n\n\
                IMPORTANT - Directory Structure:\n\
                - CURRENT DIRECTORY (worktree): Write ALL source code, tests, package.json here\n\
                - SHARED DIRECTORY ({}): Read/write PLAN.md, WORKLOG.md, STATUS.json here\n\n\
                Read the handoff document and continue from the exact next step:\n\n\
                --- HANDOFF.md ---\n{}\n\n\
                Continue exactly where the previous session left off. Do not repeat work already done.",
                shared_path, handoff
            )
        } else if contract_path.exists() {
            // Check contract status for plan revision
            let contract = std::fs::read_to_string(&contract_path)
                .unwrap_or_else(|_| "Could not read CONTRACT.md".to_string());

            if contract.contains("status: ISSUES") || contract.contains("status: \"ISSUES\"") {
                // Plan was rejected - need to revise
                let plan = std::fs::read_to_string(&plan_path)
                    .unwrap_or_else(|_| "No PLAN.md found".to_string());
                let ticket = std::fs::read_to_string(&ticket_path)
                    .unwrap_or_else(|_| "No TICKET.md found".to_string());

                format!(
                    "You are FORGE. Your plan was REJECTED. Rewrite {}/PLAN.md now.\n\n\
                    --- TICKET.md ---\n{}\n\n\
                    --- Current PLAN.md ---\n{}\n\n\
                    --- REJECTION ---\n{}\n\n\
                    IMPORTANT - Directory Structure:\n\
                    - CURRENT DIRECTORY (worktree): Source code goes here\n\
                    - SHARED DIRECTORY ({}): PLAN.md, WORKLOG.md, STATUS.json go here\n\n\
                    Use GitHub MCP to fetch the issue. Read codebase in current directory. \
                    Write {}/PLAN.md with:\n\
                    - ## Understanding: What we're building\n\
                    - ## Segments: Specific files in CURRENT DIRECTORY like 'src/counter.ts'\n\
                    - ## Files Changed: Every file you'll touch (all in current directory)\n\
                    - ## Risks: What could go wrong",
                    shared_path, ticket, plan, contract, shared_path, shared_path
                )
            } else if contract.contains("status: AGREED") || contract.contains("status: \"AGREED\"")
            {
                // Contract agreed - continue implementation
                let worklog_path = shared.join("WORKLOG.md");
                let worklog = if worklog_path.exists() {
                    std::fs::read_to_string(&worklog_path)
                        .unwrap_or_else(|_| "No WORKLOG.md found".to_string())
                } else {
                    "No WORKLOG.md yet - start implementation".to_string()
                };

                format!(
                    "You are FORGE, an autonomous coding agent. Your plan was approved.\n\n\
                    --- CONTRACT.md ---\n{}\n\n\
                    --- WORKLOG.md ---\n{}\n\n\
                    IMPORTANT - Directory Structure:\n\
                    - CURRENT DIRECTORY (worktree): Write ALL source code, tests, package.json here\n\
                    - SHARED DIRECTORY ({}): Write WORKLOG.md, STATUS.json here\n\n\
                    IMPLEMENTATION WORKFLOW (one segment at a time):\n\
                    1. Implement ONE segment from PLAN.md\n\
                    2. Write tests for that segment\n\
                    3. Update {}/WORKLOG.md with segment progress\n\
                    4. WAIT for SENTINEL review - SENTINEL will evaluate your segment\n\
                    5. If APPROVED, continue to next segment\n\
                    6. If CHANGES_REQUESTED, fix issues and update WORKLOG.md\n\
                    7. Repeat until all segments complete\n\
                    8. When ALL segments APPROVED, SENTINEL does final review\n\
                    9. After final APPROVAL, create PR\n\n\
                    You have full permissions. Install deps with 'npm install'. \
                    Commit after each segment. Document progress in {}/WORKLOG.md.",
                    contract, worklog, shared_path, shared_path, shared_path
                )
            } else {
                // Unknown contract state - treat as new session
                self.new_session_prompt(&ticket_path, &task_path, shared)
            }
        } else if plan_path.exists() {
            // PLAN.md exists but no CONTRACT.md yet - SENTINEL has not reviewed the plan.
            // Since --print mode exits after one response, we should NOT respawn FORGE
            // to wait for SENTINEL. Instead, just exit cleanly. The harness event loop
            // will spawn SENTINEL and then respawn FORGE once CONTRACT.md is written.
            info!("PLAN.md exists but no CONTRACT.md - FORGE has nothing to do until SENTINEL reviews");

            // Write a minimal WORKLOG.md so the harness knows progress was made
            let worklog_path = shared.join("WORKLOG.md");
            if !worklog_path.exists() {
                let plan = std::fs::read_to_string(&plan_path)
                    .unwrap_or_else(|_| "No PLAN.md found".to_string());
                format!(
                    "You are FORGE. Your PLAN.md has been submitted for review.\n\n\
                    --- PLAN.md ---\n{}\n\n\
                    IMPORTANT: Do NOT write any code or modify any files. Your plan is pending SENTINEL review.\n\
                    Simply respond with: 'PLAN.md submitted for SENTINEL review. Awaiting CONTRACT.md.'\n\
                    Do NOT rewrite PLAN.md. Do NOT start implementation. Wait for CONTRACT.md.",
                    plan
                )
            } else {
                // WORKLOG exists but no CONTRACT - implementation was started before contract?
                // Fall through to new session
                self.new_session_prompt(&ticket_path, &task_path, shared)
            }
        } else {
            // New session - read ticket and task
            self.new_session_prompt(&ticket_path, &task_path, shared)
        }
    }

    /// Build the prompt for a new session.
    fn new_session_prompt(&self, ticket_path: &Path, task_path: &Path, shared: &Path) -> String {
        let ticket = std::fs::read_to_string(ticket_path)
            .unwrap_or_else(|_| "No TICKET.md found".to_string());
        let task =
            std::fs::read_to_string(task_path).unwrap_or_else(|_| "No TASK.md found".to_string());
        let shared_path = shared.display();

        format!(
            "You are FORGE. Write a detailed implementation plan to {}/PLAN.md.\n\n\
            --- TICKET.md ---\n{}\n\n\
            --- TASK.md ---\n{}\n\n\
            IMPORTANT - Directory Structure:\n\
            - CURRENT DIRECTORY (worktree): Write ALL source code, tests, package.json here\n\
            - SHARED DIRECTORY ({}): Write PLAN.md, WORKLOG.md, STATUS.json here\n\n\
            STEPS (do these NOW):\n\
            1. Read {}/TICKET.md and {}/TASK.md from the shared directory\n\
            2. Read the codebase in current directory: README.md, package.json/Cargo.toml, src/\n\
            3. Write PLAN.md to shared directory with:\n\
               - ## Understanding: What you're building\n\
               - ## Segments: 1-3 files each, specific file paths in CURRENT DIRECTORY\n\
               - ## Files Changed: List every file you'll touch (all in current directory)\n\
               - ## Risks: What could go wrong\n\n\
             Write PLAN.md to shared directory now. Do NOT write any code yet - only the plan.",
            shared_path, ticket, task, shared_path, shared_path, shared_path
        )
    }

    /// Build the prompt for PR creation after final SENTINEL approval.
    fn build_forge_pr_prompt(&self, shared: &Path) -> String {
        let shared_path = shared.display();
        let final_review_path = shared.join("final-review.md");
        let final_review = std::fs::read_to_string(&final_review_path)
            .unwrap_or_else(|_| "No final-review.md found".to_string());
        let contract_path = shared.join("CONTRACT.md");
        let contract = std::fs::read_to_string(&contract_path)
            .unwrap_or_else(|_| "No CONTRACT.md found".to_string());
        let worklog_path = shared.join("WORKLOG.md");
        let worklog = std::fs::read_to_string(&worklog_path)
            .unwrap_or_else(|_| "No WORKLOG.md found".to_string());

        format!(
            "You are FORGE. SENTINEL has APPROVED and CERTIFIED your implementation. Create the PR.\n\n\
            --- FINAL REVIEW (SENTINEL CERTIFIED) ---\n{}\n\n\
            --- CONTRACT.md ---\n{}\n\n\
            --- WORKLOG.md ---\n{}\n\n\
            IMPORTANT: SENTINEL has reviewed and certified this code.\n\
            The final-review.md contains SENTINEL's signature and certification.\n\n\
            DIRECTORY STRUCTURE:\n\
            - CURRENT DIRECTORY (worktree): Source code is here\n\
            - SHARED DIRECTORY ({}): Write STATUS.json here\n\n\
            PR CREATION STEPS:\n\
            1. Ensure all changes committed: 'git status' then commit if needed\n\
            2. Push branch: 'git push -u origin HEAD'\n\
            3. Create PR using GitHub MCP create_pull_request:\n\
               - title: from CONTRACT summary\n\
               - body: include SENTINEL's PR description and CERTIFICATION\n\
               - head: current branch\n\
               - base: 'main'\n\
            4. Write {}/STATUS.json:\n\
               {{\n\
                 \"status\": \"PR_OPENED\",\n\
                 \"pr_url\": \"<pr url>\",\n\
                 \"pr_number\": <number>,\n\
                 \"branch\": \"<branch>\",\n\
                 \"sentinel_certified\": true,\n\
                 \"certification\": \"Reviewed and approved by SENTINEL\"\n\
               }}\n\n\
            Include SENTINEL's certification in PR body. This proves code quality.",
            final_review, contract, worklog, shared_path, shared_path
        )
    }

    /// Build the initial prompt for SENTINEL based on mode.
    fn build_sentinel_prompt(&self, shared: &Path, mode: &SentinelMode) -> String {
        let shared_path = shared.display();

        match mode {
            SentinelMode::PlanReview => {
                let plan_path = shared.join("PLAN.md");
                let plan = std::fs::read_to_string(&plan_path)
                    .unwrap_or_else(|_| "No PLAN.md found".to_string());
                let ticket_path = shared.join("TICKET.md");
                let ticket = std::fs::read_to_string(&ticket_path)
                    .unwrap_or_else(|_| "No TICKET.md found".to_string());

                format!(
                    "You are SENTINEL. Review this plan. Write ONLY to {}/CONTRACT.md.\n\n\
                    --- TICKET.md ---\n{}\n\n\
                    --- PLAN.md ---\n{}\n\n\
                    Check the plan has these sections:\n\
                    - ## Understanding (explains what we're building)\n\
                    - ## Segments (each with Files and Definition of Done)\n\
                    - ## Files Changed (specific file paths)\n\
                    - ## Risks (identified risks)\n\n\
                    APPROVE if all sections exist and are specific (real file paths, real criteria).\n\
                    REJECT if generic/placeholder content (e.g. '[Task 1 description]').\n\n\
                    Write CONTRACT.md now:\n\
                    ---\n\
                    status: AGREED | ISSUES\n\
                    summary: <one line>\n\
                    definition_of_done:\n\
                    - <criterion from plan>\n\
                    objections:\n\
                    - <specific issue or 'None'>",
                    shared_path, ticket, plan
                )
            }
            SentinelMode::SegmentEval(n) => {
                let worklog_path = shared.join("WORKLOG.md");
                let worklog = std::fs::read_to_string(&worklog_path)
                    .unwrap_or_else(|_| "No WORKLOG.md found".to_string());
                let contract_path = shared.join("CONTRACT.md");
                let contract = std::fs::read_to_string(&contract_path)
                    .unwrap_or_else(|_| "No CONTRACT.md found".to_string());

                format!(
                    "You are SENTINEL. Evaluate segment {}.\n\n\
                    --- CONTRACT.md ---\n{}\n\n\
                    --- WORKLOG.md ---\n{}\n\n\
                    SHARED: {}\n\n\
                    EVALUATE:\n\
                    1. Run tests: 'npm test' or 'cargo test'\n\
                    2. Check CONTRACT criteria all met\n\
                    3. Check test coverage - new code has tests\n\
                    4. Check standards - follows CODING.md\n\
                    5. Check for regressions - existing tests pass\n\n\
                    Write {}/segment-{}-eval.md:\n\
                    - ## Verdict: APPROVED | CHANGES_REQUESTED\n\
                    - ## Specific feedback: issues with file:line format\n\
                    - APPROVED = certified for this segment\n\
                    - CHANGES_REQUESTED = FORGE fixes and re-submits",
                    n, contract, worklog, shared_path, shared_path, n
                )
            }
            SentinelMode::FinalReview => {
                let worklog_path = shared.join("WORKLOG.md");
                let worklog = std::fs::read_to_string(&worklog_path)
                    .unwrap_or_else(|_| "No WORKLOG.md found".to_string());
                let contract_path = shared.join("CONTRACT.md");
                let contract = std::fs::read_to_string(&contract_path)
                    .unwrap_or_else(|_| "No CONTRACT.md found".to_string());

                format!(
                    "You are SENTINEL. FINAL REVIEW.\n\n\
                    --- CONTRACT.md ---\n{}\n\n\
                    --- WORKLOG.md ---\n{}\n\n\
                    SHARED: {}\n\n\
                    FINAL CHECKLIST:\n\
                    1. All segment-eval.md files show APPROVED\n\
                    2. All CONTRACT criteria verified\n\
                    3. All tests passing\n\
                    4. No regressions\n\n\
                    Write {}/final-review.md:\n\
                    - ## Verdict: APPROVED | REJECTED\n\
                    - ## Summary: what was implemented\n\
                    - ## PR description: for PR body (if APPROVED)\n\
                    - ## Certification: 'Code certified by SENTINEL - meets all acceptance criteria'\n\
                    - ## Signature: 'Reviewed and approved by SENTINEL on [date]'\n\n\
                    If APPROVED, FORGE creates PR with your description.\n\
                    If REJECTED, list issues FORGE must fix.",
                    contract, worklog, shared_path, shared_path
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

        let child = manager
            .spawn_forge(&self.pair_id, &self.ticket_id, &self.worktree, &self.shared)
            .await?;

        // Add extra environment variables
        // Note: This doesn't work after spawn, so we need to handle this differently
        // For now, the extra_env is not used, but could be added to the Command before spawn

        Ok(child)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    #[test]
    fn test_sentinel_mode_segment_value() {
        assert_eq!(SentinelMode::PlanReview.segment_value(), "");
        assert_eq!(SentinelMode::SegmentEval(3).segment_value(), "3");
        assert_eq!(SentinelMode::FinalReview.segment_value(), "final");
    }

    #[test]
    fn test_plan_review_prompt_uses_shared_absolute_paths() {
        let manager = ProcessManager::new("ghp_test");
        let prompt =
            manager.build_sentinel_prompt(Path::new("/tmp/shared"), &SentinelMode::PlanReview);

        assert!(prompt.contains("--- TICKET.md ---"));
        assert!(prompt.contains("Write ONLY to /tmp/shared/CONTRACT.md"));
        assert!(prompt.contains("status: AGREED | ISSUES"));
        assert!(prompt.contains("REJECT if generic/placeholder content"));
        assert!(prompt.contains("definition_of_done:"));
    }

    #[test]
    fn test_segment_eval_prompt_uses_shared_absolute_paths() {
        let manager = ProcessManager::new("ghp_test");
        let prompt =
            manager.build_sentinel_prompt(Path::new("/tmp/shared"), &SentinelMode::SegmentEval(3));

        assert!(prompt.contains("--- CONTRACT.md ---"));
        assert!(prompt.contains("Write /tmp/shared/segment-3-eval.md"));
    }

    #[test]
    fn test_final_review_prompt_uses_shared_absolute_paths() {
        let manager = ProcessManager::new("ghp_test");
        let prompt =
            manager.build_sentinel_prompt(Path::new("/tmp/shared"), &SentinelMode::FinalReview);

        assert!(prompt.contains("--- CONTRACT.md ---"));
        assert!(prompt.contains("Write /tmp/shared/final-review.md"));
    }
}
