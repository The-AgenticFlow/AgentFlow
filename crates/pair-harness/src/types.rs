// crates/pair-harness/src/types.rs
//! Core types for the pair-harness system.

use serde::{Deserialize, Serialize};
use std::path::PathBuf;

/// Filesystem events detected by the watcher.
/// These drive the event-driven harness state machine.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FsEvent {
    /// FORGE submitted a segment (WORKLOG.md modified)
    WorklogUpdated,
    /// FORGE finished planning (PLAN.md created)
    PlanWritten,
    /// SENTINEL reviewed plan (CONTRACT.md created)
    ContractWritten,
    /// SENTINEL finished segment-N evaluation
    SegmentEvalWritten(u32),
    /// SENTINEL approved all segments (final-review.md created)
    FinalReviewWritten,
    /// Terminal signal (PR_OPENED, BLOCKED, FUEL_EXHAUSTED)
    StatusJsonWritten,
    /// Context reset requested (HANDOFF.md created)
    HandoffWritten,
}

/// Ticket information for assignment to a pair.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Ticket {
    /// Ticket identifier (e.g., "T-42")
    pub id: String,
    /// GitHub issue number
    pub issue_number: u64,
    /// Ticket title
    pub title: String,
    /// Ticket description/body
    pub body: String,
    /// GitHub issue URL
    pub url: String,
    /// Files that will be touched (for initial locking)
    pub touched_files: Vec<String>,
    /// Acceptance criteria extracted from the issue
    pub acceptance_criteria: Vec<String>,
}

/// Configuration for a pair slot.
#[derive(Debug, Clone)]
pub struct PairConfig {
    /// Pair identifier (e.g., "pair-1")
    pub pair_id: String,
    /// Path to the project root (contains .git)
    pub project_root: PathBuf,
    /// Path to the Git worktree for this pair
    pub worktree: PathBuf,
    /// Path to the shared directory for FORGE-SENTINEL communication
    pub shared: PathBuf,
    /// Optional Redis URL for shared store (if not provided, uses filesystem-based state)
    pub redis_url: Option<String>,
    /// GitHub token for MCP tools
    pub github_token: String,
    /// Maximum number of context resets allowed
    pub max_resets: u32,
    /// Timeout in seconds for watchdog (default: 1200 = 20 minutes)
    pub watchdog_timeout_secs: u64,
}

impl PairConfig {
    /// Create a new pair configuration with filesystem-based state.
    pub fn new(
        pair_id: impl Into<String>,
        project_root: &std::path::Path,
        github_token: impl Into<String>,
    ) -> Self {
        let pair_id = pair_id.into();
        Self {
            project_root: project_root.to_path_buf(),
            worktree: project_root.join("worktrees").join(&pair_id),
            shared: project_root
                .join("orchestration")
                .join("pairs")
                .join(&pair_id)
                .join("shared"),
            pair_id,
            redis_url: None,
            github_token: github_token.into(),
            max_resets: 10,
            watchdog_timeout_secs: 1200,
        }
    }

    /// Create a pair configuration with Redis backend.
    pub fn with_redis(
        pair_id: impl Into<String>,
        project_root: &std::path::Path,
        redis_url: impl Into<String>,
        github_token: impl Into<String>,
    ) -> Self {
        let pair_id = pair_id.into();
        Self {
            project_root: project_root.to_path_buf(),
            worktree: project_root.join("worktrees").join(&pair_id),
            shared: project_root
                .join("orchestration")
                .join("pairs")
                .join(&pair_id)
                .join("shared"),
            pair_id,
            redis_url: Some(redis_url.into()),
            github_token: github_token.into(),
            max_resets: 10,
            watchdog_timeout_secs: 1200,
        }
    }
}

/// Outcome of a pair's work on a ticket.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum PairOutcome {
    /// PR was opened successfully
    PrOpened {
        pr_url: String,
        pr_number: u64,
        branch: String,
    },
    /// Pair is blocked (needs human intervention)
    Blocked {
        reason: String,
        blockers: Vec<Blocker>,
    },
    /// Fuel exhausted (too many context resets or timeout)
    FuelExhausted { reason: String, reset_count: u32 },
}

/// A blocker preventing progress.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Blocker {
    /// Type of blocker
    #[serde(rename = "type")]
    pub blocker_type: String,
    /// Human-readable description
    pub description: String,
    /// Suggested action for NEXUS
    pub nexus_action: String,
}

/// Files changed - can be either a count (integer) or a list of paths.
/// FORGE may write either format depending on the skill version.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(untagged)]
pub enum FilesChanged {
    #[default]
    Unknown,
    Count(u64),
    List(Vec<String>),
}

impl FilesChanged {
    pub fn is_empty(&self) -> bool {
        match self {
            FilesChanged::Unknown => true,
            FilesChanged::Count(c) => *c == 0,
            FilesChanged::List(v) => v.is_empty(),
        }
    }

    pub fn to_list(&self) -> Vec<String> {
        match self {
            FilesChanged::Unknown => vec![],
            FilesChanged::Count(_) => vec![],
            FilesChanged::List(v) => v.clone(),
        }
    }
}

