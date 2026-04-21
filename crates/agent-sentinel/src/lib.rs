// crates/agent-sentinel/src/lib.rs
//! SENTINEL agent - ephemeral code reviewer spawned per evaluation.
//!
//! Per the architecture (docs/forge-sentinel-arch.md):
//! - SENTINEL is NOT a long-running process
//! - It is spawned fresh for each evaluation task (plan review, segment eval, final review)
//! - It has no memory of previous evaluations
//! - The harness manages its lifecycle via inotify events

use anyhow::{anyhow, Result};
use async_trait::async_trait;
use pair_harness::process::SentinelMode;
use pair_harness::types::{Contract, FinalReview, SegmentEval, TimeoutProfile};
use pocketflow_core::{Action, Node, SharedStore};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::path::PathBuf;
use std::time::Duration;
use tracing::{info, warn};

/// Status written by SENTINEL after evaluation.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SentinelStatus {
    /// Verdict: "approved" or "changes_requested"
    pub verdict: String,
    /// Segment number (if segment evaluation)
    pub segment: Option<u32>,
    /// PR number (if final review)
    pub pr_number: Option<u64>,
    /// Whether spec was verified
    pub spec_verified: bool,
    /// Blockers if changes requested
    #[serde(skip_serializing_if = "Vec::is_empty", default)]
    pub blockers: Vec<Blocker>,
    /// Notes
    pub notes: Option<String>,
}

/// A blocker item for changes requested.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Blocker {
    pub file: String,
    pub line: u32,
    pub kind: String,
    pub description: String,
    pub fix: String,
}

/// Configuration for SENTINEL spawning.
#[derive(Debug, Clone)]
pub struct SentinelConfig {
    /// Pair ID (e.g., "forge-1")
    pub pair_id: String,
    /// Ticket ID (e.g., "T-003")
    pub ticket_id: String,
    /// Path to the worktree
    pub worktree: PathBuf,
    /// Path to the shared directory
    pub shared: PathBuf,
    /// Evaluation mode
    pub mode: SentinelMode,
    /// Path to SENTINEL persona
    pub persona_path: PathBuf,
    /// Timeout profile from contract (parsed from CONTRACT.md)
    pub timeout_profile: Option<TimeoutProfile>,
}

/// SENTINEL node for code review.
///
/// This node spawns an ephemeral SENTINEL process to evaluate:
/// - Plan reviews (CONTRACT.md creation)
/// - Segment evaluations (segment-N-eval.md)
/// - Final reviews (final-review.md)
pub struct SentinelNode {
    /// Configuration for this evaluation
    config: SentinelConfig,
}

impl SentinelNode {
    /// Create a new SENTINEL node for evaluation.
    pub fn new(config: SentinelConfig) -> Self {
        Self { config }
    }

    /// Load the SENTINEL persona from the agent definition file.
    async fn load_persona(&self) -> Result<String> {
        let content = tokio::fs::read_to_string(&self.config.persona_path).await
            .map_err(|e| anyhow!(
                "Failed to load sentinel persona from {:?}: {}",
                self.config.persona_path, e
            ))?;
        Ok(content)
    }

