/// Git Worktree Management for AgentFlow Pair Harness
///
/// Implements isolated worktree creation and cleanup for FORGE-SENTINEL pairs.
/// Each pair gets its own worktree on a dedicated branch (forge-N/T-{id}).
use anyhow::{Context, Result};
use git2::{BranchType, Repository, ResetType};
use std::path::PathBuf;
use tracing::{info, warn};

/// Manages Git worktrees for a single pair slot
pub struct WorktreeManager {
    /// Main repository path (contains .git)
    repo_path: PathBuf,
    /// Pair identifier (e.g., "pair-1")
    pair_id: String,
}

impl WorktreeManager {
    pub fn new(repo_path: PathBuf, pair_id: String) -> Self {
        Self { repo_path, pair_id }
    }

    /// Creates a new worktree for the given ticket
    ///
    /// # Validation: C1-01 Git Worktree Isolation
    /// Creates distinct worktrees in /worktrees/pair-N
    ///
    /// # Validation: C1-02 Branch Ownership
    /// Enforces forge-N/T-{id} branch naming
    ///
    /// # Validation: C5-02 Idempotency
    /// Cleans up old worktree if it exists before creating new one
    pub async fn create_worktree(&self, ticket_id: &str) -> Result<PathBuf> {
        let worktree_path = self.worktree_path();
        let branch_name = self.branch_name(ticket_id);

        info!(
            pair_id = %self.pair_id,
            ticket_id = %ticket_id,
            worktree_path = %worktree_path.display(),
            branch_name = %branch_name,
            "Creating worktree"
        );

        // Validation: C5-02 Idempotency - Clean up old worktree if exists
        if worktree_path.exists() {
            warn!(
                pair_id = %self.pair_id,
                "Worktree already exists, removing old one"
            );
            self.remove_worktree().await?;
        }

        let repo = Repository::open(&self.repo_path).context("Failed to open repository")?;

        // Ensure main branch is up to date
        self.update_main_branch(&repo).await?;

        // Create new branch from main
        let main_commit = repo
            .revparse_single("main")
            .context("Failed to find main branch")?
            .peel_to_commit()
            .context("Failed to peel to commit")?;

        // Check if branch already exists and delete it
        if let Ok(mut branch) = repo.find_branch(&branch_name, BranchType::Local) {
            warn!(branch_name = %branch_name, "Branch already exists, deleting");
            branch
                .delete()
                .context("Failed to delete existing branch")?;
        }

        // Create the branch
        repo.branch(&branch_name, &main_commit, false)
            .context("Failed to create branch")?;

        // Create worktree directory
        std::fs::create_dir_all(worktree_path.parent().unwrap())
            .context("Failed to create worktrees directory")?;

        // Add worktree using git2
        // Note: git2 doesn't have native worktree support, so we use Command
        let output = tokio::process::Command::new("git")
            .current_dir(&self.repo_path)
            .arg("worktree")
            .arg("add")
            .arg(&worktree_path)
            .arg(&branch_name)
            .output()
            .await
            .context("Failed to execute git worktree add")?;

        if !output.status.success() {
            anyhow::bail!(
                "git worktree add failed: {}",
                String::from_utf8_lossy(&output.stderr)
            );
        }

        info!(
            pair_id = %self.pair_id,
            worktree_path = %worktree_path.display(),
            "Worktree created successfully"
        );

        Ok(worktree_path)
    }

    /// Removes the worktree for this pair
    ///
    /// # Validation: C1-01 Git Worktree Isolation
    /// Safely removes pair-specific worktree without affecting others
    pub async fn remove_worktree(&self) -> Result<()> {
        let worktree_path = self.worktree_path();

        if !worktree_path.exists() {
            info!(pair_id = %self.pair_id, "Worktree does not exist, nothing to remove");
            return Ok(());
        }

        info!(
            pair_id = %self.pair_id,
            worktree_path = %worktree_path.display(),
            "Removing worktree"
        );

        // Remove worktree using git command
        let output = tokio::process::Command::new("git")
            .current_dir(&self.repo_path)
            .arg("worktree")
            .arg("remove")
            .arg(&worktree_path)
            .arg("--force") // Force removal even if there are changes
            .output()
            .await
            .context("Failed to execute git worktree remove")?;

        if !output.status.success() {
            warn!(
                stderr = %String::from_utf8_lossy(&output.stderr),
                "git worktree remove failed, attempting manual cleanup"
            );

            // Manual cleanup if git command fails
            if worktree_path.exists() {
                std::fs::remove_dir_all(&worktree_path)
                    .context("Failed to manually remove worktree directory")?;
            }
        }

        info!(pair_id = %self.pair_id, "Worktree removed successfully");
        Ok(())
    }

