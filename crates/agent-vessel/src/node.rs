// crates/agent-vessel/src/node.rs
//
// VesselNode — orchestrates CI polling, merging, and notification.
// Implements the Node trait for integration with the Flow.

use anyhow::Result;
use async_trait::async_trait;
use config::{
    state::{KEY_TICKETS, KEY_WORKER_SLOTS, KEY_PENDING_PRS},
    ACTION_CONFLICTS_DETECTED, Ticket, TicketStatus, WorkerSlot, WorkerStatus,
};
use pocketflow_core::{Action, CiStatus, Node, PrInfo, SharedStore};
use serde_json::{json, Value};
use std::collections::HashMap;
use std::path::PathBuf;
use tracing::{debug, info, warn};

use crate::ci_poller::CiPollResult;
use crate::conflict_resolver::{ConflictResolution, ConflictResolver};
use crate::types::{VesselConfig, VesselOutcome};
use crate::{CiPoller, PrMerger, VesselNotifier};

/// VESSEL Node — DevOps Specialist and Merge Gatekeeper.
///
/// Three-phase workflow:
/// 1. prep: Read pending PRs from SharedStore
/// 2. exec: Poll CI, detect conflicts, resolve if possible, merge if green, return outcomes
/// 3. post: Emit events, update tickets, return routing action
pub struct VesselNode {
    _config: VesselConfig,
    client: github::GithubRestClient,
    poller: CiPoller,
    merger: PrMerger,
    conflict_resolver: ConflictResolver,
}

/// Environment variable for the workspace root directory.
/// Used to locate worktrees for local conflict resolution.
const ENV_WORKSPACE_ROOT: &str = "AGENTFLOW_WORKSPACE_ROOT";

impl VesselNode {
    pub fn new(config: VesselConfig) -> Self {
        let client = github::GithubRestClient::new(&config.github_token);

        Self {
            poller: CiPoller::new(config.ci_poll.clone(), client.clone()),
            merger: PrMerger::new(client.clone(), config.merge_method),
            conflict_resolver: ConflictResolver::new(client.clone()),
            client,
            _config: config,
        }
    }

    pub fn from_env() -> Self {
        Self::new(VesselConfig::from_env())
    }

    fn resolve_worktree_path(&self, pr_info: &PrInfo) -> Option<PathBuf> {
        let workspace_root = std::env::var(ENV_WORKSPACE_ROOT).ok()?;
        let branch = &pr_info.head_branch;
        let parts: Vec<&str> = branch.splitn(2, '/').collect();
        if parts.len() != 2 {
            return None;
        }
        let pair_id = parts[0].strip_prefix("forge-").unwrap_or(parts[0]);
        let ticket_id = parts[1];
        Some(PathBuf::from(workspace_root)
            .join("worktrees")
            .join(format!("{}-{}", pair_id, ticket_id)))
    }
}

#[async_trait]
impl Node for VesselNode {
    fn name(&self) -> &str {
        "vessel"
    }

    /// Phase 1: Read pending PRs and CI readiness from SharedStore.
    async fn prep(&self, store: &SharedStore) -> Result<Value> {
        debug!("VESSEL prep: reading pending PRs and CI readiness");

        let repository: Option<String> = store.get_typed("repository").await;
        let pending_prs: Option<Vec<Value>> = store.get_typed("pending_prs").await;
        let ci_readiness: Option<crate::types::CiReadiness> = store.get_typed("ci_readiness").await;

        let (owner, repo) = parse_repository(repository.as_deref());

        let has_ci_workflows = match ci_readiness {
            Some(crate::types::CiReadiness::Ready) => true,
            Some(crate::types::CiReadiness::Missing) | Some(crate::types::CiReadiness::SetupInProgress) => false,
            None => {
                if !owner.is_empty() && !repo.is_empty() {
                    match self.client.has_workflows(owner, repo).await {
                        Ok(has) => has,
                        Err(_) => true,
                    }
                } else {
                    true
                }
            }
        };

        Ok(json!({
            "owner": owner,
            "repo": repo,
            "pending_prs": pending_prs.unwrap_or_default(),
            "has_ci_workflows": has_ci_workflows,
        }))
    }

