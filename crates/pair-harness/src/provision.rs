// crates/pair-harness/src/provision.rs
//! Provisioning for pair configuration files.
//!
//! Generates settings.json for FORGE and SENTINEL with auto-mode
//! permissions and explicit allow/deny lists.

use anyhow::{Context, Result};
use serde_json::{json, Value};
use std::fs;
use std::path::{Path, PathBuf};
use tracing::{debug, info};

/// Provisions configuration files for pairs.
pub struct Provisioner {
    /// Project root directory
    project_root: PathBuf,
}

impl Provisioner {
    /// Create a new provisioner.
    pub fn new(project_root: impl Into<PathBuf>) -> Self {
        Self {
            project_root: project_root.into(),
        }
    }

    /// Provision all configuration for a pair.
    pub async fn provision_pair(
        &self,
        pair_id: &str,
        worktree: &Path,
        shared: &Path,
        github_token: &str,
        redis_url: Option<&str>,
    ) -> Result<()> {
        info!(pair = pair_id, "Provisioning pair configuration");

        // 1. Create FORGE settings.json
        self.create_forge_settings(worktree)?;

        // 2. Create SENTINEL settings.json
        self.create_sentinel_settings(shared)?;

        // 3. Create FORGE mcp.json
        let mcp_gen = crate::mcp_config::McpConfigGenerator::new(github_token, redis_url);
        mcp_gen.generate_forge_config(
            worktree,
            shared,
            &worktree.join(".claude").join("mcp.json"),
        )?;

        // 4. Create SENTINEL mcp.json
        mcp_gen.generate_sentinel_config(
            worktree,
            shared,
            &shared.join(".claude").join("mcp.json"),
        )?;

        // 5. Symlink plugin to FORGE
        self.symlink_plugin(worktree, "forge")?;

        // 6. Symlink plugin to SENTINEL
        self.symlink_plugin(shared, "sentinel")?;

        // 7. Create shared directory structure
        self.create_shared_structure(shared)?;

        info!(pair = pair_id, "Pair provisioning complete");
        Ok(())
    }

    /// Create FORGE's settings.json with auto-mode permissions.
    pub fn create_forge_settings(&self, worktree: &Path) -> Result<()> {
        let claude_dir = worktree.join(".claude");
        fs::create_dir_all(&claude_dir).context("Failed to create .claude directory")?;

        let settings_path = claude_dir.join("settings.json");

        info!(path = %settings_path.display(), "Creating FORGE settings.json");

        // Minimal settings - permissions are handled by --dangerously-skip-permissions flag
        let settings = json!({
            "permissions": {
                "defaultMode": "auto"
            }
        });

        self.write_json(&settings_path, &settings)
    }

    /// Create SENTINEL's settings.json with read-only permissions.
    pub fn create_sentinel_settings(&self, shared: &Path) -> Result<()> {
        let legacy_dir = shared.join("sentinel");
        if legacy_dir.exists() {
            fs::remove_dir_all(&legacy_dir)
                .context("Failed to remove legacy sentinel directory")?;
        }

        let claude_dir = shared.join(".claude");
        fs::create_dir_all(&claude_dir).context("Failed to create sentinel .claude directory")?;

        let settings_path = claude_dir.join("settings.json");

        info!(path = %settings_path.display(), "Creating SENTINEL settings.json");

        // Minimal settings - permissions are handled by --dangerously-skip-permissions flag
        let settings = json!({
            "permissions": {
                "defaultMode": "auto"
            }
        });

        self.write_json(&settings_path, &settings)
    }

    /// Symlink the Sprintless plugin to a .claude directory.
    pub fn symlink_plugin(&self, target_dir: &Path, role: &str) -> Result<()> {
        // First check for ORCHESTRATOR_DIR env var (points to orchestrator source with plugin)
        // Fall back to project_root for backwards compatibility
        let plugin_source = if let Ok(orch_dir) = std::env::var("ORCHESTRATOR_DIR") {
            PathBuf::from(orch_dir).join("orchestration").join("plugin")
        } else {
            self.project_root.join("orchestration").join("plugin")
        };

        // Check if plugin exists
        if !plugin_source.exists() {
            debug!(
                role = role,
                path = %plugin_source.display(),
                "Plugin directory not found, skipping symlink"
            );
            return Ok(());
        }

        let plugins_dir = target_dir.join(".claude").join("plugins");

        fs::create_dir_all(&plugins_dir).context("Failed to create plugins directory")?;

        let symlink_path = plugins_dir.join("orchestration");

        // Remove existing symlink if present
        if symlink_path.exists() || symlink_path.symlink_metadata().is_ok() {
            let _ = fs::remove_file(&symlink_path);
        }

        // Create symlink
        #[cfg(unix)]
        std::os::unix::fs::symlink(&plugin_source, &symlink_path)
            .context("Failed to create plugin symlink")?;

        #[cfg(windows)]
        std::os::windows::fs::symlink_dir(&plugin_source, &symlink_path)
            .context("Failed to create plugin symlink")?;

        debug!(
            role = role,
            source = %plugin_source.display(),
            target = %symlink_path.display(),
            "Plugin symlinked"
        );

        Ok(())
    }

