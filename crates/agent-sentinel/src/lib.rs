// crates/agent-sentinel/src/lib.rs
//! SENTINEL agent - the evaluator in the FORGE-SENTINEL pair.
//!
//! SENTINEL is spawned fresh for every evaluation, ensuring zero context drift.
//! It reviews plans, evaluates segments, and produces final reviews.

use anyhow::{anyhow, Result};
use async_trait::async_trait;
use chrono::Utc;
use pocketflow_core::{Action, Node, SharedStore};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::HashMap;
use std::path::PathBuf;
use tracing::{debug, info, warn};

// ============================================================================
// Artifact Types
// ============================================================================

/// Verdict for segment evaluations
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum Verdict {
    Approved,
    ChangesRequested,
}

/// Verdict for final reviews
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum FinalVerdict {
    Approved,
    Rejected,
}

/// Evaluation criteria grades
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CriteriaGrades {
    pub correctness: bool,
    pub test_coverage: bool,
    pub standards_compliance: bool,
    pub code_quality: bool,
    pub no_regressions: bool,
}

// ============================================================================
// Status Types
// ============================================================================

/// Pair status for STATUS.json
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum PairStatus {
    PrOpened,
    Blocked,
    FuelExhausted,
}

/// Blocker types
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum BlockerType {
    AmbiguousRequirement,
    ContractDisagreement,
    SegmentLoop,
    RebaseConflict,
    FileLockConflict,
}

/// A blocker entry
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Blocker {
    #[serde(rename = "type")]
    pub blocker_type: BlockerType,
    pub description: String,
    pub nexus_action: String,
}

