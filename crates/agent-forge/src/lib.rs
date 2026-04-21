// crates/agent-forge/src/lib.rs
use anyhow::{anyhow, Result};
use async_trait::async_trait;
use config::{
    state::{
        ACTION_EMPTY, ACTION_FAILED, ACTION_PR_OPENED, KEY_COMMAND_GATE, KEY_PENDING_PRS,
        KEY_TICKETS, KEY_WORKER_SLOTS,
    },
    Ticket, TicketStatus, WorkerSlot, WorkerStatus,
};
use pair_harness::{
    worktree::WorktreeManager, ForgeSentinelPair, PairConfig, PairOutcome, Ticket as PairTicket,
};
use pocketflow_core::{Action, BatchNode, SharedStore};
use reqwest::header::{HeaderMap, HeaderValue, ACCEPT, AUTHORIZATION, USER_AGENT};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::HashMap;
use std::path::PathBuf;
use tracing::{info, warn};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ForgeStatus {
    /// Outcome status - can be "outcome" or "status" in STATUS.json
    #[serde(alias = "status")]
    pub outcome: String,
    /// Ticket ID - can be "ticket" or "ticket_id" in STATUS.json
    /// FORGE may omit this field; fall back to the known ticket_id.
    #[serde(alias = "ticket", default)]
    pub ticket_id: Option<String>,
    /// Branch name (optional - may not be present in all STATUS.json formats)
    #[serde(default)]
    pub branch: Option<String>,
    /// PR URL if a PR was opened
    #[serde(alias = "pr")]
    pub pr_url: Option<String>,
    /// PR number if a PR was opened
    pub pr_number: Option<u32>,
    /// Notes about the work done
    pub notes: Option<String>,
    /// Summary of changes (optional)
    pub summary: Option<String>,
    /// List of changes made (optional)
    #[serde(default)]
    pub changes: Option<Vec<String>>,
    /// List of commits made (optional)
    #[serde(default)]
    pub commits: Option<Vec<String>>,
    /// List of artifacts created (optional)
    #[serde(default)]
    pub artifacts: Option<Vec<String>>,
    /// Issue URL (optional)
    pub issue: Option<String>,
    /// Reason for failure or suspension (optional)
    pub reason: Option<String>,
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
        let content = tokio::fs::read_to_string(&self.persona_path)
            .await
            .map_err(|e| {
                anyhow!(
                    "Failed to load forge persona from {:?}: {}",
                    self.persona_path,
                    e
                )
            })?;
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
            .map_err(|e| anyhow!("Failed to create worktree: {:#}", e))?;

        info!(worker = worker_id, ticket = ticket_id, path = ?worktree_path, "Worktree created");

        // Create log directory to persist logs even after worktree cleanup
        let log_dir = self
            .workspace_root
            .join("forge")
            .join("workers")
            .join(&worker_id);
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
            .arg("--dangerously-skip-permissions")
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
            .map_err(|e| anyhow!("Failed to spawn Claude Code: {:#}", e))?;

        // Write prompt to stdin
        if let Some(mut stdin) = child.stdin.take() {
            use tokio::io::AsyncWriteExt;
            stdin
                .write_all(prompt.as_bytes())
                .await
                .map_err(|e| anyhow!("Failed to write prompt to stdin: {:#}", e))?;
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
            match serde_json::from_str::<ForgeStatus>(&content) {
                Ok(forge_status) => {
                    let outcome = match forge_status.outcome.as_str() {
                        "complete" | "completed" => "success",
                        other => other,
                    };

                    return Ok(json!({
                        "worker_id": worker_id,
                        "ticket_id": ticket_id,
                        "outcome": outcome,
                        "branch": forge_status.branch,
                        "pr_url": forge_status.pr_url,
                        "pr_number": forge_status.pr_number,
                        "notes": forge_status.notes,
                        "summary": forge_status.summary,
                        "commits": forge_status.commits,
                        "artifacts": forge_status.artifacts,
                        "reason": forge_status.reason,
                    }));
                }
                Err(e) => {
                    warn!(error = %e, "Failed to parse STATUS.json - treating as missing");
                }
            }
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

        let mut tickets: Vec<Ticket> = store.get_typed(KEY_TICKETS).await.unwrap_or_default();

        let mut all_success = true;
        let worktree_mgr = WorktreeManager::new(&self.workspace_root);

        let mut ticket_updates: Vec<(String, TicketStatus)> = Vec::new();
        let mut opened_prs: Vec<Value> = Vec::new();

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
                    "success" | "pr_opened" => {
                        info!(
                            worker = worker_id,
                            ticket = ticket_id,
                            outcome,
                            "Work completed successfully"
                        );
                        slot.status = WorkerStatus::Done {
                            ticket_id: ticket_id.to_string(),
                            outcome: outcome.to_string(),
                        };
                        ticket_updates.push((
                            ticket_id.to_string(),
                            TicketStatus::Completed {
                                worker_id: worker_id.to_string(),
                                outcome: outcome.to_string(),
                            },
                        ));

                        let pr_number = res["pr_number"].as_u64().unwrap_or(0);
                        let branch = res["branch"].as_str().unwrap_or("");
                        if pr_number > 0 {
                            opened_prs.push(json!({
                                "number": pr_number,
                                "ticket_id": ticket_id,
                                "branch": branch,
                                "worker_id": worker_id,
                            }));
                        }

                        if let Err(e) =
                            worktree_mgr.remove_worktree_for_ticket(worker_id, ticket_id)
                        {
                            warn!(worker = worker_id, error = %e, "Failed to cleanup worktree");
                        } else {
                            info!(worker = worker_id, "Worktree cleaned up");
                        }
                    }
                    "suspended" | "blocked" => {
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
                        let prev_attempts = tickets
                            .iter()
                            .find(|t| t.id == ticket_id)
                            .map(|t| t.attempts)
                            .unwrap_or(0)
                            + 1;
                        if prev_attempts >= Ticket::MAX_ATTEMPTS {
                            ticket_updates.push((
                                ticket_id.to_string(),
                                TicketStatus::Exhausted {
                                    worker_id: worker_id.to_string(),
                                    attempts: prev_attempts,
                                },
                            ));
                        } else {
                            ticket_updates.push((
                                ticket_id.to_string(),
                                TicketStatus::Failed {
                                    worker_id: worker_id.to_string(),
                                    reason: outcome.to_string(),
                                    attempts: prev_attempts,
                                },
                            ));
                        }
                        if let Err(e) =
                            worktree_mgr.remove_worktree_for_ticket(worker_id, ticket_id)
                        {
                            warn!(worker = worker_id, error = %e, "Failed to cleanup worktree");
                        } else {
                            info!(worker = worker_id, "Worktree cleaned up");
                        }
                    }
                }
            }
        }

        for (ticket_id, new_status) in ticket_updates {
            if let Some(ticket) = tickets.iter_mut().find(|t| t.id == ticket_id) {
                if let TicketStatus::Failed { attempts, .. } = &new_status {
                    ticket.attempts = *attempts;
                } else if let TicketStatus::Exhausted { attempts, .. } = &new_status {
                    ticket.attempts = *attempts;
                }
                ticket.status = new_status;
            }
        }

        store.set(KEY_WORKER_SLOTS, json!(slots)).await;
        store.set(KEY_COMMAND_GATE, json!(command_gate)).await;
        store.set(KEY_TICKETS, json!(tickets)).await;

        let has_prs = !opened_prs.is_empty();
        if has_prs {
            let mut pending_prs: Vec<Value> =
                store.get_typed(KEY_PENDING_PRS).await.unwrap_or_default();
            pending_prs.extend(opened_prs);
            store.set(KEY_PENDING_PRS, json!(pending_prs)).await;
            info!("Updated pending_prs for VESSEL processing");
        }

        let has_suspended = slots
            .values()
            .any(|s| matches!(s.status, WorkerStatus::Suspended { .. }));

        if has_suspended {
            Ok(Action::new("suspended"))
        } else if (has_prs || all_success) && !results.is_empty() {
            Ok(Action::new(ACTION_PR_OPENED))
        } else if results.is_empty() {
            Ok(Action::new(ACTION_EMPTY))
        } else {
            Ok(Action::new(ACTION_FAILED))
        }
    }
}