    /// Phase 2: Process each pending PR (check CI readiness → poll CI → merge → return outcome).
    async fn exec(&self, prep_result: Value) -> Result<Value> {
        let owner = prep_result["owner"].as_str().unwrap_or("");
        let repo = prep_result["repo"].as_str().unwrap_or("");
        let pending_prs = prep_result["pending_prs"].as_array().cloned().unwrap_or_default();
        let has_ci_workflows = prep_result["has_ci_workflows"].as_bool().unwrap_or(true);

        if pending_prs.is_empty() {
            info!("No pending PRs to process");
            return Ok(json!({ "outcomes": [], "has_work": false }));
        }

        info!(count = pending_prs.len(), has_ci_workflows, "Processing pending PRs");

        let mut outcomes = Vec::new();

        for pr in pending_prs {
            let pr_number = pr["number"].as_u64().unwrap_or(0);

            if pr_number == 0 {
                warn!(pr = ?pr, "Skipping invalid PR entry");
                continue;
            }

            if pr["conflict_status"].as_str() == Some("awaiting_rework") {
                debug!(pr_number, "Skipping PR awaiting rework — conflict resolution in progress");
                continue;
            }

            debug!(pr_number, "Fetching PR details");
            
            let pr_info = match self.client.get_pull_request(owner, repo, pr_number).await {
                Ok(info) => info,
                Err(e) => {
                    warn!(pr_number, error = %e, "Failed to fetch PR details, skipping");
                    continue;
                }
            };

            let outcome = if !has_ci_workflows {
                warn!(pr_number, "No CI workflows configured — treating as success and alerting NEXUS");
                self.merge_without_ci(owner, repo, pr_info).await?
            } else {
                self.process_single_pr(owner, repo, pr_info).await?
            };
            outcomes.push(outcome);
        }

        Ok(json!({
            "outcomes": outcomes,
            "has_work": !outcomes.is_empty(),
        }))
    }

