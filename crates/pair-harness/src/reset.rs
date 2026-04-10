// crates/pair-harness/src/reset.rs
//! Context reset mechanism for long-running tasks.
//!
//! When FORGE's context window approaches its limit, it writes a HANDOFF.md
//! and exits. The harness spawns a fresh FORGE that reads the handoff and
//! continues from the exact next step.

use anyhow::{Context, Result};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::{Path, PathBuf};
use tracing::{debug, info, warn};

/// Manages context resets and handoff synthesis.
pub struct ResetManager {
    /// Path to the shared directory
    shared: PathBuf,
    /// Maximum number of resets allowed
    max_resets: u32,
    /// Current reset count
    reset_count: u32,
}

impl ResetManager {
    /// Create a new reset manager.
    pub fn new(shared: PathBuf, max_resets: u32) -> Self {
        Self {
            shared,
            max_resets,
            reset_count: 0,
        }
    }

    /// Check if a handoff exists.
    pub fn has_handoff(&self) -> bool {
        self.shared.join("HANDOFF.md").exists()
    }

    /// Read the handoff file.
    pub fn read_handoff(&self) -> Result<Handoff> {
        let path = self.shared.join("HANDOFF.md");
        let content = fs::read_to_string(&path).context("Failed to read HANDOFF.md")?;

        Ok(Handoff::parse(&content))
    }

    /// Check if we can do another reset.
    pub fn can_reset(&self) -> bool {
        self.reset_count < self.max_resets
    }

    /// Increment the reset count.
    pub fn increment_reset(&mut self) -> u32 {
        self.reset_count += 1;
        self.reset_count
    }

    /// Get the current reset count.
    pub fn reset_count(&self) -> u32 {
        self.reset_count
    }

    /// Synthesize a handoff from WORKLOG.md when FORGE exits uncleanly.
    pub async fn synthesize_handoff(&self) -> Result<()> {
        info!("Synthesizing handoff from WORKLOG.md");

        let worklog_path = self.shared.join("WORKLOG.md");
        if !worklog_path.exists() {
            warn!("No WORKLOG.md to synthesize handoff from");
            return Ok(());
        }

        let worklog = fs::read_to_string(&worklog_path)?;
        let handoff = self.synthesize_from_worklog(&worklog)?;

        // Write the synthesized handoff
        let handoff_path = self.shared.join("HANDOFF.md");
        fs::write(&handoff_path, handoff.to_string())?;

        info!("Synthesized handoff written");
        Ok(())
    }

    /// Parse WORKLOG.md and create a synthesized handoff.
    fn synthesize_from_worklog(&self, worklog: &str) -> Result<Handoff> {
        let mut completed_segments = Vec::new();
        let mut current_segment = None;
        let mut decisions = Vec::new();
        let mut files_changed = Vec::new();

        // Simple parsing of WORKLOG.md
        for line in worklog.lines() {
            // Look for segment headers
            if line.starts_with("## Segment") {
                if let Some(seg) = current_segment.take() {
                    completed_segments.push(seg);
                }
                // Parse segment number
                let seg_num = line
                    .split_whitespace()
                    .nth(2)
                    .and_then(|s| s.parse::<u32>().ok())
                    .unwrap_or(0);
                current_segment = Some(SegmentSummary {
                    number: seg_num,
                    status: "UNKNOWN".to_string(),
                    files: Vec::new(),
                });
            }

            // Look for files changed
            if line.starts_with("  - ") || line.starts_with("- ") {
                let file = line
                    .trim_start_matches("  - ")
                    .trim_start_matches("- ")
                    .to_string();
                if !file.is_empty() && !file.starts_with("Status:") {
                    files_changed.push(file.clone());
                    if let Some(ref mut seg) = current_segment {
                        seg.files.push(file);
                    }
                }
            }

            // Look for decisions
            if line.starts_with("  - Decision:") || line.starts_with("- Decision:") {
                let decision = line
                    .trim_start_matches("  - Decision:")
                    .trim_start_matches("- Decision:")
                    .trim()
                    .to_string();
                decisions.push(decision);
            }

            // Look for status
            if line.contains("SENTINEL APPROVED") {
                if let Some(ref mut seg) = current_segment {
                    seg.status = "APPROVED".to_string();
                }
            }
        }

        // Add the last segment
        if let Some(seg) = current_segment {
            completed_segments.push(seg);
        }

        // Determine next step
        let next_segment = completed_segments.len() as u32 + 1;
        let next_step = format!(
            "Continue with segment {} (check PLAN.md for details)",
            next_segment
        );

        Ok(Handoff {
            ticket_id: "UNKNOWN".to_string(),
            pair_id: "UNKNOWN".to_string(),
            completed_segments,
            in_progress: None,
            decisions,
            files_changed,
            next_step,
            timestamp: Utc::now(),
        })
    }

    /// Clear the handoff file after it's been read.
    pub fn clear_handoff(&self) -> Result<()> {
        let path = self.shared.join("HANDOFF.md");
        if path.exists() {
            fs::remove_file(&path).context("Failed to remove HANDOFF.md")?;
            debug!("HANDOFF.md cleared");
        }
        Ok(())
    }
}

/// Summary of a completed segment.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SegmentSummary {
    pub number: u32,
    pub status: String,
    pub files: Vec<String>,
}

