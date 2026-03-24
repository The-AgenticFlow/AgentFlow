// crates/pocketflow-core/src/command_gate.rs
//
// CommandGate — FORGE proposes, NEXUS approves.
// Dangerous shell commands are written to the SharedStore by the worker,
// then NEXUS reads and writes back a decision before the worker proceeds.

use anyhow::{bail, Result};
use serde::{Deserialize, Serialize};
use std::time::Duration;
use tracing::{info, warn};

use crate::SharedStore;

// ── Dangerous pattern list ── (loaded statically; see SECURITY.md) ────────

const DANGEROUS_PATTERNS: &[&str] = &[
    "rm -rf",
    "rm -r /",
    "chmod 777",
    "git push --force",
    "curl | sh",
    "wget | sh",
    "bash <(",
    "DROP TABLE",
    "DROP DATABASE",
    "DELETE FROM",
    "mkfs",
    "dd if=",
    "> /dev/",
];

/// Returns true if the command matches any known dangerous pattern.
pub fn is_dangerous(cmd: &str) -> bool {
    let lower = cmd.to_lowercase();
    DANGEROUS_PATTERNS
        .iter()
        .any(|p| lower.contains(&p.to_lowercase()))
}

// ── Proposal ─────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CommandProposal {
    pub worker_id:   String,
    pub command:     String,
    pub reason:      String,
    pub risk_level:  RiskLevel,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum RiskLevel {
    Low,
    Medium,
    High,
}

// ── Decision ─────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum CommandDecision {
    Approved,
    Rejected { reason: String },
}

// ── Gate (worker side) ───────────────────────────────────────────────────

pub struct CommandGate;

impl CommandGate {
    /// Check if a command needs approval, and if so block until NEXUS decides.
    /// Returns Ok(()) if approved or not dangerous, Err if rejected.
    ///
    /// Timeout: 60 seconds. If NEXUS doesn't respond, the command is rejected.
    pub async fn check_and_wait(
        store: &SharedStore,
        worker_id: &str,
        command: &str,
        reason: &str,
    ) -> Result<()> {
        if !is_dangerous(command) {
            return Ok(());
        }

        let proposal = CommandProposal {
            worker_id:  worker_id.to_string(),
            command:    command.to_string(),
            reason:     reason.to_string(),
            risk_level: RiskLevel::High,
        };

        let proposal_key  = format!("command_gate::{}",             worker_id);
        let decision_key  = format!("command_gate::{}::decision",   worker_id);

        // Write the proposal
        store.del(&decision_key).await; // clear stale decision
        store.set_typed(&proposal_key, &proposal).await?;
        store
            .emit(
                worker_id,
                "command_gate_proposed",
                serde_json::json!({ "command": command }),
            )
            .await;

        warn!(worker = worker_id, cmd = command, "dangerous command proposed — awaiting NEXUS");

        // Poll for decision (60s timeout)
        let timeout    = Duration::from_secs(60);
        let poll_interval = Duration::from_millis(500);
        let start      = std::time::Instant::now();

        loop {
            if start.elapsed() >= timeout {
                store.del(&proposal_key).await;
                bail!(
                    "CommandGate timeout: NEXUS did not respond within 60s for worker {}",
                    worker_id
                );
            }

            if let Some(decision) = store.get_typed::<CommandDecision>(&decision_key).await {
                // Clean up
                store.del(&proposal_key).await;
                store.del(&decision_key).await;

                return match decision {
                    CommandDecision::Approved => {
                        info!(worker = worker_id, cmd = command, "command approved by NEXUS");
                        store
                            .emit(worker_id, "command_gate_approved",
                                  serde_json::json!({ "command": command }))
                            .await;
                        Ok(())
                    }
                    CommandDecision::Rejected { reason } => {
                        warn!(worker = worker_id, cmd = command, %reason, "command rejected by NEXUS");
                        store
                            .emit(worker_id, "command_gate_rejected",
                                  serde_json::json!({ "command": command, "reason": reason }))
                            .await;
                        bail!("Command rejected by NEXUS: {}", reason)
                    }
                };
            }

            tokio::time::sleep(poll_interval).await;
        }
    }

    /// Called by NEXUS to approve a worker's pending command.
    pub async fn approve(store: &SharedStore, worker_id: &str) -> Result<()> {
        let decision_key = format!("command_gate::{}::decision", worker_id);
        store
            .set_typed(&decision_key, &CommandDecision::Approved)
            .await
    }

    /// Called by NEXUS to reject a worker's pending command.
    pub async fn reject(store: &SharedStore, worker_id: &str, reason: &str) -> Result<()> {
        let decision_key = format!("command_gate::{}::decision", worker_id);
        store
            .set_typed(
                &decision_key,
                &CommandDecision::Rejected { reason: reason.to_string() },
            )
            .await
    }

    /// Called by NEXUS during its poll loop to get any pending proposal.
    pub async fn pending_proposal(
        store: &SharedStore,
        worker_id: &str,
    ) -> Option<CommandProposal> {
        let proposal_key = format!("command_gate::{}", worker_id);
        store.get_typed::<CommandProposal>(&proposal_key).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::SharedStore;

    #[test]
    fn test_dangerous_pattern_detection() {
        assert!(is_dangerous("rm -rf /tmp/something"));
        assert!(is_dangerous("git push --force origin main"));
        assert!(is_dangerous("DROP TABLE users;"));
        assert!(!is_dangerous("cargo build --release"));
        assert!(!is_dangerous("ls -la"));
        assert!(!is_dangerous("cat README.md"));
    }

    #[tokio::test]
    async fn test_gate_approve_flow() {
        let store     = SharedStore::new_in_memory();
        let store2    = store.clone(); // simulated NEXUS side
        let worker_id = "forge-1";

        // Simulate NEXUS approving after 100ms
        tokio::spawn(async move {
            tokio::time::sleep(Duration::from_millis(100)).await;
            CommandGate::approve(&store2, worker_id).await.unwrap();
        });

        let result = CommandGate::check_and_wait(
            &store,
            worker_id,
            "rm -rf /tmp/old-build",
            "cleanup before fresh build",
        ).await;

        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn test_gate_reject_flow() {
        let store     = SharedStore::new_in_memory();
        let store2    = store.clone();
        let worker_id = "forge-2";

        tokio::spawn(async move {
            tokio::time::sleep(Duration::from_millis(100)).await;
            CommandGate::reject(&store2, worker_id, "too broad — scope to worker dir only").await.unwrap();
        });

        let result = CommandGate::check_and_wait(
            &store,
            worker_id,
            "rm -rf /workspace",
            "full reset",
        ).await;

        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("rejected by NEXUS"));
    }

    #[tokio::test]
    async fn test_safe_command_passes_immediately() {
        let store = SharedStore::new_in_memory();

        let result = CommandGate::check_and_wait(
            &store, "forge-1", "cargo test --release", "run tests",
        ).await;

        assert!(result.is_ok());
    }
}
