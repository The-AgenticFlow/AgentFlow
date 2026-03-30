// crates/agent-forge/src/lib.rs
use anyhow::{anyhow, Result};
use async_trait::async_trait;
use config::{
    state::{ACTION_EMPTY, ACTION_FAILED, ACTION_PR_OPENED, KEY_COMMAND_GATE, KEY_WORKER_SLOTS},
    WorkerSlot, WorkerStatus,
};
use pair_harness::worktree::WorktreeManager;
use pocketflow_core::{Action, BatchNode, SharedStore};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::HashMap;
use std::path::PathBuf;
use tracing::{info, warn};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ForgeStatus {
    pub outcome: String,
    pub ticket_id: String,
    pub branch: String,
    pub pr_url: Option<String>,
    pub pr_number: Option<u32>,
    pub notes: Option<String>,
}

pub struct ForgeNode {
    pub workspace_root: PathBuf,
    pub persona_path: PathBuf,
}

impl ForgeNode {
    pub fn new(workspace_root: impl Into<PathBuf>, persona_path: impl Into<PathBuf>) -> Self {
        Self {
            workspace_root: workspace_root.into(),
            persona_path: persona_path.into(),
        }
    }

    async fn load_persona(&self) -> Result<String> {
        let content = tokio::fs::read_to_string(&self.persona_path).await
            .map_err(|e| anyhow!("Failed to load forge persona from {:?}: {}", self.persona_path, e))?;
        Ok(content)
    }
}

#[async_trait]
impl BatchNode for ForgeNode {
    fn name(&self) -> &str {
        "forge"
    }

    async fn prep_batch(&self, store: &SharedStore) -> Result<Vec<Value>> {
        let slots: HashMap<String, WorkerSlot> =
            store.get_typed(KEY_WORKER_SLOTS).await.unwrap_or_default();

        let active_workers: Vec<Value> = slots
            .values()
            .filter(|s| {
                matches!(
                    s.status,
                    WorkerStatus::Assigned { .. } | WorkerStatus::Working { .. }
                )
            })
            .map(|s| json!(s))
            .collect();

        Ok(active_workers)
    }

    async fn exec_one(&self, item: Value) -> Result<Value> {
        let slot: WorkerSlot = serde_json::from_value(item)?;
        let worker_id = slot.id.clone();

        let (ticket_id, issue_url) = match &slot.status {
            WorkerStatus::Assigned {
                ticket_id,
                issue_url,
            } => (ticket_id.clone(), issue_url.clone()),
            WorkerStatus::Working {
                ticket_id,
                issue_url,
            } => (ticket_id.clone(), issue_url.clone()),
            _ => return Ok(json!({"outcome": "idle", "worker_id": worker_id})),
        };

        // Create worktree manager
        let worktree_mgr = WorktreeManager::new(&self.workspace_root);
        
        // Create worktree for this worker
        let worktree_path = worktree_mgr
            .create_worktree(&worker_id, &ticket_id)
            .map_err(|e| anyhow!("Failed to create worktree: {}", e))?;

        info!(worker = worker_id, ticket = ticket_id, path = ?worktree_path, "Worktree created");

        // Create log directory to persist logs even after worktree cleanup
        let log_dir = self.workspace_root.join("forge").join("workers").join(&worker_id);
        tokio::fs::create_dir_all(&log_dir).await?;
        
        let status_path = worktree_path.join("STATUS.json");
        let log_path = log_dir.join("worker.log");
        let log_file = std::fs::File::create(&log_path)?;
        let log_file_err = log_file.try_clone()?;

        info!(worker = worker_id, ticket = ticket_id, issue_url = ?issue_url, "Spawning Claude Code...");

        // Load the persona from the agent definition file (source of truth)
        let persona_content = self.load_persona().await?;

        // 1. Prepare command - build prompt from persona + task context
        let issue_context = if let Some(url) = &issue_url {
            format!("Issue URL: {}. Use your MCP tools (e.g. `get_issue` or `read_url`) to fetch the full description.", url)
        } else {
            "".to_string()
        };

        let branch_name = WorktreeManager::branch_name(&worker_id, &ticket_id);
        
        // Combine persona with task-specific context
        let prompt = format!(
            "{}\n\n---\n\n# Current Task\n\nYou are FORGE agent {} (worker slot).\nImplement ticket {}.\n{}\nBranch: {}.\nWhen done, open a PR and write STATUS.json.",
            persona_content, worker_id, ticket_id, issue_context, branch_name
        );

        // Use CLI flags to grant permissions
        // Note: When using --allowedTools with comma-separated values, Claude Code
        // doesn't properly recognize the prompt as a positional argument.
        // We must pass the prompt via stdin instead.
        let mut child = tokio::process::Command::new("claude")
            .args(["--print", "--output-format", "json"])
            .args(["--permission-mode", "auto"])
            .args(["--allowedTools", "Read,Write,Edit,Bash,WebFetch"])
            .current_dir(&worktree_path)
            .env(
                "ANTHROPIC_API_KEY",
                std::env::var("ANTHROPIC_API_KEY").unwrap_or_default(),
            )
            .stdin(std::process::Stdio::piped())
            .stdout(log_file)
            .stderr(log_file_err)
            .spawn()
            .map_err(|e| anyhow!("Failed to spawn Claude Code: {}", e))?;

        // Write prompt to stdin
        if let Some(mut stdin) = child.stdin.take() {
            use tokio::io::AsyncWriteExt;
            stdin.write_all(prompt.as_bytes()).await
                .map_err(|e| anyhow!("Failed to write prompt to stdin: {}", e))?;
        }

        // MONITORING: Since we redirected stdout/stderr to a file, we can't easily
        // monitor for "Dangerous command" strings in real-time within this process
        // without tailing the file. For now, we'll let it run and check the STATUS.json
        // or the log file afterwards.

        let timeout_dur = std::time::Duration::from_secs(1800); // 30 minutes

        // 2. Wait for process
        let result = tokio::time::timeout(timeout_dur, child.wait()).await;

        match result {
            Err(_) => {
                child.kill().await?;
                warn!(worker = worker_id, "Claude Code timed out after 30m");
                return Ok(json!({
                    "worker_id": worker_id,
                    "ticket_id": ticket_id,
                    "outcome": "fuel_exhausted",
                    "reason": "timeout"
                }));
            }
            Ok(Ok(status)) if !status.success() => {
                warn!(worker = worker_id, exit = ?status.code(), "Claude Code failed");
            }
            _ => {}
        }

        // 3. Read STATUS.json
        if tokio::fs::try_exists(&status_path).await? {
            let content = tokio::fs::read_to_string(&status_path).await?;
            let forge_status: ForgeStatus = serde_json::from_str(&content)?;
            return Ok(json!({
                "worker_id": worker_id,
                "ticket_id": ticket_id,
                "outcome": forge_status.outcome,
                "branch": forge_status.branch,
                "pr_url": forge_status.pr_url,
                "pr_number": forge_status.pr_number,
                "notes": forge_status.notes,
            }));
        }

        Ok(json!({
            "worker_id": worker_id,
            "ticket_id": ticket_id,
            "outcome": "failed",
            "reason": "STATUS.json not written"
        }))
    }

