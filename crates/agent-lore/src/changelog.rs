// crates/agent-lore/src/changelog.rs
use anyhow::Result;
use std::path::PathBuf;
use tracing::info;

use crate::types::{ChangeCategory, MergedTicketInfo};

pub struct ChangelogManager {
    changelog_path: PathBuf,
}

impl ChangelogManager {
    pub fn new(docs_dir: PathBuf) -> Self {
        Self {
            changelog_path: docs_dir.join("CHANGELOG.md"),
        }
    }

    pub async fn add_entry(
        &self,
        category: ChangeCategory,
        entry: &str,
        pr_number: u64,
    ) -> Result<()> {
        let content = self.read_current().await?;

        let entry_line = self.format_entry(entry, pr_number);

        let updated = self.insert_entry(&content, category, &entry_line)?;

        tokio::fs::write(&self.changelog_path, &updated).await?;
        info!(
            category = category.as_str(),
            pr_number, "Changelog entry added"
        );

        Ok(())
    }

    pub async fn add_entry_for_ticket(&self, ticket_info: &MergedTicketInfo) -> Result<()> {
        let category = ChangeCategory::from_pr_title(&ticket_info.pr_title);
        let entry = ticket_info
            .pr_title
            .strip_prefix(&format!("[{}] ", ticket_info.ticket_id))
            .unwrap_or(&ticket_info.pr_title);

        self.add_entry(category, entry, ticket_info.pr_number).await
    }

    pub async fn read_current(&self) -> Result<String> {
        if self.changelog_path.exists() {
            let content = tokio::fs::read_to_string(&self.changelog_path).await?;
            Ok(content)
        } else {
            Ok(self.create_initial_changelog())
        }
    }

    fn create_initial_changelog(&self) -> String {
        r#"# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Added

### Changed

### Deprecated

### Removed

### Fixed

### Security
"#
        .to_string()
    }

    fn format_entry(&self, entry: &str, pr_number: u64) -> String {
        format!("- {} (#{})", entry, pr_number)
    }

    fn insert_entry(
        &self,
        content: &str,
        category: ChangeCategory,
        entry_line: &str,
    ) -> Result<String> {
        let section_header = format!("### {}", category.as_str());

        let lines: Vec<&str> = content.lines().collect();
        let mut result = String::new();

        let mut in_unreleased = false;
        let mut in_section = false;
        let mut inserted = false;

        for (i, line) in lines.iter().enumerate() {
            if line.starts_with("## [Unreleased]") {
                in_unreleased = true;
            } else if line.starts_with("## ") && in_unreleased {
                in_unreleased = false;
            }

            if in_unreleased && line == &section_header {
                in_section = true;
                result.push_str(line);
                result.push('\n');
                continue;
            }

            if in_section && line.starts_with("### ") && line != &section_header {
                in_section = false;
            }

            if in_section && !inserted {
                result.push_str(entry_line);
                result.push('\n');
                inserted = true;
            }

            result.push_str(line);
            if i < lines.len() - 1 || !line.is_empty() {
                result.push('\n');
            }
        }

        if !inserted {
            if !result.contains("## [Unreleased]") {
                let unreleased_section = self.create_unreleased_section(category, entry_line);
                let insert_pos = result.find("# Changelog\n\n").map(|p| p + 13).unwrap_or(0);
                result.insert_str(insert_pos, &unreleased_section);
            } else {
                let section = format!("### {}\n{}\n", category.as_str(), entry_line);
                let unreleased_end = result
                    .find("## [Unreleased]")
                    .and_then(|p| {
                        result[p..]
                            .find("\n\n## ")
                            .map(|offset| p + offset)
                            .or(Some(result.len()))
                    })
                    .unwrap_or(result.len());

                result.insert_str(unreleased_end, &section);
            }
        }

        Ok(result)
    }

    fn create_unreleased_section(&self, category: ChangeCategory, entry_line: &str) -> String {
        format!(
            r#"## [Unreleased]

### {}
{}

"#,
            category.as_str(),
            entry_line
        )
    }

    pub fn categorize_from_pr(&self, pr_title: &str, pr_body: Option<&str>) -> ChangeCategory {
        let mut category = ChangeCategory::from_pr_title(pr_title);

        if let Some(body) = pr_body {
            let lower = body.to_lowercase();
            if lower.contains("breaking change") || lower.contains("## breaking") {
                category = ChangeCategory::Changed;
            }
            if lower.contains("security")
                || lower.contains("vulnerability")
                || lower.contains("cve-")
            {
                category = ChangeCategory::Security;
            }
        }

        category
    }

    pub async fn ensure_changelog_exists(&self) -> Result<()> {
        if !self.changelog_path.exists() {
            if let Some(parent) = self.changelog_path.parent() {
                tokio::fs::create_dir_all(parent).await?;
            }
            let content = self.create_initial_changelog();
            tokio::fs::write(&self.changelog_path, &content).await?;
            info!(path = %self.changelog_path.display(), "Created initial CHANGELOG.md");
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_categorize_from_pr() {
        let mgr = ChangelogManager::new(std::path::PathBuf::from("/tmp"));

        assert_eq!(
            mgr.categorize_from_pr("Add OAuth2 support", None),
            ChangeCategory::Added
        );
        assert_eq!(
            mgr.categorize_from_pr("Fix login bug", None),
            ChangeCategory::Fixed
        );
        assert_eq!(
            mgr.categorize_from_pr("Remove deprecated API", None),
            ChangeCategory::Removed
        );
    }
}
