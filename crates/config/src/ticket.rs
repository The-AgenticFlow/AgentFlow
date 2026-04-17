use serde::{Deserialize, Serialize};
use serde_json::Value;

/// Central ticket model used by orchestration nodes.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Ticket {
    pub id: String,
    pub title: String,
    pub body: String,
    pub priority: u32,
    pub branch: Option<String>,
    /// Status of the ticket lifecycle (open, assigned, merged, etc.)
    #[serde(default)]
    pub status: TicketStatus,
    /// Optional link to original issue or PR
    #[serde(default)]
    pub issue_url: Option<String>,
    /// Retry attempts
    #[serde(default)]
    pub attempts: u32,
    /// Explicit dependency list. Each entry is a ticket id like "T-001".
    #[serde(default)]
    pub depends_on: Vec<String>,
}

impl Ticket {
    pub const MAX_ATTEMPTS: u32 = 3;

    /// A ticket is assignable if it's Open or Failed with remaining attempts.
    /// Dependency checks are performed by the orchestrator (NEXUS).
    pub fn is_assignable(&self) -> bool {
        match &self.status {
            TicketStatus::Open => true,
            TicketStatus::Failed { attempts, .. } => *attempts < Self::MAX_ATTEMPTS,
            _ => false,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum TicketStatus {
    #[serde(rename = "open")]
    Open,
    #[serde(rename = "assigned")]
    Assigned { worker_id: String },
    #[serde(rename = "in_progress")]
    InProgress { worker_id: String },
    #[serde(rename = "failed")]
    Failed {
        worker_id: String,
        reason: String,
        attempts: u32,
    },
    #[serde(rename = "completed")]
    Completed { worker_id: String, outcome: String },
    #[serde(rename = "exhausted")]
    Exhausted { worker_id: String, attempts: u32 },
    /// Ticket cannot be assigned because it depends on other tickets.
    #[serde(rename = "waiting_on_dependency")]
    WaitingOnDependency { depends_on: Vec<String> },
    /// Ticket has been merged into main by VESSEL.
    #[serde(rename = "merged")]
    Merged {
        worker_id: String,
        pr_url: Option<String>,
    },
}

impl Default for TicketStatus {
    fn default() -> Self {
        TicketStatus::Open
    }
}

// Convenience helpers for JSON-typed fields if needed later.
impl TicketStatus {
    pub fn to_value(&self) -> Value {
        serde_json::to_value(self).unwrap_or(Value::Null)
    }
}
