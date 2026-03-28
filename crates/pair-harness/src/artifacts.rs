/// Artifact Types for AgentFlow Pair Harness
///
/// Defines the schema for artifacts exchanged between FORGE, SENTINEL, and NEXUS.
///
/// # Validation: C5-01 Schema Compliance
/// STATUS.json struct requires status, pair, ticket_id
use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::path::Path;

/// Possible status values for a pair
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum Status {
    /// PR successfully opened
    PrOpened,
    /// Task blocked (needs human intervention)
    Blocked,
    /// Context window exhausted, max resets reached
    FuelExhausted,
}

/// Blocker information when status is BLOCKED
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Blocker {
    #[serde(rename = "type")]
    pub blocker_type: String,
    pub description: String,
    pub nexus_action: String,
}

/// Test results summary
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TestResults {
    pub passed: u32,
    pub failed: u32,
    pub skipped: u32,
}

/// Terminal status artifact written by FORGE
///
/// # Validation: C5-01 Schema Compliance
/// Requires status, pair, ticket_id fields
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StatusJson {
    /// Current status
    pub status: Status,

    /// Pair identifier
    pub pair: String,

    /// Ticket identifier
    pub ticket_id: String,

    /// PR URL (if PR_OPENED)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub pr_url: Option<String>,

    /// PR number (if PR_OPENED)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub pr_number: Option<u32>,

    /// Branch name
    #[serde(skip_serializing_if = "Option::is_none")]
    pub branch: Option<String>,

    /// Files changed
    pub files_changed: Vec<String>,

    /// Test results
    #[serde(skip_serializing_if = "Option::is_none")]
    pub test_results: Option<TestResults>,

    /// Segments completed
    #[serde(skip_serializing_if = "Option::is_none")]
    pub segments_completed: Option<u32>,

    /// Number of context resets
    #[serde(skip_serializing_if = "Option::is_none")]
    pub context_resets: Option<u32>,

    /// Whether SENTINEL approved
    #[serde(skip_serializing_if = "Option::is_none")]
    pub sentinel_approved: Option<bool>,

    /// Blockers (if BLOCKED)
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub blockers: Vec<Blocker>,

    /// Elapsed time in milliseconds
    #[serde(skip_serializing_if = "Option::is_none")]
    pub elapsed_ms: Option<u64>,

    /// Timestamp
    pub timestamp: String,
}

impl StatusJson {
    /// Reads STATUS.json from the shared directory
    pub async fn read_from_file(path: &Path) -> Result<Self> {
        let content = tokio::fs::read_to_string(path)
            .await
            .context("Failed to read STATUS.json")?;

        // Validation: C5-01 - Deserialization fails if required fields are missing
        let status: StatusJson = serde_json::from_str(&content)
            .context("Failed to parse STATUS.json - missing required fields")?;

        Ok(status)
    }

    /// Writes STATUS.json to the shared directory
    pub async fn write_to_file(&self, path: &Path) -> Result<()> {
        let content =
            serde_json::to_string_pretty(self).context("Failed to serialize STATUS.json")?;

        tokio::fs::write(path, content)
            .await
            .context("Failed to write STATUS.json")?;

        Ok(())
    }

    /// Validates that all required fields are present
    ///
    /// # Validation: C5-01 Schema Compliance
    pub fn validate(&self) -> Result<()> {
        if self.pair.is_empty() {
            anyhow::bail!("STATUS.json validation failed: 'pair' is empty");
        }

        if self.ticket_id.is_empty() {
            anyhow::bail!("STATUS.json validation failed: 'ticket_id' is empty");
        }

        // If status is PR_OPENED, pr_url must be present
        if self.status == Status::PrOpened && self.pr_url.is_none() {
            anyhow::bail!("STATUS.json validation failed: 'pr_url' required for PR_OPENED");
        }

        // If status is BLOCKED, blockers must not be empty
        if self.status == Status::Blocked && self.blockers.is_empty() {
            anyhow::bail!("STATUS.json validation failed: 'blockers' required for BLOCKED");
        }

        Ok(())
    }
}

