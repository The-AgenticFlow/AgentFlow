// binary/src/state.rs
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
pub enum TeamStatus {
    #[default]
    Idle,
    Working,
    Reviewing,
    Deploying,
    Documenting,
    Suspended,
    Finished,
    Error,
}

// ── Shared Store Keys ─────────────────────────────────────────────────────

/// The key for the list of available tickets.
pub const KEY_TICKETS: &str = "active_tickets";
/// The key for the map of FORGE worker slots.
pub const KEY_WORKER_SLOTS: &str = "worker_slots";
/// The key for branch -> PR number mapping.
pub const KEY_PR_TRACKING: &str = "pr_tracking";

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Ticket {
    pub id:          String,
    pub title:       String,
    pub body:        String,
    pub priority:    u8, // 1-5
    pub branch:      Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum WorkerStatus {
    Idle,
    Assigned { ticket_id: String },
    Working  { ticket_id: String },
    Done     { ticket_id: String, outcome: String },
    Suspended,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkerSlot {
    pub id:     String,
    pub status: WorkerStatus,
}

// ── Actions ───────────────────────────────────────────────────────────────

pub const ACTION_WORK_ASSIGNED: &str = "work_assigned";
pub const ACTION_NO_WORK:       &str = "no_work";
pub const ACTION_PR_OPENED:     &str = "pr_opened";
pub const ACTION_FAILED:        &str = "failed";
pub const ACTION_EMPTY:         &str = "empty";