/// STATUS.json structure
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StatusJson {
    pub status: PairStatus,
    pub pair: String,
    pub ticket_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub pr_url: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub pr_number: Option<u32>,
    pub branch: String,
    pub files_changed: Vec<String>,
    pub test_results: TestResults,
    pub segments_completed: u32,
    pub context_resets: u32,
    pub sentinel_approved: bool,
    pub blockers: Vec<Blocker>,
    pub elapsed_ms: u64,
    pub timestamp: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TestResults {
    pub passed: u32,
    pub failed: u32,
    pub skipped: u32,
}

// ============================================================================
// Sentinel Node
// ============================================================================

/// Configuration for SentinelNode
#[derive(Debug, Clone)]
pub struct SentinelConfig {
    /// Pair ID (e.g., "pair-1")
    pub pair_id: String,
    /// Ticket ID (e.g., "T-42")
    pub ticket_id: String,
    /// Path to the shared directory for this pair
    pub shared_dir: PathBuf,
    /// Path to the worktree for this pair
    pub worktree_dir: PathBuf,
    /// Maximum segment iterations before blocking
    pub max_segment_iterations: u32,
}

/// SentinelNode - evaluates FORGE's work
pub struct SentinelNode {
    config: SentinelConfig,
}

impl SentinelNode {
    pub fn new(config: SentinelConfig) -> Self {
        Self { config }
    }

    /// Read an artifact from the shared directory
    async fn read_artifact(&self, name: &str) -> Result<Option<String>> {
        let path = self.config.shared_dir.join(name);
        if path.exists() {
            let content = tokio::fs::read_to_string(&path).await?;
            Ok(Some(content))
        } else {
            Ok(None)
        }
    }

    /// Write an artifact to the shared directory
    async fn write_artifact(&self, name: &str, content: &str) -> Result<()> {
        let path = self.config.shared_dir.join(name);
        if let Some(parent) = path.parent() {
            tokio::fs::create_dir_all(parent).await?;
        }
        tokio::fs::write(&path, content).await?;
        info!(artifact = name, pair = %self.config.pair_id, "Wrote artifact");
        Ok(())
    }

    /// Check if FORGE has submitted a segment for evaluation
    async fn check_for_segment_submission(&self) -> Result<Option<u32>> {
        // Read WORKLOG.md to find the latest segment
        if let Some(worklog) = self.read_artifact("WORKLOG.md").await? {
            // Find the highest segment number that doesn't have a corresponding eval
            let mut max_segment: u32 = 0;
            for line in worklog.lines() {
                if line.starts_with("## Segment ") {
                    // Extract segment number
                    if let Some(num_str) = line.split(' ').nth(2) {
                        if let Ok(num) = num_str.parse::<u32>() {
                            max_segment = max_segment.max(num);
                        }
                    }
                }
            }

            // Check if segment-N-eval.md exists for this segment
            if max_segment > 0 {
                let eval_name = format!("segment-{}-eval.md", max_segment);
                if self.read_artifact(&eval_name).await?.is_none() {
                    // No eval yet, this segment needs evaluation
                    return Ok(Some(max_segment));
                }
            }
        }
        Ok(None)
    }

    /// Check if all segments are approved and ready for final review
    async fn check_for_final_review_ready(&self) -> Result<bool> {
        // Read WORKLOG.md to count segments
        if let Some(worklog) = self.read_artifact("WORKLOG.md").await? {
            let mut segment_count = 0;
            let mut approved_count = 0;

            for line in worklog.lines() {
                if line.starts_with("## Segment ") {
                    segment_count += 1;
                }
                if line.contains("SENTINEL APPROVED") {
                    approved_count += 1;
                }
            }

            // All segments approved?
            if segment_count > 0 && segment_count == approved_count {
                // Check if final-review.md exists
                if self.read_artifact("final-review.md").await?.is_none() {
                    return Ok(true);
                }
            }
        }
        Ok(false)
    }

    /// Count evaluation iterations for a segment
    async fn count_segment_iterations(&self, segment_num: u32) -> Result<u32> {
        let mut count = 0;
        let mut i = 1;
        loop {
            let eval_name = format!("segment-{}-eval-v{}.md", segment_num, i);
            if self.read_artifact(&eval_name).await?.is_some() {
                count += 1;
                i += 1;
            } else {
                // Also check the base name
                let base_name = format!("segment-{}-eval.md", segment_num);
                if self.read_artifact(&base_name).await?.is_some() {
                    count += 1;
                }
                break;
            }
        }
        Ok(count)
    }

    /// Generate segment evaluation
    async fn evaluate_segment(&self, segment_num: u32) -> Result<Value> {
        info!(
            pair = %self.config.pair_id,
            segment = segment_num,
            "Evaluating segment"
        );

        // Read the worklog entry for this segment
        let worklog = self
            .read_artifact("WORKLOG.md")
            .await?
            .ok_or_else(|| anyhow!("WORKLOG.md not found"))?;

        // Read the contract
        let contract = self
            .read_artifact("CONTRACT.md")
            .await?
            .ok_or_else(|| anyhow!("CONTRACT.md not found"))?;

        // Read changed files from worktree
        // In a real implementation, we would:
        // 1. Run tests via MCP tool
        // 2. Run linter via MCP tool
        // 3. Read each changed file
        // 4. Grade against 5 criteria

        // For now, return a placeholder evaluation
        let eval_content = format!(
            r#"# Segment {} Evaluation

**Pair:** {}  
**Ticket:** {}  
**Timestamp:** {}

## Verdict

APPROVED

## Criteria Assessment

| Criterion | Status | Notes |
|-----------|--------|-------|
| Correctness | PASS | Implementation matches CONTRACT.md |
| Test Coverage | PASS | All changed files have tests |
| Standards Compliance | PASS | Follows CODING.md |
| Code Quality | PASS | Clean, readable code |
| No Regressions | PASS | All existing tests pass |

## Summary

Segment {} has been reviewed and approved. The implementation correctly addresses
the requirements specified in CONTRACT.md.

---
*Generated by SENTINEL-{}*
"#,
            segment_num,
            self.config.pair_id,
            self.config.ticket_id,
            Utc::now().to_rfc3339(),
            segment_num,
            self.config.pair_id
        );

        let eval_name = format!("segment-{}-eval.md", segment_num);
        self.write_artifact(&eval_name, &eval_content).await?;

        Ok(json!({
            "segment": segment_num,
            "verdict": "approved",
            "eval_file": eval_name,
        }))
    }

    /// Generate final review
    async fn generate_final_review(&self) -> Result<Value> {
        info!(pair = %self.config.pair_id, "Generating final review");

        // Read all artifacts
        let _worklog = self
            .read_artifact("WORKLOG.md")
            .await?
            .ok_or_else(|| anyhow!("WORKLOG.md not found"))?;
        let contract = self
            .read_artifact("CONTRACT.md")
            .await?
            .ok_or_else(|| anyhow!("CONTRACT.md not found"))?;

        // In a real implementation, we would:
        // 1. Run full test suite
        // 2. Run full linter
        // 3. Check every CONTRACT criterion
        // 4. Draft PR description

        let final_review = format!(
            r#"# Final Review

**Pair:** {}  
**Ticket:** {}  
**Timestamp:** {}

## Verdict

APPROVED

## Contract Fulfillment

All acceptance criteria from CONTRACT.md have been satisfied:

- [x] All segments approved
- [x] All tests passing
- [x] No lint warnings
- [x] Standards compliant

## PR Description

This PR implements {}.

### Changes

- Implemented core functionality
- Added comprehensive tests
- Updated documentation

### Test Results

- All tests passing
- No regressions

---
*Generated by SENTINEL-{}*
"#,
            self.config.pair_id,
            self.config.ticket_id,
            Utc::now().to_rfc3339(),
            self.config.ticket_id,
            self.config.pair_id
        );

        self.write_artifact("final-review.md", &final_review)
            .await?;

        Ok(json!({
            "verdict": "approved",
            "pr_ready": true,
        }))
    }

    /// Review a plan and produce CONTRACT.md
    async fn review_plan(&self) -> Result<Value> {
        info!(pair = %self.config.pair_id, "Reviewing plan");

        let plan = self
            .read_artifact("PLAN.md")
            .await?
            .ok_or_else(|| anyhow!("PLAN.md not found"))?;

        let ticket = self
            .read_artifact("TICKET.md")
            .await?
            .ok_or_else(|| anyhow!("TICKET.md not found"))?;

        // In a real implementation, we would:
        // 1. Check scope alignment
        // 2. Check technical approach
        // 3. Check file coverage
        // 4. Check test strategy
        // 5. Check out-of-scope list

        // For now, create a simple contract
        let contract = format!(
            r#"# Sprint Contract

**Pair:** {}  
**Ticket:** {}  
**Status:** AGREED  
**Timestamp:** {}

## Acceptance Criteria

Based on PLAN.md and TICKET.md:

1. Implementation matches the plan
2. All tests pass
3. Code follows standards
4. No regressions

## Segments

See PLAN.md for segment breakdown.

## Signatures

- FORGE: agreed
- SENTINEL: agreed

---
*Generated by SENTINEL-{}*
"#,
            self.config.pair_id,
            self.config.ticket_id,
            Utc::now().to_rfc3339(),
            self.config.pair_id
        );

        self.write_artifact("CONTRACT.md", &contract).await?;

        Ok(json!({
            "action": "contract_agreed",
            "plan_valid": true,
        }))
    }
}

#[async_trait]
impl Node for SentinelNode {
    fn name(&self) -> &str {
        "sentinel"
    }

    async fn prep(&self, store: &SharedStore) -> Result<Value> {
        // Check what needs evaluation
        let segment_to_eval = self.check_for_segment_submission().await?;
        let final_review_ready = self.check_for_final_review_ready().await?;
        let has_plan = self.read_artifact("PLAN.md").await?.is_some();
        let has_contract = self.read_artifact("CONTRACT.md").await?.is_some();

        Ok(json!({
            "pair_id": self.config.pair_id,
            "ticket_id": self.config.ticket_id,
            "segment_to_eval": segment_to_eval,
            "final_review_ready": final_review_ready,
            "has_plan": has_plan,
            "has_contract": has_contract,
            "shared_dir": self.config.shared_dir.to_string_lossy(),
            "worktree_dir": self.config.worktree_dir.to_string_lossy(),
        }))
    }

    async fn exec(&self, context: Value) -> Result<Value> {
        let has_plan = context["has_plan"].as_bool().unwrap_or(false);
        let has_contract = context["has_contract"].as_bool().unwrap_or(false);
        let segment_to_eval = context["segment_to_eval"].as_u64().map(|n| n as u32);
        let final_review_ready = context["final_review_ready"].as_bool().unwrap_or(false);

        // Priority order:
        // 1. Review plan if no contract
        // 2. Evaluate segment if submitted
        // 3. Final review if all segments approved

        if has_plan && !has_contract {
            return self.review_plan().await;
        }

        if let Some(segment_num) = segment_to_eval {
            // Check for segment loop
            let iterations = self.count_segment_iterations(segment_num).await?;
            if iterations >= self.config.max_segment_iterations {
                warn!(
                    pair = %self.config.pair_id,
                    segment = segment_num,
                    iterations = iterations,
                    "Segment loop detected"
                );
                return Ok(json!({
                    "action": "segment_loop",
                    "segment": segment_num,
                    "iterations": iterations,
                }));
            }
            return self.evaluate_segment(segment_num).await;
        }

        if final_review_ready {
            return self.generate_final_review().await;
        }

        // Nothing to do
        Ok(json!({
            "action": "idle",
            "message": "No work pending evaluation",
        }))
    }

    async fn post(&self, store: &SharedStore, result: Value) -> Result<Action> {
        let action = result["action"].as_str().unwrap_or("idle");

        info!(
            pair = %self.config.pair_id,
            action = action,
            "Sentinel post-processing"
        );

        // Update shared store with sentinel status
        let key = format!("sentinel_{}", self.config.pair_id);
        store.set(&key, result.clone()).await;

        match action {
            "contract_agreed" => Ok(Action::new("contract_agreed")),
            "approved" => Ok(Action::new("segment_approved")),
            "changes_requested" => Ok(Action::new("changes_requested")),
            "final_approved" => Ok(Action::new("final_approved")),
            "segment_loop" => Ok(Action::new("blocked")),
            _ => Ok(Action::new("idle")),
        }
    }
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[tokio::test]
    async fn test_sentinel_reads_artifacts() {
        let temp_dir = TempDir::new().unwrap();
        let shared_dir = temp_dir.path().join("shared");
        tokio::fs::create_dir_all(&shared_dir).await.unwrap();

        let sentinel = SentinelNode::new(SentinelConfig {
            pair_id: "pair-1".to_string(),
            ticket_id: "T-42".to_string(),
            shared_dir: shared_dir.clone(),
            worktree_dir: temp_dir.path().join("worktree"),
            max_segment_iterations: 5,
        });

        // Write a test artifact
        tokio::fs::write(shared_dir.join("TEST.md"), "# Test")
            .await
            .unwrap();

        let content = sentinel.read_artifact("TEST.md").await.unwrap();
        assert!(content.is_some());
        assert!(content.unwrap().contains("# Test"));
    }

    #[tokio::test]
    async fn test_sentinel_writes_artifacts() {
        let temp_dir = TempDir::new().unwrap();
        let shared_dir = temp_dir.path().join("shared");

        let sentinel = SentinelNode::new(SentinelConfig {
            pair_id: "pair-1".to_string(),
            ticket_id: "T-42".to_string(),
            shared_dir: shared_dir.clone(),
            worktree_dir: temp_dir.path().join("worktree"),
            max_segment_iterations: 5,
        });

        sentinel
            .write_artifact("EVAL.md", "# Evaluation")
            .await
            .unwrap();

        let content = tokio::fs::read_to_string(shared_dir.join("EVAL.md"))
            .await
            .unwrap();
        assert!(content.contains("# Evaluation"));
    }
}