/// Handoff document for context reset.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Handoff {
    /// Ticket ID
    pub ticket_id: String,
    /// Pair ID
    pub pair_id: String,
    /// Completed segments
    pub completed_segments: Vec<SegmentSummary>,
    /// In-progress segment (if any)
    pub in_progress: Option<SegmentSummary>,
    /// Decisions made so far
    pub decisions: Vec<String>,
    /// Files changed so far
    pub files_changed: Vec<String>,
    /// Exact next step
    pub next_step: String,
    /// Timestamp
    pub timestamp: DateTime<Utc>,
}

impl Handoff {
    /// Parse a HANDOFF.md file.
    pub fn parse(content: &str) -> Self {
        let mut handoff = Handoff {
            ticket_id: String::new(),
            pair_id: String::new(),
            completed_segments: Vec::new(),
            in_progress: None,
            decisions: Vec::new(),
            files_changed: Vec::new(),
            next_step: String::new(),
            timestamp: Utc::now(),
        };

        let mut current_section = String::new();

        for line in content.lines() {
            // Section headers
            if line.starts_with("## ") {
                current_section = line.trim_start_matches("## ").to_string();
                continue;
            }

            // Parse based on section
            match current_section.as_str() {
                "Ticket" | "Ticket ID" => {
                    handoff.ticket_id = line.trim().to_string();
                }
                "Pair" | "Pair ID" => {
                    handoff.pair_id = line.trim().to_string();
                }
                "Completed Segments" => {
                    if line.starts_with("- Segment") {
                        // Parse segment summary
                        let num = line
                            .split_whitespace()
                            .nth(1)
                            .and_then(|s| s.parse::<u32>().ok())
                            .unwrap_or(0);
                        handoff.completed_segments.push(SegmentSummary {
                            number: num,
                            status: "APPROVED".to_string(),
                            files: Vec::new(),
                        });
                    }
                }
                "Decisions" => {
                    if line.starts_with("- ") {
                        handoff
                            .decisions
                            .push(line.trim_start_matches("- ").to_string());
                    }
                }
                "Files Changed" => {
                    if line.starts_with("- ") {
                        handoff
                            .files_changed
                            .push(line.trim_start_matches("- ").to_string());
                    }
                }
                "Exact next step" => {
                    if !line.is_empty() && !line.starts_with("#") {
                        handoff.next_step.push_str(line);
                        handoff.next_step.push('\n');
                    }
                }
                _ => {}
            }
        }

        handoff.next_step = handoff.next_step.trim().to_string();
        handoff
    }

    /// Convert to markdown format.
    pub fn to_string(&self) -> String {
        let mut md = String::new();

        md.push_str("# HANDOFF\n\n");
        md.push_str(&format!("**Ticket:** {}\n\n", self.ticket_id));
        md.push_str(&format!("**Pair:** {}\n\n", self.pair_id));
        md.push_str(&format!(
            "**Timestamp:** {}\n\n",
            self.timestamp.to_rfc3339()
        ));

        md.push_str("## Completed Segments\n\n");
        for seg in &self.completed_segments {
            md.push_str(&format!("- Segment {}: {}\n", seg.number, seg.status));
            for file in &seg.files {
                md.push_str(&format!("  - {}\n", file));
            }
        }

        if let Some(ref in_prog) = self.in_progress {
            md.push_str("\n## In Progress\n\n");
            md.push_str(&format!(
                "- Segment {}: {}\n",
                in_prog.number, in_prog.status
            ));
        }

        md.push_str("\n## Decisions\n\n");
        for decision in &self.decisions {
            md.push_str(&format!("- {}\n", decision));
        }

        md.push_str("\n## Files Changed\n\n");
        for file in &self.files_changed {
            md.push_str(&format!("- {}\n", file));
        }

        md.push_str("\n## Exact next step\n\n");
        md.push_str(&self.next_step);
        md.push_str("\n");

        md
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_handoff_parse() {
        let content = r#"# HANDOFF

**Ticket:** T-42

**Pair:** pair-1

## Completed Segments

- Segment 1: APPROVED
  - src/auth/login.ts
  - tests/auth/login.test.ts

## Decisions

- Used httpOnly cookies for refresh tokens

## Files Changed

- src/auth/login.ts
- tests/auth/login.test.ts

## Exact next step

Continue with segment 2: Implement JWT token generation.
"#;

        let handoff = Handoff::parse(content);
        assert_eq!(handoff.ticket_id, "T-42");
        assert_eq!(handoff.pair_id, "pair-1");
        assert_eq!(handoff.completed_segments.len(), 1);
        assert_eq!(handoff.decisions.len(), 1);
        assert!(handoff.next_step.contains("segment 2"));
    }

    #[test]
    fn test_handoff_roundtrip() {
        let handoff = Handoff {
            ticket_id: "T-42".to_string(),
            pair_id: "pair-1".to_string(),
            completed_segments: vec![SegmentSummary {
                number: 1,
                status: "APPROVED".to_string(),
                files: vec!["src/auth.ts".to_string()],
            }],
            in_progress: None,
            decisions: vec!["Use httpOnly cookies".to_string()],
            files_changed: vec!["src/auth.ts".to_string()],
            next_step: "Continue with segment 2".to_string(),
            timestamp: Utc::now(),
        };

        let md = handoff.to_string();
        let parsed = Handoff::parse(&md);

        assert_eq!(parsed.ticket_id, handoff.ticket_id);
        assert_eq!(parsed.pair_id, handoff.pair_id);
        assert_eq!(
            parsed.completed_segments.len(),
            handoff.completed_segments.len()
        );
    }
}