    /// Phase 3: Emit events, update SharedStore, recycle workers, return routing action.
    async fn post(&self, store: &SharedStore, exec_result: Value) -> Result<Action> {
        let outcomes: Vec<VesselOutcome> = serde_json::from_value(exec_result["outcomes"].clone())
            .unwrap_or_default();
        let has_work = exec_result["has_work"].as_bool().unwrap_or(false);

        if !has_work {
            debug!("No PRs were processed");
            return Ok(Action::new("no_work"));
        }

        let pending_prs: Vec<Value> = store.get_typed(KEY_PENDING_PRS).await.unwrap_or_default();

        let mut any_success = false;
        let mut any_failure = false;
        let mut any_conflicts = false;

        for outcome in outcomes {
            match &outcome {
                VesselOutcome::Merged { ticket_id, pr_number, sha } => {
                    VesselNotifier::emit_ticket_merged(store, ticket_id, *pr_number, sha).await;
                    VesselNotifier::set_ticket_status_merged(store, ticket_id).await;
                    
                    self.update_ticket_status(store, ticket_id, "merged").await;
                    self.remove_from_pending_prs(store, *pr_number).await;

                    if let Some(pr) = pending_prs.iter().find(|p| p["number"].as_u64() == Some(*pr_number)) {
                        self.recycle_worker(store, pr).await;
                    }
                    
                    any_success = true;
                }
                VesselOutcome::CiFailed { ticket_id, pr_number, reason } => {
                    VesselNotifier::emit_ci_failed(store, ticket_id.as_deref(), *pr_number, reason).await;
                    let tid = ticket_id.clone().unwrap_or_else(|| format!("T-{}", pr_number));
                    self.mark_ticket_failed(store, &tid, &format!("CI failed for PR #{}", pr_number)).await;
                    self.remove_from_pending_prs(store, *pr_number).await;
                    any_failure = true;
                }
                VesselOutcome::MergeBlocked { ticket_id, pr_number, reason } => {
                    VesselNotifier::emit_merge_blocked(store, ticket_id.as_deref(), *pr_number, reason).await;
                    let tid = ticket_id.clone().unwrap_or_else(|| format!("T-{}", pr_number));
                    self.mark_ticket_failed(store, &tid, &format!("Merge blocked for PR #{}: {}", pr_number, reason)).await;
                    self.remove_from_pending_prs(store, *pr_number).await;
                    any_failure = true;
                }
                VesselOutcome::CiTimeout { ticket_id, pr_number } => {
                    VesselNotifier::emit_ci_timeout(store, ticket_id.as_deref(), *pr_number).await;
                    let tid = ticket_id.clone().unwrap_or_else(|| format!("T-{}", pr_number));
                    self.mark_ticket_failed(store, &tid, &format!("CI timed out for PR #{}", pr_number)).await;
                    self.remove_from_pending_prs(store, *pr_number).await;
                    any_failure = true;
                }
                VesselOutcome::CiMissing { ticket_id, pr_number } => {
                    VesselNotifier::emit_ci_missing(store, ticket_id.as_deref(), *pr_number).await;
                    let tid = ticket_id.clone().unwrap_or_else(|| format!("T-{}", pr_number));
                    VesselNotifier::emit_ticket_merged(store, &tid, *pr_number, "").await;
                    VesselNotifier::set_ticket_status_merged(store, &tid).await;
                    
                    self.update_ticket_status(store, &tid, "merged_no_ci").await;
                    self.remove_from_pending_prs(store, *pr_number).await;

                    if let Some(pr) = pending_prs.iter().find(|p| p["number"].as_u64() == Some(*pr_number)) {
                        self.recycle_worker(store, pr).await;
                    }
                    
                    any_success = true;
                }
                VesselOutcome::Conflicts { ticket_id, pr_number, conflicted_files } => {
                    VesselNotifier::emit_conflicts_detected(
                        store,
                        ticket_id.as_deref(),
                        *pr_number,
                        conflicted_files,
                    ).await;
                    let tid = ticket_id.clone().unwrap_or_else(|| format!("T-{}", pr_number));
                    self.mark_ticket_failed(store, &tid, &format!("Merge conflicts for PR #{}", pr_number)).await;
                    self.mark_pr_awaiting_rework(store, *pr_number).await;
                    any_conflicts = true;
                }
            }
        }

        if any_conflicts {
            Ok(Action::new(ACTION_CONFLICTS_DETECTED))
        } else if any_success {
            Ok(Action::DEPLOYED.into())
        } else if any_failure {
            Ok(Action::DEPLOY_FAILED.into())
        } else {
            Ok(Action::new("no_work"))
        }
    }
}