/// Ticket information
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Ticket {
    pub id: String,
    pub title: String,
    pub description: String,
    pub acceptance_criteria: Vec<String>,
    pub touched_files: Vec<String>,
    pub labels: Vec<String>,
}

impl Ticket {
    /// Writes TICKET.md to the shared directory
    pub async fn write_ticket_md(&self, path: &Path) -> Result<()> {
        let mut content = format!("# Ticket: {}\n\n", self.id);
        content.push_str(&format!("## Title\n{}\n\n", self.title));
        content.push_str(&format!("## Description\n{}\n\n", self.description));

        content.push_str("## Acceptance Criteria\n");
        for criterion in &self.acceptance_criteria {
            content.push_str(&format!("- [ ] {}\n", criterion));
        }
        content.push_str("\n");

        if !self.touched_files.is_empty() {
            content.push_str("## Files to Touch\n");
            for file in &self.touched_files {
                content.push_str(&format!("- {}\n", file));
            }
            content.push_str("\n");
        }

        if !self.labels.is_empty() {
            content.push_str(&format!("## Labels\n{}\n", self.labels.join(", ")));
        }

        tokio::fs::write(path, content)
            .await
            .context("Failed to write TICKET.md")?;

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_status_json_validation() {
        // Valid PR_OPENED status
        let valid = StatusJson {
            status: Status::PrOpened,
            pair: "pair-1".to_string(),
            ticket_id: "T-42".to_string(),
            pr_url: Some("https://github.com/org/repo/pull/123".to_string()),
            pr_number: Some(123),
            branch: Some("forge-1/T-42".to_string()),
            files_changed: vec!["src/main.rs".to_string()],
            test_results: None,
            segments_completed: Some(3),
            context_resets: Some(1),
            sentinel_approved: Some(true),
            blockers: vec![],
            elapsed_ms: Some(5420000),
            timestamp: "2025-03-24T14:22:00Z".to_string(),
        };
        assert!(valid.validate().is_ok());

        // Invalid: PR_OPENED without pr_url
        let invalid = StatusJson {
            status: Status::PrOpened,
            pair: "pair-1".to_string(),
            ticket_id: "T-42".to_string(),
            pr_url: None,
            pr_number: None,
            branch: None,
            files_changed: vec![],
            test_results: None,
            segments_completed: None,
            context_resets: None,
            sentinel_approved: None,
            blockers: vec![],
            elapsed_ms: None,
            timestamp: "2025-03-24T14:22:00Z".to_string(),
        };
        assert!(invalid.validate().is_err());

        // Invalid: BLOCKED without blockers
        let invalid_blocked = StatusJson {
            status: Status::Blocked,
            pair: "pair-1".to_string(),
            ticket_id: "T-42".to_string(),
            pr_url: None,
            pr_number: None,
            branch: None,
            files_changed: vec![],
            test_results: None,
            segments_completed: None,
            context_resets: None,
            sentinel_approved: None,
            blockers: vec![],
            elapsed_ms: None,
            timestamp: "2025-03-24T14:22:00Z".to_string(),
        };
        assert!(invalid_blocked.validate().is_err());
    }

    #[test]
    fn test_status_serialization() {
        let status = StatusJson {
            status: Status::PrOpened,
            pair: "pair-1".to_string(),
            ticket_id: "T-42".to_string(),
            pr_url: Some("https://github.com/org/repo/pull/123".to_string()),
            pr_number: Some(123),
            branch: Some("forge-1/T-42".to_string()),
            files_changed: vec!["src/main.rs".to_string()],
            test_results: Some(TestResults {
                passed: 89,
                failed: 0,
                skipped: 0,
            }),
            segments_completed: Some(3),
            context_resets: Some(1),
            sentinel_approved: Some(true),
            blockers: vec![],
            elapsed_ms: Some(5420000),
            timestamp: "2025-03-24T14:22:00Z".to_string(),
        };

        let json = serde_json::to_string(&status).unwrap();
        assert!(json.contains("PR_OPENED"));
        assert!(json.contains("pair-1"));
        assert!(json.contains("T-42"));
    }
}