/// ForgePairNode - integrates the full event-driven FORGE-SENTINEL lifecycle.
///
/// This node uses the ForgeSentinelPair from pair-harness to manage:
/// - FORGE as a long-running process
/// - SENTINEL spawned ephemeral for evaluations
/// - Event-driven lifecycle based on filesystem watches
/// - Automatic context resets via HANDOFF.md
///
/// Uses filesystem-based state by default (no Redis required).
pub struct ForgePairNode {
    pub workspace_root: PathBuf,
    pub github_token: String,
}

#[derive(Debug, Deserialize)]
struct GithubIssue {
    number: u64,
    title: String,
    #[serde(default)]
    body: String,
    html_url: String,
}

impl ForgePairNode {
    /// Create a new ForgePairNode with filesystem-based state.
    pub fn new(workspace_root: impl Into<PathBuf>, github_token: impl Into<String>) -> Self {
        Self {
            workspace_root: workspace_root.into(),
            github_token: github_token.into(),
        }
    }

    fn parse_github_issue_url(issue_url: &str) -> Option<(String, String, u64)> {
        let trimmed = issue_url.trim_end_matches('/');
        let parts: Vec<_> = trimmed.split('/').collect();
        let issue_idx = parts.iter().position(|part| *part == "issues")?;
        if issue_idx < 2 || issue_idx + 1 >= parts.len() {
            return None;
        }

        let owner = parts.get(issue_idx - 2)?.to_string();
        let repo = parts.get(issue_idx - 1)?.to_string();
        let number = parts.get(issue_idx + 1)?.parse().ok()?;

        Some((owner, repo, number))
    }