impl VesselNode {
    /// Process a single PR: poll CI → detect conflicts → resolve if possible → merge if green → return outcome.
    async fn process_single_pr(&self, owner: &str, repo: &str, pr_info: PrInfo) -> Result<VesselOutcome> {
        let ticket_id = pr_info.ticket_id.clone();
        let pr_number = pr_info.number;

        info!(pr_number, ticket_id = ?ticket_id, "Processing PR");

        let poll_result = self.poller.poll_until_terminal(owner, repo, &pr_info).await?;

        match poll_result {
            CiPollResult::Status(CiStatus::Success) => {
                match self.merger.merge(owner, repo, &pr_info).await {
                    Ok(result) if result.merged => Ok(VesselOutcome::Merged {
                        ticket_id: ticket_id.unwrap_or_else(|| format!("T-{}", pr_number)),
                        pr_number,
                        sha: result.sha.unwrap_or_default(),
                    }),
                    Ok(result) => Ok(VesselOutcome::MergeBlocked {
                        ticket_id,
                        pr_number,
                        reason: result.message,
                    }),
                    Err(e) => Ok(VesselOutcome::MergeBlocked {
                        ticket_id,
                        pr_number,
                        reason: e.to_string(),
                    }),
                }
            }
            CiPollResult::Status(status) => Ok(VesselOutcome::CiFailed {
                ticket_id,
                pr_number,
                reason: format!("CI status: {:?}", status),
            }),
            CiPollResult::Conflicts => {
                warn!(pr_number, "Merge conflicts detected during CI poll — attempting resolution");
                self.handle_conflicts(owner, repo, pr_info).await
            }
            CiPollResult::Timeout => {
                warn!(pr_number, "CI timed out — checking for conflicts as likely cause");
                let fresh_pr = self.client.get_pull_request(owner, repo, pr_number).await;
                if let Ok(ref info) = fresh_pr {
                    if info.has_conflicts() {
                        warn!(pr_number, "Conflicts found after timeout — treating as conflict case");
                        return self.handle_conflicts(owner, repo, fresh_pr.unwrap()).await;
                    }
                }
                match self.merger.merge(owner, repo, &pr_info).await {
                    Ok(result) if result.merged => Ok(VesselOutcome::CiMissing {
                        ticket_id,
                        pr_number,
                    }),
                    Ok(_result) => Ok(VesselOutcome::CiTimeout { ticket_id, pr_number }),
                    Err(_) => Ok(VesselOutcome::CiTimeout { ticket_id, pr_number }),
                }
            }
        }
    }

    /// Handle merge conflicts: attempt auto-resolution via GitHub API or local rebase.
    /// If resolution succeeds, re-poll CI for the updated branch.
    /// If resolution fails, return Conflicts outcome for nexus to reassign.
    async fn handle_conflicts(
        &self,
        owner: &str,
        repo: &str,
        pr_info: PrInfo,
    ) -> Result<VesselOutcome> {
        let ticket_id = pr_info.ticket_id.clone();
        let pr_number = pr_info.number;
        let worktree_path = self.resolve_worktree_path(&pr_info);

        if worktree_path.is_none() {
            info!(pr_number, "No local worktree found — attempting GitHub update-branch only");
        } else {
            info!(
                pr_number,
                path = ?worktree_path,
                "Worktree found — attempting full conflict resolution"
            );
        }

        let resolution = self
            .conflict_resolver
            .resolve(
                owner,
                repo,
                &pr_info,
                worktree_path.as_ref().map(|p| p.as_path()),
            )
            .await?;

        match resolution {
            ConflictResolution::Resolved => {
                info!(pr_number, "Conflicts resolved — re-polling CI for updated branch");
                let fresh_pr = self.client.get_pull_request(owner, repo, pr_number).await?;
                let re_poll = self.poller.poll_until_terminal(owner, repo, &fresh_pr).await?;
                match re_poll {
                    CiPollResult::Status(CiStatus::Success) => {
                        match self.merger.merge(owner, repo, &fresh_pr).await {
                            Ok(result) if result.merged => Ok(VesselOutcome::Merged {
                                ticket_id: ticket_id.unwrap_or_else(|| format!("T-{}", pr_number)),
                                pr_number,
                                sha: result.sha.unwrap_or_default(),
                            }),
                            Ok(result) => Ok(VesselOutcome::MergeBlocked {
                                ticket_id,
                                pr_number,
                                reason: result.message,
                            }),
                            Err(e) => Ok(VesselOutcome::MergeBlocked {
                                ticket_id,
                                pr_number,
                                reason: e.to_string(),
                            }),
                        }
                    }
                    CiPollResult::Status(status) => Ok(VesselOutcome::CiFailed {
                        ticket_id,
                        pr_number,
                        reason: format!("CI status after conflict resolution: {:?}", status),
                    }),
                    CiPollResult::Conflicts => {
                        warn!(pr_number, "Conflicts re-appeared after resolution — needs forge rework");
                        Ok(VesselOutcome::Conflicts {
                            ticket_id,
                            pr_number,
                            conflicted_files: vec!["re-conflict after resolution".to_string()],
                        })
                    }
                    CiPollResult::Timeout => Ok(VesselOutcome::CiTimeout { ticket_id, pr_number }),
                }
            }
            ConflictResolution::NeedsIntelligentResolution { conflicted_files } => {
                warn!(
                    pr_number,
                    files = conflicted_files.len(),
                    "Conflicts require intelligent resolution — routing to forge for rework"
                );
                if let Some(ref wt) = worktree_path {
                    let _ = ConflictResolver::abort_rebase(wt).await;
                }
                Ok(VesselOutcome::Conflicts {
                    ticket_id,
                    pr_number,
                    conflicted_files,
                })
            }
            ConflictResolution::Unresolvable { conflicted_files } => {
                warn!(pr_number, "Conflicts unresolvable — routing to forge for rework");
                Ok(VesselOutcome::Conflicts {
                    ticket_id,
                    pr_number,
                    conflicted_files,
                })
            }
        }
    }