    /// Checks for divergence from main and rebases if necessary
    ///
    /// Returns the number of commits behind main
    pub async fn check_divergence(&self) -> Result<usize> {
        let worktree_path = self.worktree_path();

        let output = tokio::process::Command::new("git")
            .current_dir(&worktree_path)
            .arg("rev-list")
            .arg("--count")
            .arg("HEAD..origin/main")
            .output()
            .await
            .context("Failed to check divergence")?;

        if !output.status.success() {
            anyhow::bail!(
                "Failed to check divergence: {}",
                String::from_utf8_lossy(&output.stderr)
            );
        }

        let count_str = String::from_utf8_lossy(&output.stdout);
        let count = count_str
            .trim()
            .parse::<usize>()
            .context("Failed to parse commit count")?;

        Ok(count)
    }

    /// Rebases the worktree branch onto main
    pub async fn rebase_on_main(&self) -> Result<()> {
        let worktree_path = self.worktree_path();

        info!(
            pair_id = %self.pair_id,
            worktree_path = %worktree_path.display(),
            "Rebasing on main"
        );

        let output = tokio::process::Command::new("git")
            .current_dir(&worktree_path)
            .arg("rebase")
            .arg("origin/main")
            .output()
            .await
            .context("Failed to execute git rebase")?;

        if !output.status.success() {
            anyhow::bail!("Rebase failed: {}", String::from_utf8_lossy(&output.stderr));
        }

        info!(pair_id = %self.pair_id, "Rebase completed successfully");
        Ok(())
    }

    /// Returns the path to this pair's worktree
    ///
    /// # Validation: C1-01 Git Worktree Isolation
    /// Path follows /worktrees/pair-N pattern
    pub fn worktree_path(&self) -> PathBuf {
        self.repo_path.join("worktrees").join(&self.pair_id)
    }

    /// Returns the branch name for a given ticket
    ///
    /// # Validation: C1-02 Branch Ownership
    /// Format: forge-N/T-{id}
    fn branch_name(&self, ticket_id: &str) -> String {
        format!("forge-{}/{}", self.extract_pair_number(), ticket_id)
    }

    /// Extracts pair number from pair_id (e.g., "pair-1" -> "1")
    fn extract_pair_number(&self) -> &str {
        self.pair_id.strip_prefix("pair-").unwrap_or(&self.pair_id)
    }

    /// Updates the main branch to latest from origin
    async fn update_main_branch(&self, repo: &Repository) -> Result<()> {
        info!("Updating main branch from origin");

        // Fetch origin
        let mut remote = repo
            .find_remote("origin")
            .context("Failed to find origin remote")?;

        remote
            .fetch(&["main"], None, None)
            .context("Failed to fetch from origin")?;

        // Get origin/main commit
        let _origin_main = repo
            .revparse_single("origin/main")
            .context("Failed to find origin/main")?
            .peel_to_commit()
            .context("Failed to peel origin/main to commit")?;

        // Reset main to origin/main (in the main checkout, not worktree)
        let main_path = self.repo_path.join("main");
        if main_path.exists() {
            let main_repo = Repository::open(&main_path).context("Failed to open main checkout")?;

            let obj = main_repo
                .revparse_single("origin/main")
                .context("Failed to find origin/main in main checkout")?;

            main_repo
                .reset(&obj, ResetType::Hard, None)
                .context("Failed to reset main to origin/main")?;
        }

        info!("Main branch updated successfully");
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_branch_name_generation() {
        let manager = WorktreeManager::new(PathBuf::from("/project"), "pair-1".to_string());
        assert_eq!(manager.branch_name("T-42"), "forge-1/T-42");
    }

    #[test]
    fn test_worktree_path() {
        let manager = WorktreeManager::new(PathBuf::from("/project"), "pair-3".to_string());
        assert_eq!(
            manager.worktree_path(),
            PathBuf::from("/project/worktrees/pair-3")
        );
    }

    #[test]
    fn test_extract_pair_number() {
        let manager = WorktreeManager::new(PathBuf::from("/project"), "pair-5".to_string());
        assert_eq!(manager.extract_pair_number(), "5");
    }
}