    fn extract_acceptance_criteria(body: &str) -> Vec<String> {
        fn normalize_bullet(line: &str) -> Option<String> {
            let trimmed = line.trim();
            let stripped = trimmed
                .strip_prefix("- [ ] ")
                .or_else(|| trimmed.strip_prefix("- [x] "))
                .or_else(|| trimmed.strip_prefix("- "))
                .or_else(|| trimmed.strip_prefix("* "))
                .or_else(|| trimmed.strip_prefix("1. "))
                .or_else(|| trimmed.strip_prefix("2. "))
                .or_else(|| trimmed.strip_prefix("3. "))
                .or_else(|| trimmed.strip_prefix("4. "))
                .or_else(|| trimmed.strip_prefix("5. "))?;
            let value = stripped.trim();
            if value.is_empty() {
                None
            } else {
                Some(value.to_string())
            }
        }

        let mut in_acceptance_section = false;
        let mut criteria = Vec::new();

        for line in body.lines() {
            let trimmed = line.trim();
            let lower = trimmed.to_ascii_lowercase();

            if trimmed.starts_with('#') {
                in_acceptance_section = lower.contains("acceptance criteria");
                continue;
            }

            if in_acceptance_section {
                if let Some(item) = normalize_bullet(trimmed) {
                    criteria.push(item);
                    continue;
                }

                if !trimmed.is_empty() {
                    in_acceptance_section = false;
                }
            }
        }

        if criteria.is_empty() {
            for line in body.lines() {
                if let Some(item) = normalize_bullet(line) {
                    criteria.push(item);
                }
            }
        }

        criteria.dedup();
        criteria
    }

    async fn fetch_issue(&self, owner: &str, repo: &str, number: u64) -> Result<GithubIssue> {
        let mut headers = HeaderMap::new();
        headers.insert(USER_AGENT, HeaderValue::from_static("agentflow/forge"));
        headers.insert(
            ACCEPT,
            HeaderValue::from_static("application/vnd.github+json"),
        );
        headers.insert(
            AUTHORIZATION,
            HeaderValue::from_str(&format!("Bearer {}", self.github_token))?,
        );

        let client = reqwest::Client::builder()
            .default_headers(headers)
            .build()?;

        let response = client
            .get(format!(
                "https://api.github.com/repos/{owner}/{repo}/issues/{number}"
            ))
            .send()
            .await?;

        let response = response.error_for_status()?;
        Ok(response.json::<GithubIssue>().await?)
    }

    async fn build_ticket(&self, ticket_id: &str, issue_url: Option<&str>) -> PairTicket {
        let mut ticket = PairTicket {
            id: ticket_id.to_string(),
            issue_number: 0,
            title: format!("Ticket {}", ticket_id),
            body: issue_url.unwrap_or_default().to_string(),
            url: issue_url.unwrap_or_default().to_string(),
            touched_files: vec![],
            acceptance_criteria: vec![],
        };

        let Some(issue_url) = issue_url else {
            return ticket;
        };

        if let Some((owner, repo, number)) = Self::parse_github_issue_url(issue_url) {
            ticket.issue_number = number;

            match self.fetch_issue(&owner, &repo, number).await {
                Ok(issue) => {
                    ticket.issue_number = issue.number;
                    ticket.title = issue.title;
                    ticket.body = issue.body;
                    ticket.url = issue.html_url;
                    ticket.acceptance_criteria = Self::extract_acceptance_criteria(&ticket.body);
                }
                Err(error) => {
                    warn!(
                        ticket = ticket_id,
                        issue_url,
                        error = %error,
                        "Failed to fetch GitHub issue details; falling back to minimal ticket"
                    );
                }
            }
        } else {
            warn!(
                ticket = ticket_id,
                issue_url, "Could not parse GitHub issue URL; falling back to minimal ticket"
            );
        }

        ticket
    }

