// crates/agent-lore/src/adr.rs
use anyhow::Result;
use std::path::PathBuf;
use tracing::info;

use crate::types::ArchitecturalDecision;

pub struct AdrGenerator {
    adr_dir: PathBuf,
}

impl AdrGenerator {
    pub fn new(adr_dir: PathBuf) -> Self {
        Self { adr_dir }
    }

    pub async fn generate(&self, decision: &ArchitecturalDecision) -> Result<PathBuf> {
        if !self.adr_dir.exists() {
            tokio::fs::create_dir_all(&self.adr_dir).await?;
            info!(path = ?self.adr_dir, "Created ADR directory");
        }

        let filename = Self::generate_filename(&decision.date, &decision.title);
        let path = self.adr_dir.join(&filename);
        let adr_id = filename.trim_end_matches(".md").to_string();

        let content = Self::format_adr(decision, &adr_id);

        tokio::fs::write(&path, &content).await?;
        info!(path = %path.display(), adr_id, "ADR written");

        Ok(path)
    }

    pub async fn list_existing(&self) -> Result<Vec<PathBuf>> {
        if !self.adr_dir.exists() {
            return Ok(Vec::new());
        }

        let mut entries = Vec::new();
        let mut dir = tokio::fs::read_dir(&self.adr_dir).await?;
        while let Some(entry) = dir.next_entry().await? {
            let path = entry.path();
            if path.extension().map(|e| e == "md").unwrap_or(false) {
                entries.push(path);
            }
        }
        entries.sort();
        Ok(entries)
    }

    pub fn generate_filename(date: &str, title: &str) -> String {
        let slug = Self::slugify(title);
        format!("{}-{}.md", date, slug)
    }

    fn slugify(title: &str) -> String {
        title
            .to_lowercase()
            .chars()
            .map(|c| {
                if c.is_alphanumeric() {
                    c
                } else if c.is_whitespace() || c == '-' || c == '_' || c == ':' {
                    '-'
                } else {
                    ' '
                }
            })
            .collect::<String>()
            .split('-')
            .filter(|s| !s.is_empty())
            .collect::<Vec<_>>()
            .join("-")
    }

    fn format_adr(decision: &ArchitecturalDecision, _adr_id: &str) -> String {
        let pr_reference = decision
            .pr_number
            .map(|n| format!("- PR: #{}", n))
            .unwrap_or_default();

        format!(
            r#"# {}

## Status
Accepted

## Context
{}

## Decision
{}

## Consequences
{}

## References
- Ticket: {}
{}
- Date: {}
"#,
            decision.title,
            decision.context,
            decision.decision,
            decision.consequences,
            decision.ticket_id,
            pr_reference,
            decision.date
        )
    }

    pub async fn adr_exists_for_ticket(&self, ticket_id: &str) -> bool {
        if !self.adr_dir.exists() {
            return false;
        }

        let Ok(mut dir) = tokio::fs::read_dir(&self.adr_dir).await else {
            return false;
        };

        while let Ok(Some(entry)) = dir.next_entry().await {
            let path = entry.path();
            if path.extension().map(|e| e == "md").unwrap_or(false) {
                if let Ok(content) = tokio::fs::read_to_string(&path).await {
                    if content.contains(&format!("- Ticket: {}", ticket_id)) {
                        return true;
                    }
                }
            }
        }
        false
    }

    pub async fn read_adr(&self, adr_id: &str) -> Result<Option<String>> {
        let path = self.adr_dir.join(format!("{}.md", adr_id));
        if path.exists() {
            let content = tokio::fs::read_to_string(&path).await?;
            Ok(Some(content))
        } else {
            let mut dir = tokio::fs::read_dir(&self.adr_dir).await?;
            while let Some(entry) = dir.next_entry().await? {
                let path = entry.path();
                if let Some(filename) = path.file_name().and_then(|n| n.to_str()) {
                    if filename.starts_with(adr_id) || filename.contains(adr_id) {
                        let content = tokio::fs::read_to_string(&path).await?;
                        return Ok(Some(content));
                    }
                }
            }
            Ok(None)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_slugify() {
        assert_eq!(AdrGenerator::slugify("Add OAuth2 Support"), "add-oauth2-support");
        assert_eq!(AdrGenerator::slugify("Use PostgreSQL for Storage"), "use-postgresql-for-storage");
        assert_eq!(AdrGenerator::slugify("Fix: API Rate Limiting"), "fix-api-rate-limiting");
    }

    #[test]
    fn test_generate_filename() {
        let filename = AdrGenerator::generate_filename("2024-01-15", "Add OAuth2 Support");
        assert_eq!(filename, "2024-01-15-add-oauth2-support.md");
    }
}
