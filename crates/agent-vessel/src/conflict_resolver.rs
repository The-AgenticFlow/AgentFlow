// crates/agent-vessel/src/conflict_resolver.rs
//
// Merge Conflict Resolution — detects, attempts auto-resolution, and falls
// back to LLM-assisted intelligent resolution to preserve both sides' intent.

use anyhow::{Context, Result};
use std::path::Path;
use tracing::{debug, info, warn};

pub struct ConflictResolver {
    client: github::GithubRestClient,
}

impl ConflictResolver {
    pub fn new(client: github::GithubRestClient) -> Self {
        Self { client }
    }

    /// Attempt to resolve merge conflicts for a PR.
    ///
    /// Strategy:
    ///   1. Try GitHub's "update branch" API (merges base into head).
    ///      If no text conflicts exist, this succeeds and CI re-triggers.
    ///   2. If update-branch fails with conflicts, attempt local git rebase
    ///      in the worktree using `git rebase origin/main`.
    ///   3. If the rebase itself has conflicts, list the conflicted files
    ///      and return them for LLM-assisted resolution (handled by the caller).
    pub async fn resolve(
        &self,
        owner: &str,
        repo: &str,
        pr_info: &pocketflow_core::PrInfo,
        worktree_path: Option<&Path>,
    ) -> Result<ConflictResolution> {
        info!(
            pr = pr_info.number,
            branch = %pr_info.head_branch,
            "Attempting conflict resolution"
        );

        // Strategy 1: GitHub "update branch" API
        match self.client.update_branch(owner, repo, pr_info.number).await {
            Ok(()) => {
                info!(pr = pr_info.number, "Branch updated via GitHub API — conflicts resolved cleanly");
                return Ok(ConflictResolution::Resolved);
            }
            Err(e) => {
                let msg = e.to_string();
                if msg.contains("Merge conflict") || msg.contains("merge conflict") {
                    info!(pr = pr_info.number, "GitHub update-branch confirmed conflicts — need local resolution");
                } else {
                    warn!(pr = pr_info.number, error = %e, "GitHub update-branch failed — trying local rebase");
                }
            }
        }

        // Strategy 2: Local git rebase in worktree
        if let Some(wt_path) = worktree_path {
            return self.local_rebase(wt_path, &pr_info.head_branch).await;
        }

        // No worktree available — can't do local resolution
        warn!(
            pr = pr_info.number,
            "No worktree path available — cannot attempt local conflict resolution"
        );
        Ok(ConflictResolution::Unresolvable {
            conflicted_files: vec!["unknown — worktree not available".to_string()],
        })
    }

    /// Perform a local git rebase onto origin/main in the worktree.
    async fn local_rebase(
        &self,
        worktree_path: &Path,
        branch: &str,
    ) -> Result<ConflictResolution> {
        info!(
            path = %worktree_path.display(),
            branch,
            "Attempting local git rebase onto origin/main"
        );

        // Fetch latest from origin
        let fetch_output = tokio::process::Command::new("git")
            .args(["fetch", "origin"])
            .current_dir(worktree_path)
            .output()
            .await
            .context("Failed to run git fetch")?;

        if !fetch_output.status.success() {
            let stderr = String::from_utf8_lossy(&fetch_output.stderr);
            warn!(error = %stderr, "git fetch failed");
            anyhow::bail!("git fetch failed: {}", stderr);
        }

        // Attempt rebase onto origin/main
        let rebase_output = tokio::process::Command::new("git")
            .args(["rebase", "origin/main"])
            .current_dir(worktree_path)
            .output()
            .await
            .context("Failed to run git rebase")?;

        if rebase_output.status.success() {
            // Rebase succeeded cleanly — push the result
            let push_output = tokio::process::Command::new("git")
                .args(["push", "origin", branch, "--force-with-lease"])
                .current_dir(worktree_path)
                .output()
                .await
                .context("Failed to push rebased branch")?;

            if push_output.status.success() {
                info!(branch, "Rebase succeeded and pushed — CI will re-trigger");
                return Ok(ConflictResolution::Resolved);
            } else {
                let stderr = String::from_utf8_lossy(&push_output.stderr);
                warn!(error = %stderr, "Push after rebase failed");
                let _ = Self::abort_rebase(worktree_path).await;
                anyhow::bail!("Push after rebase failed: {}", stderr);
            }
        }

        // Rebase has conflicts — list conflicted files
        let conflicted = self.list_conflicted_files(worktree_path).await?;

        if conflicted.is_empty() {
            let _ = Self::abort_rebase(worktree_path).await;
            anyhow::bail!("Rebase failed but no conflicted files found — unknown error");
        }

        warn!(
            files = conflicted.len(),
            "Rebase has conflicts that require intelligent resolution"
        );

        Ok(ConflictResolution::NeedsIntelligentResolution { conflicted_files: conflicted })
    }

    /// List files with conflict markers from an in-progress rebase/merge.
    async fn list_conflicted_files(&self, worktree_path: &Path) -> Result<Vec<String>> {
        let output = tokio::process::Command::new("git")
            .args(["diff", "--name-only", "--diff-filter=U"])
            .current_dir(worktree_path)
            .output()
            .await
            .context("Failed to list conflicted files")?;

        let stdout = String::from_utf8_lossy(&output.stdout);
        let files: Vec<String> = stdout
            .lines()
            .map(|l| l.trim().to_string())
            .filter(|l| !l.is_empty())
            .collect();

        Ok(files)
    }

    /// Abort an in-progress rebase.
    pub async fn abort_rebase(worktree_path: &Path) -> Result<()> {
        let output = tokio::process::Command::new("git")
            .args(["rebase", "--abort"])
            .current_dir(worktree_path)
            .output()
            .await
            .context("Failed to abort rebase")?;

        if output.status.success() {
            debug!(path = %worktree_path.display(), "Rebase aborted");
        } else {
            warn!(path = %worktree_path.display(), "Failed to abort rebase");
        }
        Ok(())
    }
}

/// Result of conflict resolution attempt.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ConflictResolution {
    /// Conflicts resolved (either by GitHub update-branch or clean rebase)
    Resolved,
    /// Conflicts require intelligent (LLM) resolution — files listed
    NeedsIntelligentResolution { conflicted_files: Vec<String> },
    /// Conflicts cannot be resolved automatically
    Unresolvable { conflicted_files: Vec<String> },
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_conflict_resolution_variants() {
        let resolved = ConflictResolution::Resolved;
        assert!(matches!(resolved, ConflictResolution::Resolved));

        let needs_llm = ConflictResolution::NeedsIntelligentResolution {
            conflicted_files: vec!["src/main.rs".to_string()],
        };
        assert!(matches!(needs_llm, ConflictResolution::NeedsIntelligentResolution { .. }));

        let unresolvable = ConflictResolution::Unresolvable {
            conflicted_files: vec!["src/lib.rs".to_string()],
        };
        assert!(matches!(unresolvable, ConflictResolution::Unresolvable { .. }));
    }
}
