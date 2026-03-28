// crates/agent-forge/src/lib.rs
use anyhow::{anyhow, Result};
use async_trait::async_trait;
use config::{
    state::{ACTION_EMPTY, ACTION_FAILED, ACTION_PR_OPENED, KEY_COMMAND_GATE, KEY_WORKER_SLOTS},
    WorkerSlot, WorkerStatus,
};
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
}

impl ForgeNode {
    pub fn new(workspace_root: impl Into<PathBuf>) -> Self {
        Self {
            workspace_root: workspace_root.into(),
        }
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

        // Ensure worker artifact dir exists (for logs and STATUS.json)
        let worker_dir = self
            .workspace_root
            .join("forge")
            .join("workers")
            .join(&worker_id);
        if !worker_dir.exists() {
            tokio::fs::create_dir_all(&worker_dir).await?;
        }
        let status_path = worker_dir.join("STATUS.json");
        if tokio::fs::try_exists(&status_path).await? {
            tokio::fs::remove_file(&status_path).await?;
        }

        let log_path = worker_dir.join("worker.log");
        let log_file = std::fs::File::create(&log_path)?;
        let log_file_err = log_file.try_clone()?;

        // Determine working directory - use workspace root for Claude Code
        // Claude Code should work from the project root, not the artifact directory
        let working_dir = &self.workspace_root;

        info!(worker = worker_id, ticket = ticket_id, issue_url = ?issue_url, working_dir = %working_dir.display(), "Spawning Claude Code...");

        // 1. Prepare command
        let issue_context = if let Some(url) = &issue_url {
            format!("Issue URL: {}. Use your MCP tools (e.g. `get_issue` or `read_url`) to fetch the full description.", url)
        } else {
            "".to_string()
        };

        let prompt = format!(
            "You are FORGE agent {}. \
             Implement ticket {}. \
             {} \
             Branch: forge/{}/{}. \
             Write STATUS.json to {} when done.",
            worker_id,
            ticket_id,
            issue_context,
            worker_id,
            ticket_id,
            status_path.display()
        );

        let mut child = tokio::process::Command::new("claude")
            .args(["--print", "--output-format", "json"])
            .arg(&prompt)
            .current_dir(working_dir) // Work from workspace root, not artifact directory
            .env(
                "ANTHROPIC_API_KEY",
                std::env::var("ANTHROPIC_API_KEY").unwrap_or_default(),
            )
            .env("FORGE_WORKER_ID", &worker_id)
            .env("FORGE_ARTIFACT_DIR", worker_dir.display().to_string())
            .stdout(log_file)
            .stderr(log_file_err)
            .spawn()
            .map_err(|e| anyhow!("Failed to spawn Claude Code: {}", e))?;

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

        let log_output = tokio::fs::read_to_string(&log_path)
            .await
            .unwrap_or_default();
        if contains_dangerous_command_prompt(&log_output) {
            return Ok(json!({
                "worker_id": worker_id,
                "ticket_id": ticket_id,
                "issue_url": issue_url,
                "outcome": "suspended",
                "reason": "dangerous_command"
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

fn contains_dangerous_command_prompt(log_output: &str) -> bool {
    let log_output = log_output.to_ascii_lowercase();
    log_output.contains("dangerous command") || log_output.contains("run this command? (y/n)")
}