    /// Build the prompt for SENTINEL based on evaluation mode.
    async fn build_prompt(&self, persona: &str) -> Result<String> {
        let shared = &self.config.shared;
        let mode_prompt = match &self.config.mode {
            SentinelMode::PlanReview => {
                format!(
                    r#"SENTINEL MODE: PLAN REVIEW

Your task:
1. Read {}/PLAN.md
2. Read {}/TICKET.md (or the original issue)
3. Read orchestration/agent/arch/patterns.md and orchestration/agent/standards/CODING.md if available
4. Evaluate the plan against acceptance criteria
5. Write {}/CONTRACT.md with status AGREED or ISSUES

If ISSUES, list specific objections.
If AGREED, extract the definition of done as contract terms.

When done, exit. This is an ephemeral process."#,
                    shared.display(),
                    shared.display(),
                    shared.display()
                )
            }
            SentinelMode::SegmentEval(segment) => {
                format!(
                    r#"SENTINEL MODE: SEGMENT {} EVALUATION

Your task:
1. Read {}/WORKLOG.md - find segment {} entry
2. Read {}/CONTRACT.md - these are the acceptance criteria
3. Read files changed in segment {}
4. Run tests if available
5. Run linter on changed files
6. Evaluate against the five criteria:
   - Correctness
   - Test coverage
   - Standards compliance
   - Code quality
   - No regressions
7. Write {}/segment-{}-eval.md with verdict

Verdict must be APPROVED or CHANGES_REQUESTED.
If CHANGES_REQUESTED, provide file:line:problem:fix for each item.

When done, exit. This is an ephemeral process."#,
                    segment,
                    shared.display(),
                    segment,
                    shared.display(),
                    segment,
                    shared.display(),
                    segment
                )
            }
            SentinelMode::FinalReview => {
                format!(
                    r#"SENTINEL MODE: FINAL REVIEW

Your task:
1. Read all segment-N-eval.md files in {}/
2. Verify all segments have verdict: APPROVED
3. Run full test suite (all tests, not just changed)
4. Run full linter (entire project)
5. Check every CONTRACT acceptance criterion
6. Write {}/final-review.md with verdict

Verdict must be APPROVED or REJECTED.
If APPROVED, include PR description (this becomes the GitHub PR body).
If REJECTED, list remaining issues.

When done, exit. This is an ephemeral process."#,
                    shared.display(),
                    shared.display()
                )
            }
        };

        Ok(format!(
            "{}\n\n---\n\n# Current Task\n\nPair: {}\nTicket: {}\nWorktree: {}\n\n{}",
            persona,
            self.config.pair_id,
            self.config.ticket_id,
            self.config.worktree.display(),
            mode_prompt
        ))
    }

    /// Resolve the effective timeout for this evaluation.
    /// Uses the contract timeout profile if available, otherwise falls back to mode-based defaults.
    fn resolve_timeout(&self) -> Duration {
        const ENV_OVERHEAD_SECS: u64 = 45;

        let base_secs = match &self.config.timeout_profile {
            Some(profile) => match &self.config.mode {
                SentinelMode::PlanReview => profile.plan_review_secs,
                SentinelMode::SegmentEval(_) => profile.segment_eval_secs,
                SentinelMode::FinalReview => profile.final_review_secs,
            },
            None => match &self.config.mode {
                SentinelMode::PlanReview => 120,
                SentinelMode::SegmentEval(_) => 300,
                SentinelMode::FinalReview => 480,
            },
        };

        Duration::from_secs(base_secs + ENV_OVERHEAD_SECS)
    }

