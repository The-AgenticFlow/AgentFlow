// crates/agent-lore/src/retrospective.rs
use anyhow::Result;
use chrono::Utc;
use pocketflow_core::SharedStore;
use std::path::PathBuf;
use tracing::info;

use crate::types::{MergedTicketInfo, SprintMetrics};

pub struct RetrospectiveGenerator {
    output_dir: PathBuf,
}

impl RetrospectiveGenerator {
    pub fn new(docs_dir: PathBuf) -> Self {
        Self {
            output_dir: docs_dir.join("retrospectives"),
        }
    }

    pub async fn generate(
        &self,
        sprint_id: &str,
        tickets: &[MergedTicketInfo],
        metrics: Option<&SprintMetrics>,
    ) -> Result<PathBuf> {
        if !self.output_dir.exists() {
            tokio::fs::create_dir_all(&self.output_dir).await?;
        }

        let default_metrics = SprintMetrics {
            sprint_id: sprint_id.to_string(),
            tickets_completed: tickets.len() as u32,
            tickets_carried_over: 0,
            blockers: Vec::new(),
            highlights: Vec::new(),
            lessons_learned: Vec::new(),
        };
        let metrics = metrics.unwrap_or(&default_metrics);

        let content = self.format_retrospective(sprint_id, tickets, metrics);
        let filename = format!("{}-retrospective.md", sprint_id);
        let path = self.output_dir.join(&filename);

        tokio::fs::write(&path, &content).await?;
        info!(path = %path.display(), sprint_id, "Retrospective generated");

        Ok(path)
    }

    fn format_retrospective(
        &self,
        sprint_id: &str,
        tickets: &[MergedTicketInfo],
        metrics: &SprintMetrics,
    ) -> String {
        let date = Utc::now().format("%Y-%m-%d").to_string();
        let tickets_list = if tickets.is_empty() {
            "- No tickets completed this sprint".to_string()
        } else {
            tickets
                .iter()
                .map(|t| format!("- {} (#{})", t.pr_title, t.pr_number))
                .collect::<Vec<_>>()
                .join("\n")
        };

        let blockers_list = if metrics.blockers.is_empty() {
            "- No significant blockers".to_string()
        } else {
            metrics
                .blockers
                .iter()
                .map(|b| format!("- {}", b))
                .collect::<Vec<_>>()
                .join("\n")
        };

        let highlights_list = if metrics.highlights.is_empty() {
            tickets
                .iter()
                .take(3)
                .map(|t| format!("- {}", t.pr_title))
                .collect::<Vec<_>>()
                .join("\n")
        } else {
            metrics
                .highlights
                .iter()
                .map(|h| format!("- {}", h))
                .collect::<Vec<_>>()
                .join("\n")
        };

        let lessons_list = if metrics.lessons_learned.is_empty() {
            "- Document lessons in next retrospective".to_string()
        } else {
            metrics
                .lessons_learned
                .iter()
                .map(|l| format!("- {}", l))
                .collect::<Vec<_>>()
                .join("\n")
        };

        format!(
            r#"# Sprint {} Retrospective

**Date**: {}

## Summary

- **Tickets Completed**: {}
- **Tickets Carried Over**: {}

## Completed Work

{}

## Highlights

{}

## Blockers & Challenges

{}

## Lessons Learned

{}

## Action Items

- [ ] Review process improvements for next sprint
- [ ] Address any carried-over tickets early
"#,
            sprint_id,
            date,
            metrics.tickets_completed,
            metrics.tickets_carried_over,
            tickets_list,
            highlights_list,
            blockers_list,
            lessons_list
        )
    }

    pub async fn read_sprint_history(store: &SharedStore) -> Vec<MergedTicketInfo> {
        let events = store.get_events_since(0).await;
        events
            .iter()
            .filter(|e| e.event_type == "ticket_merged")
            .filter_map(|e| {
                let ticket_id = e.payload["ticket_id"].as_str()?.to_string();
                let pr_number = e.payload["pr_number"].as_u64()?;
                let sha = e.payload["sha"].as_str().unwrap_or("").to_string();

                Some(MergedTicketInfo {
                    ticket_id,
                    pr_number,
                    pr_title: format!("PR #{}", pr_number),
                    pr_body: None,
                    sha,
                    merged_at: chrono::Utc::now().to_rfc3339(),
                    changes: Vec::new(),
                })
            })
            .collect()
    }

    pub async fn list_existing(&self) -> Result<Vec<PathBuf>> {
        if !self.output_dir.exists() {
            return Ok(Vec::new());
        }

        let mut entries = Vec::new();
        let mut dir = tokio::fs::read_dir(&self.output_dir).await?;
        while let Some(entry) = dir.next_entry().await? {
            let path = entry.path();
            if path.extension().map(|e| e == "md").unwrap_or(false) {
                entries.push(path);
            }
        }
        entries.sort();
        Ok(entries)
    }
}