    /// Merge a PR without CI validation (no CI workflows configured).
    /// Still attempts the merge but emits a ci_missing event to alert NEXUS.
    async fn merge_without_ci(&self, owner: &str, repo: &str, pr_info: PrInfo) -> Result<VesselOutcome> {
        let ticket_id = pr_info.ticket_id.clone();
        let pr_number = pr_info.number;

        info!(pr_number, ticket_id = ?ticket_id, "Merging PR without CI — no workflows configured");

        match self.merger.merge(owner, repo, &pr_info).await {
            Ok(result) if result.merged => Ok(VesselOutcome::CiMissing {
                ticket_id,
                pr_number,
            }),
            Ok(result) => Ok(VesselOutcome::MergeBlocked {
                ticket_id,
                pr_number,
                reason: result.message,
            }),
            Err(e) => Ok(VesselOutcome::MergeBlocked {
                ticket_id,
                pr_number,
                reason: e.to_string(),
            }),
        }
    }

    /// Update ticket status in SharedStore.
    async fn update_ticket_status(&self, store: &SharedStore, ticket_id: &str, status: &str) {
        let mut tickets: Vec<Value> = store.get_typed("tickets").await.unwrap_or_default();
        
        for ticket in tickets.iter_mut() {
            if ticket["id"].as_str() == Some(ticket_id) {
                ticket["status"] = json!({ "type": status });
                break;
            }
        }
        
        store.set(KEY_TICKETS, json!(tickets)).await;
    }

    async fn mark_ticket_failed(&self, store: &SharedStore, ticket_id: &str, reason: &str) {
        let mut tickets: Vec<Ticket> = store.get_typed(KEY_TICKETS).await.unwrap_or_default();

        for ticket in tickets.iter_mut() {
            if ticket.id == ticket_id {
                let attempts = ticket.attempts + 1;
                ticket.attempts = attempts;
                ticket.status = TicketStatus::Failed {
                    worker_id: String::from("vessel"),
                    reason: reason.to_string(),
                    attempts,
                };
                break;
            }
        }

        store.set(KEY_TICKETS, json!(tickets)).await;
    }

    /// Remove PR from pending_prs list.
    async fn remove_from_pending_prs(&self, store: &SharedStore, pr_number: u64) {
        let mut pending: Vec<Value> = store.get_typed("pending_prs").await.unwrap_or_default();
        pending.retain(|pr| pr["number"].as_u64() != Some(pr_number));
        store.set("pending_prs", json!(pending)).await;
    }

    /// Mark a PR as awaiting rework so vessel skips it on subsequent cycles
    /// and nexus sync_open_prs doesn't re-add it blindly.
    async fn mark_pr_awaiting_rework(&self, store: &SharedStore, pr_number: u64) {
        let mut pending: Vec<Value> = store.get_typed("pending_prs").await.unwrap_or_default();
        for pr in pending.iter_mut() {
            if pr["number"].as_u64() == Some(pr_number) {
                pr["conflict_status"] = json!("awaiting_rework");
                break;
            }
        }
        store.set("pending_prs", json!(pending)).await;
    }

