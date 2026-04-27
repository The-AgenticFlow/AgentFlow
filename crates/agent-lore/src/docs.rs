// crates/agent-lore/src/docs.rs
use anyhow::Result;
use std::path::PathBuf;
use tracing::info;

use crate::types::DocScope;

pub struct DocsManager {
    docs_dir: PathBuf,
}

impl DocsManager {
    pub fn new(docs_dir: PathBuf) -> Self {
        Self { docs_dir }
    }

    pub async fn ensure_structure(&self) -> Result<()> {
        let subdirs = vec!["api", "architecture", "adr", "usage", "guides"];

        for subdir in subdirs {
            let path = self.docs_dir.join(subdir);
            if !path.exists() {
                tokio::fs::create_dir_all(&path).await?;
                info!(path = %path.display(), "Created docs subdirectory");
            }
        }

        Ok(())
    }

    pub async fn list_docs(&self, scope: DocScope) -> Result<Vec<PathBuf>> {
        let subdir = match scope {
            DocScope::Full => self.docs_dir.clone(),
            DocScope::Api => self.docs_dir.join("api"),
            DocScope::UserGuide => self.docs_dir.join("usage"),
            DocScope::Architecture => self.docs_dir.join("architecture"),
        };

        if !subdir.exists() {
            return Ok(Vec::new());
        }

        let mut entries = Vec::new();
        Box::pin(collect_md_files(&subdir, &mut entries)).await?;
        entries.sort();
        Ok(entries)
    }

    pub async fn write_doc(&self, relative_path: &str, content: &str) -> Result<PathBuf> {
        let path = self.docs_dir.join(relative_path);

        if let Some(parent) = path.parent() {
            if !parent.exists() {
                tokio::fs::create_dir_all(parent).await?;
            }
        }

        tokio::fs::write(&path, content).await?;
        info!(path = %path.display(), "Documentation file written");
        Ok(path)
    }

    pub async fn read_doc(&self, relative_path: &str) -> Result<Option<String>> {
        let path = self.docs_dir.join(relative_path);
        if path.exists() {
            let content = tokio::fs::read_to_string(&path).await?;
            Ok(Some(content))
        } else {
            Ok(None)
        }
    }

    pub async fn doc_exists(&self, relative_path: &str) -> bool {
        self.docs_dir.join(relative_path).exists()
    }
}

async fn collect_md_files(dir: &PathBuf, entries: &mut Vec<PathBuf>) -> Result<()> {
    let mut dir_reader = tokio::fs::read_dir(dir).await?;
    while let Some(entry) = dir_reader.next_entry().await? {
        let path = entry.path();
        if path.is_dir() {
            Box::pin(collect_md_files(&path, entries)).await?;
        } else if path.extension().map(|e| e == "md").unwrap_or(false) {
            entries.push(path);
        }
    }
    Ok(())
}
