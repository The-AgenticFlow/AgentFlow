/// Process Management for AgentFlow Pair Harness
///
/// Spawns and manages FORGE (long-running) and SENTINEL (ephemeral) processes.
///
/// # Key Constraints:
/// - FORGE is long-running and may undergo context resets
/// - SENTINEL is ephemeral: spawned on-demand, exits after evaluation
/// - No raw Git credentials provided to FORGE (must use MCP tools)
use anyhow::{Context, Result};
use std::collections::HashMap;
use std::path::PathBuf;
use std::process::Stdio;
use tokio::process::{Child, Command};
use tracing::{error, info, warn};

/// Process handle for a running agent
pub struct AgentProcess {
    /// The child process
    pub child: Child,
    /// Agent type (FORGE or SENTINEL)
    pub agent_type: AgentType,
    /// Process start time
    pub start_time: std::time::Instant,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum AgentType {
    Forge,
    Sentinel,
}

/// Manages spawning and lifecycle of FORGE and SENTINEL processes
pub struct ProcessManager {
    /// Pair identifier
    pair_id: String,
    /// Ticket identifier
    ticket_id: String,
    /// Path to worktree
    worktree_path: PathBuf,
    /// Path to shared directory
    shared_path: PathBuf,
    /// Redis URL
    redis_url: String,
    /// GitHub token for MCP
    github_token: String,
}

impl ProcessManager {
    pub fn new(
        pair_id: String,
        ticket_id: String,
        worktree_path: PathBuf,
        shared_path: PathBuf,
        redis_url: String,
        github_token: String,
    ) -> Self {
        Self {
            pair_id,
            ticket_id,
            worktree_path,
            shared_path,
            redis_url,
            github_token,
        }
    }

    /// Spawns a FORGE process (long-running)
    ///
    /// # Validation: C4-01 No Raw Git in Code
    /// FORGE is not spawned with git credentials - must use MCP tools
    pub async fn spawn_forge(&self) -> Result<AgentProcess> {
        info!(
            pair_id = %self.pair_id,
            ticket_id = %self.ticket_id,
            worktree = %self.worktree_path.display(),
            "Spawning FORGE process"
        );

        let env_vars = self.build_forge_env();

        // Validation: C4-01 - FORGE has NO git credentials in environment
        // It must use MCP tools (github-mcp-server) for all git operations
        let child = Command::new("claude-code")
            .current_dir(&self.worktree_path)
            .envs(env_vars)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .context("Failed to spawn FORGE process")?;

        info!(
            pair_id = %self.pair_id,
            pid = child.id(),
            "FORGE process spawned successfully"
        );

        Ok(AgentProcess {
            child,
            agent_type: AgentType::Forge,
            start_time: std::time::Instant::now(),
        })
    }

    /// Spawns a SENTINEL process (ephemeral)
    ///
    /// # Validation: C2-01 SENTINEL is Ephemeral
    /// SENTINEL is spawned only on event (WORKLOG.md change)
    /// Process terminates immediately after eval write
    /// No long-running loop
    pub async fn spawn_sentinel(&self) -> Result<AgentProcess> {
        info!(
            pair_id = %self.pair_id,
            ticket_id = %self.ticket_id,
            "Spawning ephemeral SENTINEL process"
        );

        let env_vars = self.build_sentinel_env();

        // Validation: C2-01 - SENTINEL is ephemeral, spawned on-demand
        let child = Command::new("claude-code")
            .current_dir(&self.shared_path)
            .envs(env_vars)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .context("Failed to spawn SENTINEL process")?;

        info!(
            pair_id = %self.pair_id,
            pid = child.id(),
            "SENTINEL process spawned successfully (ephemeral)"
        );

        Ok(AgentProcess {
            child,
            agent_type: AgentType::Sentinel,
            start_time: std::time::Instant::now(),
        })
    }

    /// Waits for a SENTINEL process to complete
    ///
    /// # Validation: C2-01 SENTINEL is Ephemeral
    /// SENTINEL exits immediately after writing evaluation
    pub async fn wait_sentinel_completion(&self, mut sentinel: AgentProcess) -> Result<()> {
        info!(
            pair_id = %self.pair_id,
            "Waiting for SENTINEL to complete evaluation"
        );

        let status = sentinel
            .child
            .wait()
            .await
            .context("Failed to wait for SENTINEL process")?;

        let duration = sentinel.start_time.elapsed();

        if status.success() {
            info!(
                pair_id = %self.pair_id,
                duration_secs = duration.as_secs(),
                "SENTINEL completed successfully"
            );
        } else {
            warn!(
                pair_id = %self.pair_id,
                exit_code = ?status.code(),
                duration_secs = duration.as_secs(),
                "SENTINEL exited with error"
            );
        }

        // Validation: C2-01 - SENTINEL process terminates immediately after eval
        // No long-running state retained
        Ok(())
    }

    /// Terminates a running agent process
    pub async fn terminate_process(&self, mut process: AgentProcess) -> Result<()> {
        let agent_type = match process.agent_type {
            AgentType::Forge => "FORGE",
            AgentType::Sentinel => "SENTINEL",
        };

        info!(
            pair_id = %self.pair_id,
            agent_type = agent_type,
            pid = process.child.id(),
            "Terminating process"
        );

        // Try graceful shutdown first
        if let Err(e) = process.child.kill().await {
            warn!(
                pair_id = %self.pair_id,
                agent_type = agent_type,
                error = %e,
                "Failed to kill process"
            );
        }

        // Wait for process to actually exit
        if let Ok(status) = process.child.wait().await {
            info!(
                pair_id = %self.pair_id,
                agent_type = agent_type,
                exit_code = ?status.code(),
                "Process terminated"
            );
        }

        Ok(())
    }