    /// Create the shared directory structure.
    pub fn create_shared_structure(&self, shared: &Path) -> Result<()> {
        fs::create_dir_all(shared).context("Failed to create shared directory")?;

        // Clean up the legacy sentinel subdirectory from older runs.
        let legacy_dir = shared.join("sentinel");
        if legacy_dir.exists() {
            fs::remove_dir_all(&legacy_dir)
                .context("Failed to remove legacy sentinel directory")?;
        }

        // Create .gitignore for shared directory
        let gitignore = shared.join(".gitignore");
        fs::write(
            &gitignore,
            "# Shared artifacts are runtime state, not committed\n*\n!.gitignore\n",
        )
        .context("Failed to write .gitignore")?;

        debug!(path = %shared.display(), "Shared directory structure created");
        Ok(())
    }

    /// Write JSON to file atomically.
    fn write_json(&self, path: &Path, value: &Value) -> Result<()> {
        let temp_path = path.with_extension("json.tmp");
        let content = serde_json::to_string_pretty(value).context("Failed to serialize JSON")?;

        fs::write(&temp_path, content).context("Failed to write JSON")?;

        fs::rename(&temp_path, path).context("Failed to rename JSON file")?;

        Ok(())
    }

    /// Write TICKET.md to shared directory.
    pub fn write_ticket(&self, shared: &Path, ticket: &crate::types::Ticket) -> Result<()> {
        let path = shared.join("TICKET.md");

        let content = format!(
            "# {}\n\n**Issue:** #{} \n**URL:** {}\n\n{}\n\n## Acceptance Criteria\n\n{}\n",
            ticket.title,
            ticket.issue_number,
            ticket.url,
            ticket.body,
            ticket
                .acceptance_criteria
                .iter()
                .map(|c| format!("- {}", c))
                .collect::<Vec<_>>()
                .join("\n")
        );

        fs::write(&path, content).context("Failed to write TICKET.md")?;

        info!(path = %path.display(), "TICKET.md written");
        Ok(())
    }

    /// Write TASK.md to shared directory.
    pub fn write_task(&self, shared: &Path, task: &str) -> Result<()> {
        let path = shared.join("TASK.md");

        fs::write(&path, task).context("Failed to write TASK.md")?;

        info!(path = %path.display(), "TASK.md written");
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn test_create_forge_settings() {
        let dir = tempdir().unwrap();
        let worktree = dir.path();

        let provisioner = Provisioner::new(dir.path());
        provisioner.create_forge_settings(worktree).unwrap();

        let settings_path = worktree.join(".claude").join("settings.json");
        assert!(settings_path.exists());

        let content = fs::read_to_string(&settings_path).unwrap();
        let settings: Value = serde_json::from_str(&content).unwrap();

        assert_eq!(settings["permissions"]["defaultMode"], "auto");
    }

    #[test]
    fn test_create_sentinel_settings() {
        let dir = tempdir().unwrap();
        let shared = dir.path();

        let provisioner = Provisioner::new(dir.path());
        provisioner.create_sentinel_settings(shared).unwrap();

        let settings_path = shared.join(".claude").join("settings.json");
        assert!(settings_path.exists());
        assert!(!shared.join("sentinel").exists());

        let content = fs::read_to_string(&settings_path).unwrap();
        let settings: Value = serde_json::from_str(&content).unwrap();

        assert_eq!(settings["permissions"]["defaultMode"], "auto");
    }

    #[test]
    fn test_create_shared_structure() {
        let dir = tempdir().unwrap();
        let shared = dir.path().join("shared");

        let provisioner = Provisioner::new(dir.path());
        provisioner.create_shared_structure(&shared).unwrap();

        assert!(shared.exists());
        assert!(!shared.join("sentinel").exists());
        assert!(shared.join(".gitignore").exists());
    }
}