    async fn check_existing_pr(
        &self,
        worker_id: &str,
        ticket_id: &str,
    ) -> Result<Option<(String, u64, String)>> {
        let repo_str = std::env::var("GITHUB_REPOSITORY").unwrap_or_default();
        let (owner, repo_name) = repo_str
            .split_once('/')
            .unwrap_or(("The-AgenticFlow", "template-counterapp"));

        let branch_name = WorktreeManager::branch_name(worker_id, ticket_id);

        let client = reqwest::Client::new();
        let resp = client
            .get(format!(
                "https://api.github.com/repos/{}/{}/pulls?head={}:{}&state=open",
                owner, repo_name, owner, branch_name
            ))
            .header("Authorization", format!("Bearer {}", self.github_token))
            .header("User-Agent", "agentflow-forge")
            .header("Accept", "application/vnd.github+json")
            .send()
            .await?;

        if !resp.status().is_success() {
            return Ok(None);
        }

        let prs: Vec<serde_json::Value> = resp.json().await.unwrap_or_default();
        if let Some(pr) = prs.first() {
            let pr_url = pr["html_url"].as_str().unwrap_or_default().to_string();
            let pr_number = pr["number"].as_u64().unwrap_or(0);
            if pr_number > 0 {
                info!(
                    worker = worker_id,
                    pr_number,
                    branch = %branch_name,
                    "Found existing PR on GitHub for fuel-exhausted worker"
                );
                return Ok(Some((pr_url, pr_number, branch_name)));
            }
        }

        Ok(None)
    }

