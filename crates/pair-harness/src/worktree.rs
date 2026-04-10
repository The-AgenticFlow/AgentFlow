// crates/pair-harness/src/worktree.rs
//! Git worktree management for pair isolation.

use anyhow::{anyhow, Context, Result};
use std::path::{Path, PathBuf};
use std::process::Command;
use tracing::{debug, info, warn};

/// Manages Git worktrees for pair isolation.
pub struct WorktreeManager {
    /// Project root directory (contains .git)
    project_root: PathBuf,
    /// Directory where worktrees are created
    worktrees_dir: PathBuf,
}

impl WorktreeManager {
    /// Create a new worktree manager.
    pub fn new(project_root: impl Into<PathBuf>) -> Self {
        let project_root = project_root.into();
        Self {
            worktrees_dir: project_root.join("worktrees"),
            project_root,
        }
    }

    /// Create a worktree for a pair on a new branch.
    ///
    /// # Arguments
    /// * `pair_id` - Pair identifier (e.g., "pair-1")
    /// * `ticket_id` - Ticket identifier (e.g., "T-42")
    ///
    /// # Returns
    /// Path to the created worktree.
    pub fn create_worktree(&self, pair_id: &str, ticket_id: &str) -> Result<PathBuf> {
        let worktree_path = self.worktrees_dir.join(pair_id);
        let branch_name = Self::branch_name(pair_id, ticket_id);

        info!(pair_id, ticket_id, branch = %branch_name, "Creating worktree");

        // Ensure main is current
        self.run_git_in_main(&["fetch", "origin", "main"])?;
        self.run_git_in_main(&["merge", "origin/main"])?;

        // Check if worktree already exists
        if worktree_path.exists() {
            warn!(path = %worktree_path.display(), "Worktree already exists, removing");
            self.remove_worktree(pair_id)?;
        }

        // Create the worktrees directory if needed
        std::fs::create_dir_all(&self.worktrees_dir)
            .context("Failed to create worktrees directory")?;

        // Create worktree on a new branch
        let output = Command::new("git")
            .args(["worktree", "add"])
            .arg(&worktree_path)
            .args(["-b", &branch_name])
            .current_dir(&self.project_root)
            .output()
            .context("Failed to run git worktree add")?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            // If branch already exists, try to create worktree from existing branch
            if stderr.contains("already exists") {
                info!(branch = %branch_name, "Branch exists, creating worktree from existing branch");
                let output = Command::new("git")
                    .args(["worktree", "add"])
                    .arg(&worktree_path)
                    .arg(&branch_name)
                    .current_dir(&self.project_root)
                    .output()
                    .context("Failed to run git worktree add from existing branch")?;

                if !output.status.success() {
                    return Err(anyhow!(
                        "Failed to create worktree from existing branch: {}",
                        String::from_utf8_lossy(&output.stderr)
                    ));
                }
            } else {
                return Err(anyhow!("Failed to create worktree: {}", stderr));
            }
        }

        // Verify the worktree is clean
        let status = Command::new("git")
            .args(["status", "--porcelain"])
            .current_dir(&worktree_path)
            .output()
            .context("Failed to run git status")?;

        if !status.stdout.is_empty() {
            warn!(path = %worktree_path.display(), "Worktree is not clean");
        }