    /// Spawn SENTINEL process and wait for completion.
    async fn spawn_and_wait(&self, prompt: &str) -> Result<SentinelStatus> {
        let log_dir = self.config.shared.join("logs");
        tokio::fs::create_dir_all(&log_dir).await?;

        let log_path = log_dir.join(format!(
            "sentinel-{}-{}.log",
            self.config.pair_id,
            chrono::Utc::now().format("%Y%m%d-%H%M%S")
        ));
        let log_file = std::fs::File::create(&log_path)?;
        let log_file_err = log_file.try_clone()?;

        info!(
            pair = %self.config.pair_id,
            mode = ?self.config.mode,
            "Spawning SENTINEL process (ephemeral)"
        );

        // Ensure sentinel working directory exists
        let sentinel_dir = self.config.shared.join("sentinel");
        tokio::fs::create_dir_all(&sentinel_dir).await?;

        let mut child = tokio::process::Command::new("claude")
            .args(["--print", "--output-format", "json"])
            .arg("--dangerously-skip-permissions")
            .args(["--allowedTools", "Read,Bash,WebFetch"])
            .current_dir(&sentinel_dir)
            .env(
                "ANTHROPIC_API_KEY",
                std::env::var("ANTHROPIC_API_KEY").unwrap_or_default(),
            )
            .env("SPRINTLESS_PAIR_ID", &self.config.pair_id)
            .env("SPRINTLESS_TICKET_ID", &self.config.ticket_id)
            .env("SPRINTLESS_SHARED", self.config.shared.to_str().unwrap_or(""))
            .env("SPRINTLESS_WORKTREE", self.config.worktree.to_str().unwrap_or(""))
            .stdin(std::process::Stdio::piped())
            .stdout(log_file)
            .stderr(log_file_err)
            .spawn()
            .map_err(|e| anyhow!("Failed to spawn SENTINEL: {:#}", e))?;

        // Write prompt to stdin
        if let Some(mut stdin) = child.stdin.take() {
            use tokio::io::AsyncWriteExt;
            stdin.write_all(prompt.as_bytes()).await
                .map_err(|e| anyhow!("Failed to write prompt to stdin: {:#}", e))?;
        }

        // Wait with timeout (determined by contract complexity or fallback)
        let timeout = self.resolve_timeout();
        let result = tokio::time::timeout(timeout, child.wait()).await;

        match result {
            Err(_) => {
                child.kill().await?;
                warn!(pair = %self.config.pair_id, "SENTINEL timed out after {}s", timeout.as_secs());
                return Ok(SentinelStatus {
                    verdict: "changes_requested".to_string(),
                    segment: None,
                    pr_number: None,
                    spec_verified: false,
                    blockers: vec![Blocker {
                        file: "SENTINEL".to_string(),
                        line: 0,
                        kind: "Timeout".to_string(),
                        description: "SENTINEL evaluation timed out".to_string(),
                        fix: "Re-run evaluation".to_string(),
                    }],
                    notes: Some("Evaluation timed out".to_string()),
                });
            }
            Ok(Ok(status)) if !status.success() => {
                warn!(pair = %self.config.pair_id, exit = ?status.code(), "SENTINEL failed");
            }
            _ => {}
        }

        // Read the evaluation result based on mode
        self.read_evaluation_result().await
    }

    /// Read the evaluation result from the appropriate file.
    async fn read_evaluation_result(&self) -> Result<SentinelStatus> {
        let shared = &self.config.shared;

        match &self.config.mode {
            SentinelMode::PlanReview => {
                // Read CONTRACT.md
                let contract_path = shared.join("CONTRACT.md");
                if contract_path.exists() {
                    let content = tokio::fs::read_to_string(&contract_path).await?;
                    let status = if content.contains("status: AGREED") || content.contains("AGREED") {
                        "approved"
                    } else {
                        "changes_requested"
                    };
                    Ok(SentinelStatus {
                        verdict: status.to_string(),
                        segment: None,
                        pr_number: None,
                        spec_verified: status == "approved",
                        blockers: vec![],
                        notes: Some("Plan review complete".to_string()),
                    })
                } else {
                    Ok(SentinelStatus {
                        verdict: "changes_requested".to_string(),
                        segment: None,
                        pr_number: None,
                        spec_verified: false,
                        blockers: vec![Blocker {
                            file: "CONTRACT.md".to_string(),
                            line: 0,
                            kind: "Missing".to_string(),
                            description: "CONTRACT.md was not written".to_string(),
                            fix: "SENTINEL must write CONTRACT.md".to_string(),
                        }],
                        notes: Some("Plan review failed - no CONTRACT.md".to_string()),
                    })
                }
            }
            SentinelMode::SegmentEval(segment) => {
                // Read segment-N-eval.md
                let eval_path = shared.join(format!("segment-{}-eval.md", segment));
                if eval_path.exists() {
                    let content = tokio::fs::read_to_string(&eval_path).await?;
                    let verdict = if content.contains("APPROVED") {
                        "approved"
                    } else {
                        "changes_requested"
                    };
                    Ok(SentinelStatus {
                        verdict: verdict.to_string(),
                        segment: Some(*segment),
                        pr_number: None,
                        spec_verified: verdict == "approved",
                        blockers: vec![],
                        notes: Some(format!("Segment {} evaluation complete", segment)),
                    })
                } else {
                    Ok(SentinelStatus {
                        verdict: "changes_requested".to_string(),
                        segment: Some(*segment),
                        pr_number: None,
                        spec_verified: false,
                        blockers: vec![Blocker {
                            file: format!("segment-{}-eval.md", segment),
                            line: 0,
                            kind: "Missing".to_string(),
                            description: "Segment evaluation file was not written".to_string(),
                            fix: "SENTINEL must write segment-N-eval.md".to_string(),
                        }],
                        notes: Some(format!("Segment {} evaluation failed", segment)),
                    })
                }
            }
            SentinelMode::FinalReview => {
                // Read final-review.md
                let review_path = shared.join("final-review.md");
                if review_path.exists() {
                    let content = tokio::fs::read_to_string(&review_path).await?;
                    let verdict = if content.contains("APPROVED") {
                        "approved"
                    } else {
                        "changes_requested"
                    };
                    Ok(SentinelStatus {
                        verdict: verdict.to_string(),
                        segment: None,
                        pr_number: None,
                        spec_verified: verdict == "approved",
                        blockers: vec![],
                        notes: Some("Final review complete".to_string()),
                    })
                } else {
                    Ok(SentinelStatus {
                        verdict: "changes_requested".to_string(),
                        segment: None,
                        pr_number: None,
                        spec_verified: false,
                        blockers: vec![Blocker {
                            file: "final-review.md".to_string(),
                            line: 0,
                            kind: "Missing".to_string(),
                            description: "Final review file was not written".to_string(),
                            fix: "SENTINEL must write final-review.md".to_string(),
                        }],
                        notes: Some("Final review failed".to_string()),
                    })
                }
            }
        }
    }
}