    /// Recycle a worker from Done back to Idle after its PR is merged.
    async fn recycle_worker(&self, store: &SharedStore, pr: &Value) {
        let worker_id = pr["worker_id"].as_str().unwrap_or("");
        if worker_id.is_empty() {
            return;
        }

        let mut slots: HashMap<String, WorkerSlot> =
            store.get_typed(KEY_WORKER_SLOTS).await.unwrap_or_default();

        if let Some(slot) = slots.get_mut(worker_id) {
            match &slot.status {
                WorkerStatus::Done { .. } => {
                    info!(worker_id, "Recycling worker from Done to Idle after merge");
                    slot.status = WorkerStatus::Idle;
                    store.set(KEY_WORKER_SLOTS, json!(slots)).await;
                }
                other => {
                    debug!(worker_id, status = ?other, "Worker not in Done state, skipping recycle");
                }
            }
        }
    }

    /// Reconcile startup: check for PRs that are already merged on GitHub.
    pub async fn reconcile(&self, store: &SharedStore) -> Result<()> {
        info!("Running VESSEL startup reconciliation");

        let repository: Option<String> = store.get_typed("repository").await;
        let pending_prs: Option<Vec<Value>> = store.get_typed("pending_prs").await;
        let (owner, repo) = parse_repository(repository.as_deref());

        let pending = pending_prs.unwrap_or_default();

        for pr in pending {
            let pr_number = pr["number"].as_u64().unwrap_or(0);
            if pr_number == 0 {
                continue;
            }

            if self.client.is_pr_merged(owner, repo, pr_number).await? {
                warn!(pr_number, "Found already-merged PR during reconciliation");
                
                let ticket_id = pr["ticket_id"].as_str().map(String::from);
                let pr_info = self.client.get_pull_request(owner, repo, pr_number).await;
                
                if let Ok(info) = pr_info {
                    let tid = ticket_id.or(info.ticket_id).unwrap_or_else(|| format!("T-{}", pr_number));
                    VesselNotifier::emit_ticket_merged(store, &tid, pr_number, &info.head_sha).await;
                    VesselNotifier::set_ticket_status_merged(store, &tid).await;
                    self.remove_from_pending_prs(store, pr_number).await;
                }
            }
        }

        Ok(())
    }
}