/// Status written to STATUS.json by FORGE.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StatusJson {
    /// Current status
    pub status: String,
    /// Pair identifier (optional - may not be present in all STATUS.json formats)
    #[serde(default)]
    pub pair: Option<String>,
    /// Ticket identifier - can be "ticket" or "ticket_id" in STATUS.json
    #[serde(alias = "ticket")]
    pub ticket_id: String,
    /// PR URL (if PR_OPENED)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub pr_url: Option<String>,
    /// PR number (if PR_OPENED)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub pr_number: Option<u64>,
    /// Branch name (optional - may not be present in all STATUS.json formats)
    #[serde(default)]
    pub branch: Option<String>,
    /// Files changed (can be count or list)
    #[serde(default)]
    pub files_changed: FilesChanged,
    /// Test results (optional)
    #[serde(default)]
    pub test_results: Option<TestResults>,
    /// Number of segments completed (optional)
    #[serde(default)]
    pub segments_completed: u32,
    /// Number of context resets (optional)
    #[serde(default)]
    pub context_resets: u32,
    /// Whether SENTINEL approved (optional)
    #[serde(default)]
    pub sentinel_approved: bool,
    /// Active blockers (if BLOCKED)
    #[serde(skip_serializing_if = "Vec::is_empty", default)]
    pub blockers: Vec<Blocker>,
    /// Elapsed time in milliseconds (optional)
    #[serde(default)]
    pub elapsed_ms: u64,
    /// Timestamp (optional)
    #[serde(default)]
    pub timestamp: String,
}

/// Test results summary.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TestResults {
    pub passed: u32,
    pub failed: u32,
    pub skipped: u32,
}

/// Contract status written by SENTINEL.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Contract {
    /// Status: AGREED or ISSUES
    pub status: String,
    /// Contract terms (definition of done)
    pub terms: Vec<ContractTerm>,
    /// Objections (if status is ISSUES)
    #[serde(skip_serializing_if = "Vec::is_empty", default)]
    pub objections: Vec<String>,
}

/// A single contract term.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ContractTerm {
    pub criterion: String,
    pub verification: String,
}

/// Segment evaluation written by SENTINEL.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SegmentEval {
    /// Segment number
    pub segment: u32,
    /// Verdict: APPROVED or CHANGES_REQUESTED
    pub verdict: String,
    /// Specific feedback items (if CHANGES_REQUESTED)
    #[serde(skip_serializing_if = "Vec::is_empty", default)]
    pub feedback: Vec<FeedbackItem>,
}

/// A specific feedback item for changes requested.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FeedbackItem {
    pub file: String,
    pub line: u32,
    pub problem: String,
    pub fix: String,
}

/// Final review written by SENTINEL.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FinalReview {
    /// Verdict: APPROVED or REJECTED
    pub verdict: String,
    /// PR description (if APPROVED)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub pr_description: Option<String>,
    /// Remaining issues (if REJECTED)
    #[serde(skip_serializing_if = "Vec::is_empty", default)]
    pub issues: Vec<String>,
}

/// File lock metadata.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FileLock {
    /// Pair that owns the lock
    pub pair: String,
    /// File path (relative to project root)
    pub file: String,
    /// When the lock was acquired
    pub acquired_at: String,
}

impl FileLock {
    /// Create a new file lock for a pair.
    pub fn new(pair: impl Into<String>, file: impl Into<String>) -> Self {
        Self {
            pair: pair.into(),
            file: file.into(),
            acquired_at: chrono::Utc::now().to_rfc3339(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_files_changed_count() {
        let json = r#"{
            "ticket_id": "T-1",
            "status": "IMPLEMENTATION_COMPLETE",
            "branch": "forge-1/T-1",
            "files_changed": 14
        }"#;

        let status: StatusJson = serde_json::from_str(json).expect("Failed to parse");
        assert_eq!(status.ticket_id, "T-1");
        assert_eq!(status.status, "IMPLEMENTATION_COMPLETE");
        match status.files_changed {
            FilesChanged::Count(n) => assert_eq!(n, 14),
            _ => panic!("Expected Count variant, got {:?}", status.files_changed),
        }
    }

    #[test]
    fn test_files_changed_list() {
        let json = r#"{
            "ticket_id": "T-2",
            "status": "PR_OPENED",
            "branch": "forge-1/T-2",
            "files_changed": ["src/main.rs", "src/lib.rs"]
        }"#;

        let status: StatusJson = serde_json::from_str(json).expect("Failed to parse");
        assert_eq!(status.ticket_id, "T-2");
        match status.files_changed {
            FilesChanged::List(v) => assert_eq!(v.len(), 2),
            _ => panic!("Expected List variant, got {:?}", status.files_changed),
        }
    }

    #[test]
    fn test_files_changed_missing() {
        let json = r#"{
            "ticket_id": "T-3",
            "status": "BLOCKED",
            "branch": "forge-1/T-3"
        }"#;

        let status: StatusJson = serde_json::from_str(json).expect("Failed to parse");
        assert_eq!(status.ticket_id, "T-3");
        assert!(status.files_changed.is_empty());
    }
}