    async fn push_and_create_pr(
        &self,
        worker_id: &str,
        ticket_id: &str,
        ticket_title: &str,
        ticket_body: &str,
    ) -> Result<(String, u64, String)> {
        use anyhow::Context as _;
        use std::process::Command as StdCommand;

        let worktree_path = self
            .workspace_root
            .join("worktrees")
            .join(format!("{}-{}", worker_id, ticket_id));
        let branch_name = WorktreeManager::branch_name(worker_id, ticket_id);

        if !worktree_path.exists() {
            return Err(anyhow!(
                "Worktree does not exist at {}",
                worktree_path.display()
            ));
        }

        let has_changes = StdCommand::new("git")
            .args(["status", "--porcelain"])
            .current_dir(&worktree_path)
            .output()
            .map(|o| !o.stdout.is_empty())
            .unwrap_or(false);

        if has_changes {
            info!(
                worker = worker_id,
                "Committing uncommitted changes before push"
            );
            StdCommand::new("git")
                .args(["add", "-A"])
                .current_dir(&worktree_path)
                .output()
                .context("Failed to git add")?;

            StdCommand::new("git")
                .args([
                    "commit",
                    "-m",
                    &format!("{}: complete implementation", ticket_id),
                ])
                .current_dir(&worktree_path)
                .output()
                .context("Failed to git commit")?;
        }

        let has_commits = StdCommand::new("git")
            .args(["log", "main..HEAD", "--oneline"])
            .current_dir(&worktree_path)
            .output()
            .map(|o| !o.stdout.is_empty())
            .unwrap_or(false);

        if !has_commits {
            return Err(anyhow!("No commits on branch {} beyond main", branch_name));
        }

        info!(worker = worker_id, branch = %branch_name, "Pushing branch to origin");
        let push_output = StdCommand::new("git")
            .args(["push", "-u", "origin", &branch_name])
            .current_dir(&worktree_path)
            .output()
            .context("Failed to push branch")?;

        if !push_output.status.success() {
            let stderr = String::from_utf8_lossy(&push_output.stderr);
            if stderr.contains("non-fast-forward")
                || stderr.contains("rejected")
                || stderr.contains("fetch first")
            {
                info!(worker = worker_id, branch = %branch_name, "Normal push rejected — force-pushing with --force-with-lease");
                let force_push = StdCommand::new("git")
                    .args(["push", "-u", "origin", &branch_name, "--force-with-lease"])
                    .current_dir(&worktree_path)
                    .output()
                    .context("Failed to force-push branch")?;

                if !force_push.status.success() {
                    let force_stderr = String::from_utf8_lossy(&force_push.stderr);
                    return Err(anyhow!("Failed to force-push branch: {}", force_stderr));
                }
            } else if !stderr.contains("already exists") && !stderr.contains("up-to-date") {
                return Err(anyhow!("Failed to push branch: {}", stderr));
            }
        }

        let repo_str = std::env::var("GITHUB_REPOSITORY").unwrap_or_default();
        let (owner, repo_name) = repo_str
            .split_once('/')
            .unwrap_or(("The-AgenticFlow", "template-counterapp"));

        let client = reqwest::Client::new();

        let existing_pr_url = format!(
            "https://api.github.com/repos/{}/{}/pulls?head={}:{}&state=open",
            owner, repo_name, owner, branch_name
        );
        let list_resp = client
            .get(&existing_pr_url)
            .header("Authorization", format!("Bearer {}", self.github_token))
            .header("User-Agent", "agentflow-forge")
            .header("Accept", "application/vnd.github+json")
            .send()
            .await?;

        if list_resp.status().is_success() {
            let prs: Vec<serde_json::Value> = list_resp.json().await.unwrap_or_default();
            if let Some(pr) = prs.first() {
                let pr_url = pr["html_url"].as_str().unwrap_or_default().to_string();
                let pr_number = pr["number"].as_u64().unwrap_or(0);
                if pr_number > 0 {
                    info!(
                        worker = worker_id,
                        pr_number,
                        branch = %branch_name,
                        "Found existing open PR for branch — updating instead of creating new"
                    );
                    return Ok((pr_url, pr_number, branch_name));
                }
            }
        }

        let pr_title = format!("[{}] {}", ticket_id, ticket_title);
        let pr_body = format!(
            "## {}\n\nResolves #{}\n\n---\n\n### Implementation\n\n{}",
            ticket_title,
            ticket_id.trim_start_matches("T-0").trim_start_matches('0'),
            if ticket_body.is_empty() {
                "See ticket for details.".to_string()
            } else {
                ticket_body.to_string()
            }
        );

        info!(owner, repo_name, branch = %branch_name, "Creating PR via GitHub API");
        let client = reqwest::Client::new();
        let pr_body_json = serde_json::json!({
            "title": pr_title,
            "body": pr_body,
            "head": branch_name,
            "base": "main"
        });

        let resp = client
            .post(format!(
                "https://api.github.com/repos/{}/{}/pulls",
                owner, repo_name
            ))
            .header("Authorization", format!("Bearer {}", self.github_token))
            .header("User-Agent", "agentflow-forge")
            .header("Accept", "application/vnd.github+json")
            .json(&pr_body_json)
            .send()
            .await?;

        if !resp.status().is_success() {
            let status = resp.status();
            let body = resp.text().await.unwrap_or_default();
            if body.contains("already exists") {
                let list_resp = client
                    .get(format!(
                        "https://api.github.com/repos/{}/{}/pulls?head={}:{}&state=open",
                        owner, repo_name, owner, branch_name
                    ))
                    .header("Authorization", format!("Bearer {}", self.github_token))
                    .header("User-Agent", "agentflow-forge")
                    .header("Accept", "application/vnd.github+json")
                    .send()
                    .await?;

                if list_resp.status().is_success() {
                    let prs: Vec<serde_json::Value> = list_resp.json().await.unwrap_or_default();
                    if let Some(pr) = prs.first() {
                        let pr_url = pr["html_url"].as_str().unwrap_or_default().to_string();
                        let pr_number = pr["number"].as_u64().unwrap_or(0);
                        return Ok((pr_url, pr_number, branch_name));
                    }
                }
                return Err(anyhow!("PR already exists but could not fetch its details"));
            }
            return Err(anyhow!("GitHub API returned {}: {}", status, body));
        }

        #[derive(Deserialize)]
        struct PrResponse {
            html_url: String,
            number: u64,
        }
        let pr: PrResponse = resp.json().await?;
        info!(pr_url = %pr.html_url, pr_number = pr.number, "PR created via GitHub API");
        Ok((pr.html_url, pr.number, branch_name))
    }
}

