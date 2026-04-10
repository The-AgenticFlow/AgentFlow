// crates/config/src/state.rs
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Ticket {
    pub id: String,
    pub title: String,
    pub body: String,
    pub priority: u32,
    pub branch: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkerSlot {
    pub id: String,
    pub status: WorkerStatus,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum WorkerStatus {
    Idle,
    Assigned {
        ticket_id: String,
        issue_url: Option<String>,
    },
    Working {
        ticket_id: String,
        issue_url: Option<String>,
    },
    Done {
        ticket_id: String,
        outcome: String,
    },
    Suspended {
        ticket_id: String,
        reason: String,
        issue_url: Option<String>,
    },
}

pub const KEY_TICKETS: &str = "tickets";
pub const KEY_WORKER_SLOTS: &str = "worker_slots";
pub const KEY_OPEN_PRS: &str = "open_prs";
pub const KEY_COMMAND_GATE: &str = "command_gate";

pub const ACTION_WORK_ASSIGNED: &str = "work_assigned";
pub const ACTION_PR_OPENED: &str = "pr_opened";
pub const ACTION_NO_WORK: &str = "no_work";
pub const ACTION_EMPTY: &str = "empty";
pub const ACTION_FAILED: &str = "failed";
