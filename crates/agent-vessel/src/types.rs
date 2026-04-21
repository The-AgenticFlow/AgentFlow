// crates/agent-vessel/src/types.rs
//
// VESSEL-specific types and configuration.

use pocketflow_core::{CiPollConfig, MergeMethod};
use serde::{Deserialize, Serialize};

/// CI readiness state — mirrors the nexus CiReadiness for store deserialization.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CiReadiness {
    Ready,
    Missing,
    SetupInProgress,
}

/// Configuration for the VESSEL agent.
#[derive(Debug, Clone, Default)]
pub struct VesselConfig {
    pub ci_poll: CiPollConfig,
    pub merge_method: MergeMethod,
    pub github_token: String,
}

impl VesselConfig {
    pub fn from_env() -> Self {
        let github_token = std::env::var("GITHUB_PERSONAL_ACCESS_TOKEN")
            .expect("GITHUB_PERSONAL_ACCESS_TOKEN must be set");

        Self {
            ci_poll: CiPollConfig::default(),
            merge_method: MergeMethod::default(),
            github_token,
        }
    }
}

/// Result of the VESSEL workflow for a single PR.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum VesselOutcome {
    /// Successfully merged and optionally deployed
    Merged {
        ticket_id: String,
        pr_number: u64,
        sha: String,
    },
    /// CI failed, did not merge
    CiFailed {
        ticket_id: Option<String>,
        pr_number: u64,
        reason: String,
    },
    /// CI passed but merge failed (conflict, etc.)
    MergeBlocked {
        ticket_id: Option<String>,
        pr_number: u64,
        reason: String,
    },
    /// CI polling timed out
    CiTimeout {
        ticket_id: Option<String>,
        pr_number: u64,
    },
    /// No CI workflows configured — merged without CI validation
    CiMissing {
        ticket_id: Option<String>,
        pr_number: u64,
    },
    /// Merge conflicts detected — could not be auto-resolved
    Conflicts {
        ticket_id: Option<String>,
        pr_number: u64,
        conflicted_files: Vec<String>,
    },
}

impl VesselOutcome {
    pub fn ticket_id(&self) -> Option<&str> {
        match self {
            VesselOutcome::Merged { ticket_id, .. } => Some(ticket_id),
            VesselOutcome::CiFailed { ticket_id, .. } => ticket_id.as_deref(),
            VesselOutcome::MergeBlocked { ticket_id, .. } => ticket_id.as_deref(),
            VesselOutcome::CiTimeout { ticket_id, .. } => ticket_id.as_deref(),
            VesselOutcome::CiMissing { ticket_id, .. } => ticket_id.as_deref(),
            VesselOutcome::Conflicts { ticket_id, .. } => ticket_id.as_deref(),
        }
    }

    pub fn pr_number(&self) -> u64 {
        match self {
            VesselOutcome::Merged { pr_number, .. } => *pr_number,
            VesselOutcome::CiFailed { pr_number, .. } => *pr_number,
            VesselOutcome::MergeBlocked { pr_number, .. } => *pr_number,
            VesselOutcome::CiTimeout { pr_number, .. } => *pr_number,
            VesselOutcome::CiMissing { pr_number, .. } => *pr_number,
            VesselOutcome::Conflicts { pr_number, .. } => *pr_number,
        }
    }
}