#[async_trait]
impl Node for SentinelNode {
    fn name(&self) -> &str {
        "sentinel"
    }

    async fn prep(&self, _store: &SharedStore) -> Result<Value> {
        // Load persona
        let persona = self.load_persona().await?;
        // Build prompt based on mode
        let prompt = self.build_prompt(&persona).await?;
        Ok(json!({
            "prompt": prompt,
            "mode": format!("{:?}", self.config.mode),
            "pair_id": self.config.pair_id,
            "ticket_id": self.config.ticket_id,
        }))
    }

    async fn exec(&self, prep_result: Value) -> Result<Value> {
        let prompt = prep_result["prompt"].as_str().unwrap_or("");
        
        // Spawn SENTINEL and wait for evaluation
        let status = self.spawn_and_wait(prompt).await?;
        
        Ok(json!({
            "verdict": status.verdict,
            "segment": status.segment,
            "pr_number": status.pr_number,
            "spec_verified": status.spec_verified,
            "blockers": status.blockers,
            "notes": status.notes,
            "pair_id": self.config.pair_id,
            "ticket_id": self.config.ticket_id,
        }))
    }

    async fn post(&self, store: &SharedStore, exec_result: Value) -> Result<Action> {
        let verdict = exec_result["verdict"].as_str().unwrap_or("changes_requested");
        
        info!(
            pair = %exec_result["pair_id"].as_str().unwrap_or(""),
            verdict,
            "SENTINEL evaluation complete"
        );

        // Store the evaluation result for downstream nodes
        store.set(
            "sentinel_evaluation",
            exec_result.clone(),
        ).await;

        match verdict {
            "approved" => Ok(Action::new("approved")),
            _ => Ok(Action::new("changes_requested")),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn test_sentinel_config() {
        let config = SentinelConfig {
            pair_id: "forge-1".to_string(),
            ticket_id: "T-003".to_string(),
            worktree: PathBuf::from("/tmp/worktree"),
            shared: PathBuf::from("/tmp/shared"),
            mode: SentinelMode::SegmentEval(1),
            persona_path: PathBuf::from("orchestration/agent/agents/sentinel.agent.md"),
            timeout_profile: None,
        };
        
        assert_eq!(config.pair_id, "forge-1");
        assert_eq!(config.mode, SentinelMode::SegmentEval(1));
    }
}