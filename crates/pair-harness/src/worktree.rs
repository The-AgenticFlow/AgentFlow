// crates/pair-harness/src/worktree.rs
//! Git worktree management for pair isolation.

use anyhow::{anyhow, bail, Context, Result};
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
    /// Implements worktree reuse: when a pair gets a new ticket, the existing
    /// worktree is reused by fetching origin/main and creating a new branch.
    ///
    /// # Arguments
    /// * `pair_id` - Pair identifier (e.g., "pair-1", "forge-1")
    /// * `ticket_id` - Ticket identifier (e.g., "T-42")
    ///
    /// # Returns
    /// Path to the worktree.
    pub fn create_worktree(&self, pair_id: &str, ticket_id: &str) -> Result<PathBuf> {
        let worktree_path = self.worktrees_dir.join(pair_id);
        let branch_name = Self::branch_name(pair_id, ticket_id);

        info!(pair_id, ticket_id, branch = %branch_name, "Creating/reusing worktree");

        if worktree_path.exists() {
            if let Ok(current) = self.get_current_branch(&worktree_path) {
                if current == branch_name {
                    info!(
                        path = %worktree_path.display(),
                        branch = %branch_name,
                        "Worktree already on correct branch - reusing"
                    );
                    return Ok(worktree_path);
                }
                info!(
                    path = %worktree_path.display(),
                    current = %current,
                    new_branch = %branch_name,
                    "Reusing existing worktree for new ticket"
                );
                return self.reuse_worktree(&worktree_path, &branch_name);
            }
            warn!(path = %worktree_path.display(), "Worktree exists but branch unknown, replacing");
            self.remove_worktree_by_path(&worktree_path, "unknown")?;
        }

        self.prune_stale_worktrees();
        self.delete_branch_if_exists(&branch_name);

        std::fs::create_dir_all(&self.worktrees_dir)
            .context("Failed to create worktrees directory")?;

        let output = Command::new("git")
            .args(["worktree", "add"])
            .arg(&worktree_path)
            .args(["-b", &branch_name])
            .current_dir(&self.project_root)
            .output()
            .context("Failed to run git worktree add")?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
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

    /// Reuse an existing worktree by fetching origin/main and creating a new branch.
    fn reuse_worktree(&self, worktree_path: &Path, new_branch: &str) -> Result<PathBuf> {
        self.fetch_and_reset_to_main(worktree_path)?;
        self.create_branch_from_main(worktree_path, new_branch)?;

        info!(
            path = %worktree_path.display(),
            branch = %new_branch,
            "Worktree reused successfully"
        );
        Ok(worktree_path.to_path_buf())
    }

    /// Fetch origin/main and reset the worktree to it.
    fn fetch_and_reset_to_main(&self, worktree_path: &Path) -> Result<()> {
        info!(path = %worktree_path.display(), "Fetching origin/main and resetting");

        let fetch = Command::new("git")
            .args(["fetch", "origin", "main"])
            .current_dir(worktree_path)
            .output()
            .context("Failed to fetch origin/main")?;

        if !fetch.status.success() {
            warn!(
                error = %String::from_utf8_lossy(&fetch.stderr),
                "git fetch origin/main failed, continuing"
            );
        }

        let stash = Command::new("git")
            .args(["stash", "--include-untracked"])
            .current_dir(worktree_path)
            .output();

        if let Ok(output) = stash {
            if output.status.success() {
                info!(path = %worktree_path.display(), "Stashed uncommitted changes");
            }
        }

        let checkout = Command::new("git")
            .args(["checkout", "main"])
            .current_dir(worktree_path)
            .output()
            .context("Failed to checkout main")?;

        if !checkout.status.success() {
            let checkout = Command::new("git")
                .args(["checkout", "origin/main"])
                .current_dir(worktree_path)
                .output()
                .context("Failed to checkout origin/main")?;

            if !checkout.status.success() {
                return Err(anyhow!(
                    "Failed to checkout main or origin/main: {}",
                    String::from_utf8_lossy(&checkout.stderr)
                ));
            }
        }

        let pull = Command::new("git")
            .args(["pull", "origin", "main"])
            .current_dir(worktree_path)
            .output()
            .context("Failed to pull origin/main")?;

        if !pull.status.success() {
            warn!(
                error = %String::from_utf8_lossy(&pull.stderr),
                "git pull origin/main failed, continuing"
            );
        }

        Ok(())
    }

    /// Create a new branch from the current HEAD (assumed to be main).
    fn create_branch_from_main(&self, worktree_path: &Path, branch_name: &str) -> Result<()> {
        self.delete_branch_if_exists(branch_name);

        let output = Command::new("git")
            .args(["checkout", "-b", branch_name])
            .current_dir(worktree_path)
            .output()
            .context("Failed to create new branch")?;

        if !output.status.success() {
            return Err(anyhow!(
                "Failed to create branch {}: {}",
                branch_name,
                String::from_utf8_lossy(&output.stderr)
            ));
        }

        info!(path = %worktree_path.display(), branch = %branch_name, "Created new branch from main");
        Ok(())
    }

    /// Remove a worktree and its associated branch by pair_id.
    pub fn remove_worktree(&self, pair_id: &str) -> Result<()> {
        let worktree_path = self.worktrees_dir.join(pair_id);

        if !worktree_path.exists() {
            bail!("No worktree found for pair {}", pair_id);
        }

        let current_branch = self.get_current_branch(&worktree_path).ok();
        let branch_name = current_branch.unwrap_or_else(|| "unknown".to_string());

        self.remove_worktree_by_path(&worktree_path, &branch_name)
    }

    /// Remove a specific worktree (now keyed by pair_id only).
    pub fn remove_worktree_for_ticket(&self, pair_id: &str, _ticket_id: &str) -> Result<()> {
        self.remove_worktree(pair_id)
    }

    /// Remove a worktree by its path and branch name.
    fn remove_worktree_by_path(&self, worktree_path: &Path, branch_name: &str) -> Result<()> {
        info!(path = %worktree_path.display(), "Removing worktree");

        let output = Command::new("git")
            .args(["worktree", "remove"])
            .arg(worktree_path)
            .current_dir(&self.project_root)
            .output();

        match output {
            Ok(output) if output.status.success() => {
                info!("Worktree removed successfully");
            }
            Ok(output) => {
                let stderr = String::from_utf8_lossy(&output.stderr);
                warn!(error = %stderr, "Git worktree remove failed, forcing removal");

                let output = Command::new("git")
                    .args(["worktree", "remove", "--force"])
                    .arg(worktree_path)
                    .current_dir(&self.project_root)
                    .output()
                    .context("Failed to force remove worktree")?;

                if !output.status.success() {
                    warn!(path = %worktree_path.display(), "Forcing manual worktree removal");
                    if worktree_path.exists() {
                        std::fs::remove_dir_all(worktree_path)
                            .context("Failed to manually remove worktree directory")?;
                    }
                }
            }
            Err(e) => {
                warn!(error = %e, "Failed to run git worktree remove");
                if worktree_path.exists() {
                    std::fs::remove_dir_all(worktree_path)
                        .context("Failed to manually remove worktree directory")?;
                }
            }
        }

        self.prune_stale_worktrees();
        self.delete_branch_if_exists(branch_name);

        info!("Worktree removed");
        Ok(())
    }

    /// Create an idle worktree on main branch (keyed by pair_id).
    pub fn create_idle_worktree(&self, pair_id: &str) -> Result<PathBuf> {
        let worktree_path = self.worktrees_dir.join(pair_id);

        info!(pair_id, "Creating idle worktree on main");

        if worktree_path.exists() {
            let current = self.get_current_branch(&worktree_path).ok();
            self.remove_worktree_by_path(&worktree_path, &current.unwrap_or_default())?;
        }

        std::fs::create_dir_all(&self.worktrees_dir)
            .context("Failed to create worktrees directory")?;

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

    /// Fetch origin/main and merge it into the worktree branch.
    ///
    /// This materializes conflicts locally so FORGE can see and resolve them.
    /// Used when VESSEL detects merge conflicts on GitHub but the worktree
    /// doesn't have them locally because main was never merged in.
    pub fn merge_origin_main(&self, worktree_path: &Path) -> Result<MergeMainResult> {
        info!(path = %worktree_path.display(), "Fetching origin/main into worktree");

        let fetch = Command::new("git")
            .args(["fetch", "origin", "main"])
            .current_dir(worktree_path)
            .output()
            .context("Failed to fetch origin/main in worktree")?;

        if !fetch.status.success() {
            return Err(anyhow!(
                "git fetch origin/main failed in worktree: {}",
                String::from_utf8_lossy(&fetch.stderr)
            ));
        }

        info!(path = %worktree_path.display(), "Merging origin/main into worktree branch");

        let merge = Command::new("git")
            .args(["merge", "origin/main", "--no-edit"])
            .current_dir(worktree_path)
            .output()
            .context("Failed to merge origin/main in worktree")?;

        if merge.status.success() {
            info!(path = %worktree_path.display(), "origin/main merged cleanly — no conflicts");
            return Ok(MergeMainResult::Clean);
        }

        let stderr = String::from_utf8_lossy(&merge.stderr);

        if stderr.contains("refusing to merge unrelated histories") {
            warn!(
                path = %worktree_path.display(),
                "Branch and origin/main have unrelated histories — retrying with --allow-unrelated-histories"
            );
            let retry = Command::new("git")
                .args([
                    "merge",
                    "origin/main",
                    "--no-edit",
                    "--allow-unrelated-histories",
                ])
                .current_dir(worktree_path)
                .output()
                .context("Failed to merge origin/main with --allow-unrelated-histories")?;

            if retry.status.success() {
                info!(path = %worktree_path.display(), "origin/main merged cleanly with --allow-unrelated-histories");
                return Ok(MergeMainResult::Clean);
            }

            let retry_stderr = String::from_utf8_lossy(&retry.stderr);
            if retry_stderr.contains("conflict") || retry_stderr.contains("CONFLICT") {
                let conflicted_files = Self::list_conflicted_files_in(worktree_path)?;
                warn!(
                    path = %worktree_path.display(),
                    files = conflicted_files.len(),
                    "Merge with --allow-unrelated-histories produced conflict markers"
                );
                return Ok(MergeMainResult::Conflict { conflicted_files });
            }

            return Err(anyhow!(
                "git merge origin/main --allow-unrelated-histories failed: {}",
                retry_stderr
            ));
        }

        if stderr.contains("conflict") || stderr.contains("CONFLICT") {
            let conflicted_files = Self::list_conflicted_files_in(worktree_path)?;
            warn!(
                path = %worktree_path.display(),
                files = conflicted_files.len(),
                "Merge produced conflict markers in worktree"
            );
            return Ok(MergeMainResult::Conflict { conflicted_files });
        }

        Err(anyhow!("git merge origin/main failed: {}", stderr))
    }

    /// List files with conflict markers in a worktree.
    fn list_conflicted_files_in(worktree_path: &Path) -> Result<Vec<String>> {
        let output = Command::new("git")
            .args(["diff", "--name-only", "--diff-filter=U"])
            .current_dir(worktree_path)
            .output()
            .context("Failed to list conflicted files")?;

        let files = String::from_utf8_lossy(&output.stdout)
            .lines()
            .map(|l| l.trim().to_string())
            .filter(|l| !l.is_empty())
            .collect();
        Ok(files)
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

    fn prune_stale_worktrees(&self) {
        let _ = Command::new("git")
            .args(["worktree", "prune"])
            .current_dir(&self.project_root)
            .output();
    }

    /// Force-push the worktree's current branch to origin (with --force-with-lease).
    /// Used after merging origin/main during conflict rework to update the remote branch
    /// so GitHub re-evaluates the PR's mergeability.
    pub fn force_push_branch(&self, worktree_path: &Path) -> Result<()> {
        let branch = self.get_current_branch(worktree_path)?;

        let fetch = Command::new("git")
            .args(["fetch", "origin"])
            .current_dir(worktree_path)
            .output()
            .context("Failed to fetch before force-push")?;

        if !fetch.status.success() {
            warn!(
                path = %worktree_path.display(),
                error = %String::from_utf8_lossy(&fetch.stderr),
                "git fetch origin failed before force-push — continuing anyway"
            );
        }

        info!(path = %worktree_path.display(), branch = %branch, "Force-pushing branch to origin with --force-with-lease");

        let output = Command::new("git")
            .args(["push", "origin", &branch, "--force-with-lease"])
            .current_dir(worktree_path)
            .output()
            .context("Failed to force-push branch")?;

        if output.status.success() {
            info!(path = %worktree_path.display(), branch = %branch, "Force-push succeeded");
            Ok(())
        } else {
            let stderr = String::from_utf8_lossy(&output.stderr);
            if stderr.contains("stale info") || stderr.contains("rejected") {
                warn!(
                    path = %worktree_path.display(),
                    branch = %branch,
                    "force-with-lease rejected (stale info) — falling back to --force"
                );
                let force = Command::new("git")
                    .args(["push", "origin", &branch, "--force"])
                    .current_dir(worktree_path)
                    .output()
                    .context("Failed to force-push branch")?;

                if force.status.success() {
                    info!(path = %worktree_path.display(), branch = %branch, "Force-push (no lease) succeeded");
                    return Ok(());
                }
                let force_stderr = String::from_utf8_lossy(&force.stderr);
                return Err(anyhow!("Force-push failed: {}", force_stderr));
            }
            Err(anyhow!("Force-push failed: {}", stderr))
        }
    }

    fn delete_branch_if_exists(&self, branch_name: &str) {
        let output = Command::new("git")
            .args(["branch", "-D"])
            .arg(branch_name)
            .current_dir(&self.project_root)
            .output();

        match output {
            Ok(o) if o.status.success() => {
                info!(branch = branch_name, "Deleted stale branch");
            }
            _ => {
                debug!(
                    branch = branch_name,
                    "Branch does not exist or could not be deleted"
                );
            }
        }
    }

    /// Generate branch name for a pair/ticket.
    pub fn branch_name(pair_id: &str, ticket_id: &str) -> String {
        if pair_id.starts_with("forge-") {
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

/// Result of a merge origin/main operation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MergeMainResult {
    /// Merge completed cleanly — no conflicts
    Clean,
    /// Merge produced conflict markers that need resolution
    Conflict { conflicted_files: Vec<String> },
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