    async fn post_batch(&self, store: &SharedStore, results: Vec<Result<Value>>) -> Result<Action> {
        let mut slots: HashMap<String, WorkerSlot> =
            store.get_typed(KEY_WORKER_SLOTS).await.unwrap_or_default();

        let mut command_gate: HashMap<String, Value> =
            store.get_typed(KEY_COMMAND_GATE).await.unwrap_or_default();

        let mut all_success = true;
        let worktree_mgr = WorktreeManager::new(&self.workspace_root);

        for res_opt in &results {
            let res = match res_opt {
                Ok(v) => v,
                Err(e) => {
                    warn!("Batch item failed: {}", e);
                    all_success = false;
                    continue;
                }
            };
            let worker_id = res["worker_id"].as_str().unwrap_or("");
            let ticket_id = res["ticket_id"].as_str().unwrap_or("");
            let outcome = res["outcome"].as_str().unwrap_or("failed");

            if let Some(slot) = slots.get_mut(worker_id) {
                match outcome {
                    "pr_opened" => {
                        info!(
                            worker = worker_id,
                            ticket = ticket_id,
                            "Work completed successfully"
                        );
                        slot.status = WorkerStatus::Done {
                            ticket_id: ticket_id.to_string(),
                            outcome: outcome.to_string(),
                        };
                        // Cleanup worktree for completed work
                        if let Err(e) = worktree_mgr.remove_worktree(worker_id) {
                            warn!(worker = worker_id, error = %e, "Failed to cleanup worktree");
                        } else {
                            info!(worker = worker_id, "Worktree cleaned up");
                        }
                    }
                    "suspended" => {
                        let reason = res["reason"].as_str().unwrap_or("unknown");
                        info!(
                            worker = worker_id,
                            ticket = ticket_id,
                            reason,
                            "Work suspended for approval"
                        );
                        slot.status = WorkerStatus::Suspended {
                            ticket_id: ticket_id.to_string(),
                            reason: reason.to_string(),
                            issue_url: res["issue_url"].as_str().map(|s| s.to_string()),
                        };
                        // Push to command gate
                        command_gate.insert(worker_id.to_string(), res.clone());
                    }
                    "idle" => {}
                    _ => {
                        warn!(
                            worker = worker_id,
                            ticket = ticket_id,
                            outcome,
                            "Work failed"
                        );
                        slot.status = WorkerStatus::Idle;
                        all_success = false;
                        // Cleanup worktree for failed work
                        if let Err(e) = worktree_mgr.remove_worktree(worker_id) {
                            warn!(worker = worker_id, error = %e, "Failed to cleanup worktree");
                        } else {
                            info!(worker = worker_id, "Worktree cleaned up");
                        }
                    }
                }
            }
        }

        store.set(KEY_WORKER_SLOTS, json!(slots)).await;
        store.set(KEY_COMMAND_GATE, json!(command_gate)).await;

        let has_suspended = slots
            .values()
            .any(|s| matches!(s.status, WorkerStatus::Suspended { .. }));

        if has_suspended {
            Ok(Action::new("suspended"))
        } else if all_success && !results.is_empty() {
            Ok(Action::new(ACTION_PR_OPENED))
        } else if results.is_empty() {
            Ok(Action::new(ACTION_EMPTY))
        } else {
            Ok(Action::new(ACTION_FAILED))
        }
    }
}