        info!(path = %worktree_path.display(), branch = %branch_name, "Worktree created successfully");
        Ok(worktree_path)
    }

    /// Remove a worktree.
    pub fn remove_worktree(&self, pair_id: &str) -> Result<()> {
        let worktree_path = self.worktrees_dir.join(pair_id);

        info!(path = %worktree_path.display(), "Removing worktree");

        // Try graceful removal first
        let output = Command::new("git")
            .args(["worktree", "remove"])
            .arg(&worktree_path)
            .current_dir(&self.project_root)
            .output();

        match output {
            Ok(output) if output.status.success() => {
                info!(pair_id, "Worktree removed successfully");
                return Ok(());
            }
            Ok(output) => {
                let stderr = String::from_utf8_lossy(&output.stderr);
                warn!(error = %stderr, "Git worktree remove failed, forcing removal");
            }
            Err(e) => {
                warn!(error = %e, "Failed to run git worktree remove");
            }
        }

        // Force removal if graceful failed
        let output = Command::new("git")
            .args(["worktree", "remove", "--force"])
            .arg(&worktree_path)
            .current_dir(&self.project_root)
            .output()
            .context("Failed to force remove worktree")?;

        if !output.status.success() {
            // Last resort: manual removal
            warn!(path = %worktree_path.display(), "Forcing manual worktree removal");
            if worktree_path.exists() {
                std::fs::remove_dir_all(&worktree_path)
                    .context("Failed to manually remove worktree directory")?;
            }
        }

        // Clean up any stale worktree references
        let _ = Command::new("git")
            .args(["worktree", "prune"])
            .current_dir(&self.project_root)
            .output();

        info!(pair_id, "Worktree removed");
        Ok(())
    }

    /// Create an idle worktree on main branch.
    pub fn create_idle_worktree(&self, pair_id: &str) -> Result<PathBuf> {
        let worktree_path = self.worktrees_dir.join(pair_id);

        info!(pair_id, "Creating idle worktree on main");

        // Remove existing worktree if any
        if worktree_path.exists() {
            self.remove_worktree(pair_id)?;
        }

        // Create worktrees directory if needed
        std::fs::create_dir_all(&self.worktrees_dir)
            .context("Failed to create worktrees directory")?;

        // Create worktree on main branch
        let output = Command::new("git")
            .args(["worktree", "add"])
            .arg(&worktree_path)
            .arg("main")
            .current_dir(&self.project_root)
            .output()
            .context("Failed to run git worktree add")?;

        if !output.status.success() {
            return Err(anyhow!(
                "Failed to create idle worktree: {}",
                String::from_utf8_lossy(&output.stderr)
            ));
        }

        info!(path = %worktree_path.display(), "Idle worktree created");
        Ok(worktree_path)
    }

    /// Check for divergence from main and optionally rebase.
    pub fn check_divergence(
        &self,
        worktree_path: &Path,
        threshold: u32,
    ) -> Result<DivergenceStatus> {
        let behind = self.count_commits_behind(worktree_path)?;

        debug!(path = %worktree_path.display(), behind, "Divergence check");

        if behind > threshold {
            info!(behind, threshold, "Branch is behind main, rebase needed");
            return Ok(DivergenceStatus::NeedsRebase {
                commits_behind: behind,
            });
        }

        Ok(DivergenceStatus::UpToDate)
    }

    /// Rebase the worktree onto origin/main.
    pub fn rebase_onto_main(&self, worktree_path: &Path) -> Result<RebaseResult> {
        info!(path = %worktree_path.display(), "Rebasing onto origin/main");

        // Fetch latest
        let output = Command::new("git")
            .args(["fetch", "origin", "main"])
            .current_dir(worktree_path)
            .output()
            .context("Failed to fetch origin/main")?;

        if !output.status.success() {
            return Err(anyhow!(
                "Failed to fetch: {}",
                String::from_utf8_lossy(&output.stderr)
            ));
        }

        // Rebase
        let output = Command::new("git")
            .args(["rebase", "origin/main"])
            .current_dir(worktree_path)
            .output()
            .context("Failed to rebase")?;

        if output.status.success() {
            info!(path = %worktree_path.display(), "Rebase successful");
            return Ok(RebaseResult::Success);
        }

        let stderr = String::from_utf8_lossy(&output.stderr);
        if stderr.contains("conflict") {
            warn!(path = %worktree_path.display(), "Rebase has conflicts");
            return Ok(RebaseResult::Conflict);
        }

        Err(anyhow!("Rebase failed: {}", stderr))
    }

    /// Abort an in-progress rebase.
    pub fn abort_rebase(&self, worktree_path: &Path) -> Result<()> {
        let output = Command::new("git")
            .args(["rebase", "--abort"])
            .current_dir(worktree_path)
            .output()
            .context("Failed to abort rebase")?;

        if !output.status.success() {
            warn!(error = %String::from_utf8_lossy(&output.stderr), "Failed to abort rebase");
        }

        Ok(())
    }

    /// Get the current branch name in a worktree.
    pub fn get_current_branch(&self, worktree_path: &Path) -> Result<String> {
        let output = Command::new("git")
            .args(["rev-parse", "--abbrev-ref", "HEAD"])
            .current_dir(worktree_path)
            .output()
            .context("Failed to get current branch")?;

        if !output.status.success() {
            return Err(anyhow!(
                "Failed to get branch: {}",
                String::from_utf8_lossy(&output.stderr)
            ));
        }

        Ok(String::from_utf8(output.stdout)?.trim().to_string())
    }

    /// Count commits behind origin/main.
    fn count_commits_behind(&self, worktree_path: &Path) -> Result<u32> {
        let output = Command::new("git")
            .args(["rev-list", "--count", "HEAD..origin/main"])
            .current_dir(worktree_path)
            .output()
            .context("Failed to count commits behind")?;

        if !output.status.success() {
            // If origin/main doesn't exist, return 0
            return Ok(0);
        }

        let count: u32 = String::from_utf8(output.stdout)?
            .trim()
            .parse()
            .unwrap_or(0);

        Ok(count)
    }

    /// Run a git command in the main directory.
    fn run_git_in_main(&self, args: &[&str]) -> Result<()> {
        let output = Command::new("git")
            .args(args)
            .current_dir(&self.project_root)
            .output()
            .context("Failed to run git command")?;

        if !output.status.success() {
            return Err(anyhow!(
                "Git command failed: {}",
                String::from_utf8_lossy(&output.stderr)
            ));
        }

        Ok(())
    }

    /// Generate branch name for a pair/ticket.
    pub fn branch_name(pair_id: &str, ticket_id: &str) -> String {
        // Handle both "forge-1" and "pair-1" style pair IDs
        if pair_id.starts_with("forge-") || pair_id.starts_with("pair-") {
            format!("{}/{}", pair_id, ticket_id)
        } else {
            format!("forge-{}/{}", pair_id, ticket_id)
        }
    }
}

/// Status of branch divergence from main.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DivergenceStatus {
    /// Branch is up to date with main
    UpToDate,
    /// Branch needs rebase
    NeedsRebase { commits_behind: u32 },
}

/// Result of a rebase operation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RebaseResult {
    /// Rebase completed successfully
    Success,
    /// Rebase has conflicts that need resolution
    Conflict,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_branch_name() {
        assert_eq!(
            WorktreeManager::branch_name("pair-1", "T-42"),
            "forge-pair-1/T-42"
        );
    }
}