fn parse_repository(repository: Option<&str>) -> (&str, &str) {
    match repository {
        Some(repo) => {
            let parts: Vec<&str> = repo.split('/').collect();
            if parts.len() == 2 {
                (parts[0], parts[1])
            } else {
                ("", "")
            }
        }
        None => ("", ""),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_repository() {
        assert_eq!(parse_repository(Some("owner/repo")), ("owner", "repo"));
        assert_eq!(parse_repository(Some("single")), ("", ""));
        assert_eq!(parse_repository(None), ("", ""));
    }

    #[tokio::test]
    async fn test_prep_reads_pending_prs() {
        let store = SharedStore::new_in_memory();
        store.set("repository", json!("test-owner/test-repo")).await;
        store.set("pending_prs", json!([
            {"number": 1, "ticket_id": "T-1"},
            {"number": 2, "ticket_id": "T-2"},
        ])).await;

        let config = VesselConfig::default();
        let node = VesselNode::new(config);

        let result = node.prep(&store).await.unwrap();
        
        assert_eq!(result["owner"], "test-owner");
        assert_eq!(result["repo"], "test-repo");
        assert_eq!(result["pending_prs"].as_array().unwrap().len(), 2);
    }

    #[tokio::test]
    async fn test_prep_empty_pending_prs() {
        let store = SharedStore::new_in_memory();

        let config = VesselConfig::default();
        let node = VesselNode::new(config);

        let result = node.prep(&store).await.unwrap();
        
        assert_eq!(result["pending_prs"].as_array().unwrap().len(), 0);
    }

    #[tokio::test]
    async fn test_post_handles_merged_outcome() {
        let store = SharedStore::new_in_memory();
        store.set("pending_prs", json!([{"number": 42, "ticket_id": "T-42"}])).await;
        store.set("tickets", json!([{"id": "T-42", "status": {"type": "in_progress"}}])).await;

        let config = VesselConfig::default();
        let node = VesselNode::new(config);

        let exec_result = json!({
            "outcomes": [VesselOutcome::Merged {
                ticket_id: "T-42".to_string(),
                pr_number: 42,
                sha: "abc123".to_string(),
            }],
            "has_work": true,
        });

        let action = node.post(&store, exec_result).await.unwrap();
        assert_eq!(action.as_str(), Action::DEPLOYED);

        let events = store.get_events_since(0).await;
        assert!(events.iter().any(|e| e.event_type == "ticket_merged"));

        let status = store.get("ticket:T-42:status").await;
        assert_eq!(status, Some(json!("Merged")));

        let pending: Vec<Value> = store.get_typed("pending_prs").await.unwrap_or_default();
        assert!(pending.is_empty());
    }

    #[tokio::test]
    async fn test_post_handles_ci_failed_outcome() {
        let store = SharedStore::new_in_memory();

        let config = VesselConfig::default();
        let node = VesselNode::new(config);

        let exec_result = json!({
            "outcomes": [VesselOutcome::CiFailed {
                ticket_id: Some("T-42".to_string()),
                pr_number: 42,
                reason: "Tests failed".to_string(),
            }],
            "has_work": true,
        });

        let action = node.post(&store, exec_result).await.unwrap();
        assert_eq!(action.as_str(), Action::DEPLOY_FAILED);

        let events = store.get_events_since(0).await;
        assert!(events.iter().any(|e| e.event_type == "ci_failed"));
    }

    #[tokio::test]
    async fn test_post_handles_merge_blocked_outcome() {
        let store = SharedStore::new_in_memory();

        let config = VesselConfig::default();
        let node = VesselNode::new(config);

        let exec_result = json!({
            "outcomes": [VesselOutcome::MergeBlocked {
                ticket_id: Some("T-42".to_string()),
                pr_number: 42,
                reason: "Merge conflict".to_string(),
            }],
            "has_work": true,
        });

        let action = node.post(&store, exec_result).await.unwrap();
        assert_eq!(action.as_str(), Action::DEPLOY_FAILED);

        let events = store.get_events_since(0).await;
        assert!(events.iter().any(|e| e.event_type == "merge_blocked"));
    }

    #[tokio::test]
    async fn test_post_handles_no_work() {
        let store = SharedStore::new_in_memory();

        let config = VesselConfig::default();
        let node = VesselNode::new(config);

        let exec_result = json!({
            "outcomes": [],
            "has_work": false,
        });

        let action = node.post(&store, exec_result).await.unwrap();
        assert_eq!(action.as_str(), "no_work");
    }

    #[tokio::test]
    async fn test_update_ticket_status() {
        let store = SharedStore::new_in_memory();
        store.set("tickets", json!([
            {"id": "T-1", "status": {"type": "open"}},
            {"id": "T-42", "status": {"type": "in_progress"}},
        ])).await;

        let config = VesselConfig::default();
        let node = VesselNode::new(config);

        node.update_ticket_status(&store, "T-42", "merged").await;

        let tickets: Vec<Value> = store.get_typed("tickets").await.unwrap();
        let ticket = tickets.iter().find(|t| t["id"] == "T-42").unwrap();
        assert_eq!(ticket["status"]["type"], "merged");
    }

    #[tokio::test]
    async fn test_remove_from_pending_prs() {
        let store = SharedStore::new_in_memory();
        store.set("pending_prs", json!([
            {"number": 1, "ticket_id": "T-1"},
            {"number": 42, "ticket_id": "T-42"},
            {"number": 100, "ticket_id": "T-100"},
        ])).await;

        let config = VesselConfig::default();
        let node = VesselNode::new(config);

        node.remove_from_pending_prs(&store, 42).await;

        let pending: Vec<Value> = store.get_typed("pending_prs").await.unwrap();
        assert_eq!(pending.len(), 2);
        assert!(pending.iter().all(|pr| pr["number"] != 42));
    }
}