#[async_trait]
impl BatchNode for ForgePairNode {
    fn name(&self) -> &str {
        "forge_pair"
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

        // Store the worker IDs we're about to process so we can handle failures
        let worker_ids: Vec<String> = active_workers
            .iter()
            .filter_map(|v| v["id"].as_str().map(|s| s.to_string()))
            .collect();
        store.set("_forge_batch_workers", json!(worker_ids)).await;

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

        info!(
            worker = worker_id,
            ticket = ticket_id,
            "Starting FORGE-SENTINEL pair lifecycle"
        );

        let ticket = self.build_ticket(&ticket_id, issue_url.as_deref()).await;

        let config = PairConfig::new(&worker_id, &self.workspace_root, &self.github_token);

        let mut pair = ForgeSentinelPair::new(config);
        let outcome = pair
            .run(&ticket)
            .await
            .map_err(|e| anyhow!("Pair lifecycle failed: {:#}", e))?;

        match outcome {
            PairOutcome::PrOpened {
                pr_url,
                pr_number,
                branch,
            } => {
                info!(
                    worker = worker_id,
                    pr_url = %pr_url,
                    pr_number,
                    "Pair completed - PR opened"
                );
                Ok(json!({
                    "worker_id": worker_id,
                    "ticket_id": ticket_id,
                    "outcome": "pr_opened",
                    "pr_url": pr_url,
                    "pr_number": pr_number,
                    "branch": branch,
                }))
            }
            PairOutcome::Blocked { reason, blockers } => {
                if reason.contains("PR not created") || reason.contains("needs push/PR creation") {
                    info!(
                        worker = worker_id,
                        ticket = ticket_id,
                        "Work complete but no PR - attempting to push and create PR via GitHub API"
                    );
                    match self
                        .push_and_create_pr(&worker_id, &ticket_id, &ticket.title, &ticket.body)
                        .await
                    {
                        Ok((pr_url, pr_number, branch)) => {
                            info!(
                                worker = worker_id,
                                pr_url = %pr_url,
                                pr_number,
                                "PR created successfully via GitHub API"
                            );
                            return Ok(json!({
                                "worker_id": worker_id,
                                "ticket_id": ticket_id,
                                "outcome": "pr_opened",
                                "pr_url": pr_url,
                                "pr_number": pr_number,
                                "branch": branch,
                            }));
                        }
                        Err(e) => {
                            warn!(
                                worker = worker_id,
                                error = %e,
                                "Failed to create PR via GitHub API - returning blocked"
                            );
                        }
                    }
                }
                warn!(
                    worker = worker_id,
                    reason = %reason,
                    "Pair blocked - needs human intervention"
                );
                Ok(json!({
                    "worker_id": worker_id,
                    "ticket_id": ticket_id,
                    "outcome": "blocked",
                    "reason": reason,
                    "blockers": blockers,
                }))
            }
            PairOutcome::FuelExhausted {
                reason,
                reset_count,
            } => {
                warn!(
                    worker = worker_id,
                    reason = %reason,
                    resets = reset_count,
                    "Pair fuel exhausted"
                );
                Ok(json!({
                    "worker_id": worker_id,
                    "ticket_id": ticket_id,
                    "outcome": "fuel_exhausted",
                    "reason": reason,
                    "reset_count": reset_count,
                }))
            }
        }
    }

