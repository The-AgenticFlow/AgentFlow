// crates/agent-lore/src/readme.rs
use anyhow::Result;
use std::path::PathBuf;
use tracing::{info, warn};

pub struct ReadmeManager {
    readme_path: PathBuf,
}

impl ReadmeManager {
    pub fn new(workspace_root: PathBuf) -> Self {
        Self {
            readme_path: workspace_root.join("README.md"),
        }
    }

    pub async fn read_current(&self) -> Result<Option<String>> {
        if self.readme_path.exists() {
            let content = tokio::fs::read_to_string(&self.readme_path).await?;
            Ok(Some(content))
        } else {
            Ok(None)
        }
    }

    pub async fn update_feature_section(&self, feature_name: &str, description: &str) -> Result<bool> {
        let content = self.read_current().await?;

        let Some(mut content) = content else {
            warn!("README.md not found - skipping feature section update");
            return Ok(false);
        };

        let section_header = "## Features";
        let feature_line = format!("- **{}**: {}", feature_name, description);

        if content.contains(&feature_line) {
            info!(feature = feature_name, "Feature already documented in README");
            return Ok(false);
        }

        if let Some(features_pos) = content.find(section_header) {
            let section_start = features_pos + section_header.len();

            let next_section = content[section_start..]
                .find("\n## ")
                .map(|p| section_start + p)
                .unwrap_or(content.len());

            let insert_pos = if content[section_start..next_section].trim().is_empty() {
                section_start
            } else {
                let last_line_in_section = content[section_start..next_section]
                    .lines()
                    .rfind(|l| !l.trim().is_empty())
                    .and_then(|l| {
                        content[section_start..].find(l).map(|p| section_start + p + l.len())
                    })
                    .unwrap_or(section_start);
                last_line_in_section
            };

            content.insert_str(insert_pos, &format!("\n{}", feature_line));
            tokio::fs::write(&self.readme_path, &content).await?;
            info!(feature = feature_name, "Feature section updated in README");
            return Ok(true);
        }

        if !content.contains(&feature_line) {
            if let Some(first_section_pos) = content.find("\n## ") {
                let features_section = format!("\n\n## Features\n\n{}\n", feature_line);
                content.insert_str(first_section_pos, &features_section);
            } else {
                content.push_str(&format!("\n\n## Features\n\n{}\n", feature_line));
            }
            tokio::fs::write(&self.readme_path, &content).await?;
            info!(feature = feature_name, "Features section added to README");
            return Ok(true);
        }

        Ok(false)
    }

    pub async fn update_installation(&self, instructions: &str) -> Result<bool> {
        let content = self.read_current().await?;

        let Some(mut content) = content else {
            warn!("README.md not found - skipping installation update");
            return Ok(false);
        };

        let section_header = "## Installation";

        if let Some(install_pos) = content.find(section_header) {
            let section_start = install_pos + section_header.len();
            let next_section = content[section_start..]
                .find("\n## ")
                .map(|p| section_start + p)
                .unwrap_or(content.len());

            let new_section = format!("\n\n{}", instructions);
            content.replace_range(install_pos..next_section, &format!("{}{}", section_header, new_section));
            tokio::fs::write(&self.readme_path, &content).await?;
            info!("Installation section updated in README");
            return Ok(true);
        }

        let new_section = format!("\n\n{}\n\n{}", section_header, instructions);
        if let Some(first_section_pos) = content.find("\n## ") {
            content.insert_str(first_section_pos, &new_section);
        } else {
            content.push_str(&new_section);
        }
        tokio::fs::write(&self.readme_path, &content).await?;
        info!("Installation section added to README");
        Ok(true)
    }

    pub async fn needs_update(&self, feature_keywords: &[&str]) -> bool {
        let Ok(Some(content)) = self.read_current().await else {
            return false;
        };

        let has_features_section = content.contains("## Features");
        if !has_features_section {
            return true;
        }

        for keyword in feature_keywords {
            if !content.to_lowercase().contains(&keyword.to_lowercase()) {
                return true;
            }
        }

        false
    }
}