    /// Builds environment variables for FORGE
    ///
    /// # Validation: C4-01 No Raw Git in Code
    /// Git credentials are NOT included - FORGE must use MCP
    fn build_forge_env(&self) -> HashMap<String, String> {
        let mut env = HashMap::new();

        // Pair context
        env.insert("SPRINTLESS_PAIR_ID".to_string(), self.pair_id.clone());
        env.insert("SPRINTLESS_TICKET_ID".to_string(), self.ticket_id.clone());
        env.insert(
            "SPRINTLESS_WORKTREE".to_string(),
            self.worktree_path.display().to_string(),
        );
        env.insert(
            "SPRINTLESS_SHARED".to_string(),
            self.shared_path.display().to_string(),
        );

        // MCP configuration - only non-sensitive config
        env.insert("SPRINTLESS_REDIS_URL".to_string(), self.redis_url.clone());

        // Validation: C4-01 - NO raw GitHub token for FORGE
        // The token is only in the MCP server config (docker env in mcp.json)
        // FORGE MUST use github-mcp-server for all git operations
        // NO git credentials (GIT_USER, GIT_EMAIL, GIT_TOKEN, GITHUB_TOKEN, etc.)

        // Agent role
        env.insert("SPRINTLESS_AGENT_ROLE".to_string(), "FORGE".to_string());

        env
    }

    /// Builds environment variables for SENTINEL
    fn build_sentinel_env(&self) -> HashMap<String, String> {
        let mut env = HashMap::new();

        // Pair context
        env.insert("SPRINTLESS_PAIR_ID".to_string(), self.pair_id.clone());
        env.insert("SPRINTLESS_TICKET_ID".to_string(), self.ticket_id.clone());
        env.insert(
            "SPRINTLESS_WORKTREE".to_string(),
            self.worktree_path.display().to_string(),
        );
        env.insert(
            "SPRINTLESS_SHARED".to_string(),
            self.shared_path.display().to_string(),
        );

        // MCP configuration
        env.insert("SPRINTLESS_REDIS_URL".to_string(), self.redis_url.clone());
        env.insert(
            "SPRINTLESS_GITHUB_TOKEN".to_string(),
            self.github_token.clone(),
        );

        // Agent role
        env.insert("SPRINTLESS_AGENT_ROLE".to_string(), "SENTINEL".to_string());

        // SENTINEL is read-only - no git credentials needed
        env
    }

    /// Checks if a FORGE process needs a context reset
    pub async fn check_forge_alive(&self, process: &mut AgentProcess) -> Result<bool> {
        // Try to get exit status without blocking
        match process.child.try_wait() {
            Ok(Some(status)) => {
                info!(
                    pair_id = %self.pair_id,
                    exit_code = ?status.code(),
                    "FORGE process has exited"
                );
                Ok(false)
            }
            Ok(None) => {
                // Process still running
                Ok(true)
            }
            Err(e) => {
                error!(
                    pair_id = %self.pair_id,
                    error = %e,
                    "Failed to check FORGE process status"
                );
                Err(e.into())
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_forge_env_no_git_credentials() {
        let manager = ProcessManager::new(
            "pair-1".to_string(),
            "T-42".to_string(),
            PathBuf::from("/project/worktrees/pair-1"),
            PathBuf::from("/project/.sprintless/pairs/pair-1/shared"),
            "redis://localhost".to_string(),
            "ghp_token".to_string(),
        );

        let env = manager.build_forge_env();

        // Validation: C4-01 - Ensure NO git credentials
        assert!(!env.contains_key("GIT_USER"));
        assert!(!env.contains_key("GIT_EMAIL"));
        assert!(!env.contains_key("GIT_TOKEN"));
        assert!(!env.contains_key("GIT_CREDENTIALS"));
        assert!(!env.contains_key("GITHUB_TOKEN")); // MCP has this, not FORGE

        // Ensure required env vars are present
        assert_eq!(env.get("SPRINTLESS_PAIR_ID").unwrap(), "pair-1");
        assert_eq!(env.get("SPRINTLESS_TICKET_ID").unwrap(), "T-42");
        assert_eq!(env.get("SPRINTLESS_AGENT_ROLE").unwrap(), "FORGE");
    }

    #[test]
    fn test_sentinel_env() {
        let manager = ProcessManager::new(
            "pair-2".to_string(),
            "T-99".to_string(),
            PathBuf::from("/project/worktrees/pair-2"),
            PathBuf::from("/project/.sprintless/pairs/pair-2/shared"),
            "redis://localhost".to_string(),
            "ghp_token".to_string(),
        );

        let env = manager.build_sentinel_env();

        assert_eq!(env.get("SPRINTLESS_PAIR_ID").unwrap(), "pair-2");
        assert_eq!(env.get("SPRINTLESS_TICKET_ID").unwrap(), "T-99");
        assert_eq!(env.get("SPRINTLESS_AGENT_ROLE").unwrap(), "SENTINEL");
    }
}