    async fn post_batch(&self, store: &SharedStore, results: Vec<Result<Value>>) -> Result<Action> {
        let mut slots: HashMap<String, WorkerSlot> =
            store.get_typed(KEY_WORKER_SLOTS).await.unwrap_or_default();

        let mut command_gate: HashMap<String, Value> =
            store.get_typed(KEY_COMMAND_GATE).await.unwrap_or_default();

        let mut tickets: Vec<Ticket> = store.get_typed(KEY_TICKETS).await.unwrap_or_default();

        let batch_workers: Vec<String> = store
            .get("_forge_batch_workers")
            .await
            .and_then(|v| serde_json::from_value(v).ok())
            .unwrap_or_default();

        let mut successful_workers: std::collections::HashSet<String> =
            std::collections::HashSet::new();
        let mut all_success = true;

        // Collect ticket status updates to apply
        let mut ticket_updates: Vec<(String, TicketStatus)> = Vec::new();

        // Collect PRs for VESSEL to process
        let mut opened_prs: Vec<Value> = Vec::new();

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

            if !worker_id.is_empty() {
                successful_workers.insert(worker_id.to_string());
            }

            if let Some(slot) = slots.get_mut(worker_id) {
                match outcome {
                    "pr_opened" => {
                        info!(
                            worker = worker_id,
                            ticket = ticket_id,
                            "Pair completed - PR opened"
                        );
                        slot.status = WorkerStatus::Done {
                            ticket_id: ticket_id.to_string(),
                            outcome: "pr_opened".to_string(),
                        };
                        ticket_updates.push((
                            ticket_id.to_string(),
                            TicketStatus::Completed {
                                worker_id: worker_id.to_string(),
                                outcome: "pr_opened".to_string(),
                            },
                        ));

                        // Add PR to pending_prs for VESSEL
                        let pr_number = res["pr_number"].as_u64().unwrap_or(0);
                        let branch = res["branch"].as_str().unwrap_or("");
                        if pr_number > 0 {
                            opened_prs.push(json!({
                                "number": pr_number,
                                "ticket_id": ticket_id,
                                "branch": branch,
                                "worker_id": worker_id,
                            }));
                            info!(pr_number, ticket_id, "Added PR to pending_prs for VESSEL");
                        }
                    }
                    "blocked" => {
                        let reason = res["reason"].as_str().unwrap_or("unknown");
                        info!(
                            worker = worker_id,
                            ticket = ticket_id,
                            reason,
                            "Pair blocked - needs intervention"
                        );
                        slot.status = WorkerStatus::Suspended {
                            ticket_id: ticket_id.to_string(),
                            reason: reason.to_string(),
                            issue_url: res["issue_url"].as_str().map(|s| s.to_string()),
                        };
                        command_gate.insert(worker_id.to_string(), res.clone());
                    }
                    "idle" => {}
                    "fuel_exhausted" => {
                        warn!(
                            worker = worker_id,
                            ticket = ticket_id,
                            "Pair fuel exhausted - checking for existing PR on GitHub"
                        );

                        match self.check_existing_pr(worker_id, ticket_id).await {
                            Ok(Some((pr_url, pr_number, branch))) => {
                                info!(
                                    worker = worker_id,
                                    pr_number,
                                    "PR already exists for fuel-exhausted worker - routing to VESSEL"
                                );
                                slot.status = WorkerStatus::Done {
                                    ticket_id: ticket_id.to_string(),
                                    outcome: "pr_opened".to_string(),
                                };
                                ticket_updates.push((
                                    ticket_id.to_string(),
                                    TicketStatus::Completed {
                                        worker_id: worker_id.to_string(),
                                        outcome: "pr_opened".to_string(),
                                    },
                                ));
                                opened_prs.push(json!({
                                    "number": pr_number,
                                    "ticket_id": ticket_id,
                                    "branch": branch,
                                    "worker_id": worker_id,
                                    "pr_url": pr_url,
                                }));
                            }
                            _ => {
                                slot.status = WorkerStatus::Idle;
                                all_success = false;
                                let prev_attempts = tickets
                                    .iter()
                                    .find(|t| t.id == ticket_id)
                                    .map(|t| t.attempts)
                                    .unwrap_or(0)
                                    + 1;
                                if prev_attempts >= Ticket::MAX_ATTEMPTS {
                                    ticket_updates.push((
                                        ticket_id.to_string(),
                                        TicketStatus::Exhausted {
                                            worker_id: worker_id.to_string(),
                                            attempts: prev_attempts,
                                        },
                                    ));
                                } else {
                                    ticket_updates.push((
                                        ticket_id.to_string(),
                                        TicketStatus::Failed {
                                            worker_id: worker_id.to_string(),
                                            reason: "fuel_exhausted".to_string(),
                                            attempts: prev_attempts,
                                        },
                                    ));
                                }
                            }
                        }
                    }
                    _ => {
                        warn!(
                            worker = worker_id,
                            ticket = ticket_id,
                            outcome,
                            "Pair failed"
                        );
                        slot.status = WorkerStatus::Idle;
                        all_success = false;
                        let prev_attempts = tickets
                            .iter()
                            .find(|t| t.id == ticket_id)
                            .map(|t| t.attempts)
                            .unwrap_or(0)
                            + 1;
                        if prev_attempts >= Ticket::MAX_ATTEMPTS {
                            ticket_updates.push((
                                ticket_id.to_string(),
                                TicketStatus::Exhausted {
                                    worker_id: worker_id.to_string(),
                                    attempts: prev_attempts,
                                },
                            ));
                        } else {
                            ticket_updates.push((
                                ticket_id.to_string(),
                                TicketStatus::Failed {
                                    worker_id: worker_id.to_string(),
                                    reason: outcome.to_string(),
                                    attempts: prev_attempts,
                                },
                            ));
                        }
                    }
                }
            }
        }

