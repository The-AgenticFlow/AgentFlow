// crates/pair-harness/src/worktree.rs
//! Git worktree management for pair isolation.

use anyhow::{anyhow, bail, Context, Result};
use std::fs;
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
    /// `WorktreeSetupResult` containing the path and any setup warnings.
    pub fn create_worktree(&self, pair_id: &str, ticket_id: &str) -> Result<WorktreeSetupResult> {
        let worktree_path = self
            .worktrees_dir
            .join(format!("{}-{}", pair_id, ticket_id));
        let branch_name = Self::branch_name(pair_id, ticket_id);
        let mut warnings = Vec::new();

        info!(pair_id, ticket_id, branch = %branch_name, "Creating worktree");

        if let Err(e) = self.run_git_in_main(&["fetch", "origin", "main"]) {
            warn!(error = %e, "git fetch origin/main failed, continuing");
            warnings.push(SetupWarning {
                phase: "fetch_origin_main".to_string(),
                error: e.to_string(),
                affected_files: vec![],
            });
        }
        if let Err(e) = self.run_git_in_main(&["merge", "origin/main"]) {
            warn!(error = %e, "git merge origin/main failed, continuing");
            let affected_files = self.list_unmerged_files_in_main();
            warnings.push(SetupWarning {
                phase: "merge_origin_main".to_string(),
                error: e.to_string(),
                affected_files,
            });
        }

        if worktree_path.exists() {
            if let Ok(current) = self.get_current_branch(&worktree_path) {
                if current == branch_name {
                    info!(
                        path = %worktree_path.display(),
                        branch = %branch_name,
                        "Worktree already exists on correct branch - reusing"
                    );
                    return Ok(WorktreeSetupResult {
                        path: worktree_path,
                        warnings,
                    });
                }
            }
            warn!(path = %worktree_path.display(), "Worktree exists on different branch, replacing");
            self.remove_worktree_by_path(&worktree_path, &Self::branch_name(pair_id, ticket_id))?;
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
            let dirty_files = String::from_utf8_lossy(&status.stdout)
                .lines()
                .filter_map(|l| l.get(3..).map(|s| s.trim().to_string()))
                .filter(|s| !s.is_empty())
                .collect::<Vec<_>>();
            warn!(path = %worktree_path.display(), files = dirty_files.len(), "Worktree is not clean");
            warnings.push(SetupWarning {
                phase: "worktree_dirty".to_string(),
                error: "Worktree has uncommitted changes".to_string(),
                affected_files: dirty_files,
            });
        }

        info!(path = %worktree_path.display(), branch = %branch_name, "Worktree created successfully");
        Ok(WorktreeSetupResult {
            path: worktree_path,
            warnings,
        })
    }

    /// Remove a worktree and its associated branch by pair_id.
    /// Backward-compatible: scans worktrees_dir for a directory starting with `pair_id`.
    pub fn remove_worktree(&self, pair_id: &str) -> Result<()> {
        let matching: Vec<std::fs::DirEntry> = fs::read_dir(&self.worktrees_dir)
            .context("Failed to read worktrees directory")?
            .filter_map(|e: std::io::Result<std::fs::DirEntry>| e.ok())
            .filter(|e: &std::fs::DirEntry| {
                e.file_name()
                    .to_str()
                    .map(|n: &str| n.starts_with(&format!("{}-", pair_id)))
                    .unwrap_or(false)
            })
            .collect();

        if matching.is_empty() {
            bail!("No worktree found for pair {}", pair_id);
        }

        for entry in matching {
            let worktree_path = entry.path();
            let dir_name = entry.file_name().to_str().unwrap_or("").to_string();
            let ticket_id = dir_name
                .strip_prefix(&format!("{}-", pair_id))
                .unwrap_or("unknown");
            let branch_name = Self::branch_name(pair_id, ticket_id);
            self.remove_worktree_by_path(&worktree_path, &branch_name)?;
        }
        Ok(())
    }

    /// Remove a specific worktree by pair_id and ticket_id.
    pub fn remove_worktree_for_ticket(&self, pair_id: &str, ticket_id: &str) -> Result<()> {
        let worktree_path = self
            .worktrees_dir
            .join(format!("{}-{}", pair_id, ticket_id));

        let branch_name = Self::branch_name(pair_id, ticket_id);
        self.remove_worktree_by_path(&worktree_path, &branch_name)
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

    /// Create an idle worktree on main branch.
    pub fn create_idle_worktree(&self, pair_id: &str) -> Result<PathBuf> {
        let worktree_path = self.worktrees_dir.join(format!("{}-idle", pair_id));

        info!(pair_id, "Creating idle worktree on main");

        if worktree_path.exists() {
            let _ = self.remove_worktree_by_path(&worktree_path, "main");
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

    fn list_unmerged_files_in_main(&self) -> Vec<String> {
        Command::new("git")
            .args(["diff", "--name-only", "--diff-filter=U"])
            .current_dir(&self.project_root)
            .output()
            .ok()
            .and_then(|o| {
                if o.status.success() {
                    Some(
                        String::from_utf8_lossy(&o.stdout)
                            .lines()
                            .map(|l| l.trim().to_string())
                            .filter(|l| !l.is_empty())
                            .collect(),
                    )
                } else {
                    None
                }
            })
            .unwrap_or_default()
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

/// Result of worktree creation, including any setup warnings.
#[derive(Debug, Clone)]
pub struct WorktreeSetupResult {
    /// Path to the created worktree.
    pub path: PathBuf,
    /// Warnings encountered during setup (git fetch/merge errors, dirty state, etc.).
    pub warnings: Vec<SetupWarning>,
}

/// A warning encountered during worktree setup.
#[derive(Debug, Clone)]
pub struct SetupWarning {
    /// Phase that produced the warning (e.g., "fetch_origin_main", "merge_origin_main", "worktree_dirty").
    pub phase: String,
    /// The error message or git stderr.
    pub error: String,
    /// Affected files (unmerged/dirty) if detectable.
    pub affected_files: Vec<String>,
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