        for worker_id in &batch_workers {
            if !successful_workers.contains(worker_id) {
                if let Some(slot) = slots.get_mut(worker_id) {
                    let failed_ticket_id = match &slot.status {
                        WorkerStatus::Assigned { ticket_id, .. } => Some(ticket_id.clone()),
                        WorkerStatus::Working { ticket_id, .. } => Some(ticket_id.clone()),
                        _ => None,
                    };

                    warn!(
                        worker = worker_id,
                        "Resetting worker to Idle due to execution failure"
                    );
                    slot.status = WorkerStatus::Idle;

                    if let Some(ticket_id) = failed_ticket_id {
                        let prev_attempts = tickets
                            .iter()
                            .find(|t| t.id == ticket_id)
                            .map(|t| t.attempts)
                            .unwrap_or(0)
                            + 1;
                        if prev_attempts >= Ticket::MAX_ATTEMPTS {
                            ticket_updates.push((
                                ticket_id,
                                TicketStatus::Exhausted {
                                    worker_id: worker_id.to_string(),
                                    attempts: prev_attempts,
                                },
                            ));
                        } else {
                            ticket_updates.push((
                                ticket_id,
                                TicketStatus::Failed {
                                    worker_id: worker_id.to_string(),
                                    reason: "spawn_failed".to_string(),
                                    attempts: prev_attempts,
                                },
                            ));
                        }
                    }
                }
            }
        }

        // Apply ticket status updates
        for (ticket_id, new_status) in ticket_updates {
            if let Some(ticket) = tickets.iter_mut().find(|t| t.id == ticket_id) {
                if let TicketStatus::Failed { attempts, .. } = &new_status {
                    ticket.attempts = *attempts;
                } else if let TicketStatus::Exhausted { attempts, .. } = &new_status {
                    ticket.attempts = *attempts;
                }
                ticket.status = new_status;
                info!(
                    ticket = ticket.id,
                    status = ?ticket.status,
                    "Ticket status updated"
                );
            } else {
                warn!(
                    ticket_id,
                    "Ticket not found in store for status update - adding"
                );
                tickets.push(Ticket {
                    id: ticket_id.clone(),
                    title: String::new(),
                    body: String::new(),
                    priority: 0,
                    branch: None,
                    status: new_status,
                    issue_url: None,
                    attempts: 1,
                });
            }
        }

        store.set(KEY_WORKER_SLOTS, json!(slots)).await;
        store.set(KEY_COMMAND_GATE, json!(command_gate)).await;
        store.set(KEY_TICKETS, json!(tickets)).await;

        let has_prs = !opened_prs.is_empty();
        if has_prs {
            let mut pending_prs: Vec<Value> =
                store.get_typed(KEY_PENDING_PRS).await.unwrap_or_default();
            pending_prs.extend(opened_prs);
            store.set(KEY_PENDING_PRS, json!(pending_prs)).await;
            info!("Updated pending_prs for VESSEL processing");
        }

        let has_suspended = slots
            .values()
            .any(|s| matches!(s.status, WorkerStatus::Suspended { .. }));

        if has_suspended {
            Ok(Action::new("suspended"))
        } else if (has_prs || all_success) && !results.is_empty() {
            Ok(Action::new(ACTION_PR_OPENED))
        } else if results.is_empty() {
            Ok(Action::new(ACTION_EMPTY))
        } else {
            Ok(Action::new(ACTION_FAILED))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::ForgePairNode;

    #[test]
    fn parse_github_issue_url_extracts_owner_repo_and_number() {
        let parsed = ForgePairNode::parse_github_issue_url(
            "https://github.com/The-AgenticFlow/template-counterapp/issues/4",
        )
        .unwrap();

        assert_eq!(parsed.0, "The-AgenticFlow");
        assert_eq!(parsed.1, "template-counterapp");
        assert_eq!(parsed.2, 4);
    }

    #[test]
    fn extract_acceptance_criteria_prefers_dedicated_section() {
        let body = r#"
# Counter UI Frontend

## Acceptance Criteria
- Render the current count value
- Increment and decrement controls update the count
- Styling matches the provided design

## Notes
- Mobile responsive
"#;

        let criteria = ForgePairNode::extract_acceptance_criteria(body);
        assert_eq!(
            criteria,
            vec![
                "Render the current count value",
                "Increment and decrement controls update the count",
                "Styling matches the provided design",
            ]
        );
    }

    #[test]
    fn extract_acceptance_criteria_falls_back_to_markdown_tasks() {
        let body = r#"
Implement the counter experience.

- [ ] Add increment action
- [ ] Add decrement action
- [ ] Add reset action
"#;

        let criteria = ForgePairNode::extract_acceptance_criteria(body);
        assert_eq!(
            criteria,
            vec![
                "Add increment action",
                "Add decrement action",
                "Add reset action",
            ]
        );
    }
}
