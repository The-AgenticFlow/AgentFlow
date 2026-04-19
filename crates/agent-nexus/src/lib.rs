// crates/agent-nexus/src/lib.rs
use agent_client::{AgentDecision, AgentPersona, AgentRunner};
use anyhow::Result;
use async_trait::async_trait;
use config::{
    state::{KEY_COMMAND_GATE, KEY_PENDING_PRS, KEY_TICKETS, KEY_WORKER_SLOTS},
    ACTION_MERGE_PRS, ACTION_NO_WORK, ACTION_WORK_ASSIGNED, Registry, Ticket, TicketStatus,
    WorkerSlot, WorkerStatus,
};
use pocketflow_core::{node::STOP_SIGNAL, Action, Node, SharedStore};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::HashMap;
use std::path::PathBuf;
use tracing::{debug, info, warn};

const NO_WORK_THRESHOLD: u32 = 3;
const KEY_NO_WORK_COUNT: &str = "_no_work_count";
const KEY_CI_READINESS: &str = "ci_readiness";
const CI_SETUP_TICKET_ID: &str = "T-CI-001";

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CiReadiness {
    Ready,
    Missing,
    SetupInProgress,
}

fn is_ci_setup_ticket(ticket: &Ticket) -> bool {
    let t = ticket.title.to_lowercase();
    t.contains("ci") && (t.contains("setup") || t.contains("pipeline") || t.contains("workflow"))
        || ticket.id == CI_SETUP_TICKET_ID
        || ticket.id.starts_with("T-CI-")
}

fn has_ci_setup_ticket(tickets: &[Ticket]) -> bool {
    tickets.iter().any(is_ci_setup_ticket)
}

fn ci_setup_ticket_active(tickets: &[Ticket]) -> bool {
    tickets.iter().any(|t| is_ci_setup_ticket(t) && t.is_assignable())
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct UnmergedPr {
    pub pr_number: u64,
    pub ticket_id: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct OrphanedTicket {
    pub ticket_id: String,
    pub worker_id: String,
    pub reason: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct StaleWorker {
    pub worker_id: String,
    pub ticket_id: String,
    pub reason: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct FlowRecovery {
    pub unmerged_prs: Vec<UnmergedPr>,
    pub orphaned_tickets: Vec<OrphanedTicket>,
    pub stale_workers: Vec<StaleWorker>,
    pub completed_without_pr: Vec<String>,
    pub has_unmerged_prs: bool,
    pub has_orphaned_tickets: bool,
    pub has_stale_workers: bool,
    pub has_completed_without_pr: bool,
    pub needs_recovery: bool,
}

#[derive(Debug, Deserialize)]
struct GitHubIssue {
    number: u64,
    title: String,
    body: Option<String>,
    html_url: String,
    pull_request: Option<serde_json::Value>,
}

pub struct NexusNode {
    pub persona_path: PathBuf,
    pub registry_path: PathBuf,
}

impl NexusNode {
    pub fn new(persona_path: impl Into<PathBuf>, registry_path: impl Into<PathBuf>) -> Self {
        Self {
            persona_path: persona_path.into(),
            registry_path: registry_path.into(),
        }
    }

    async fn sync_issues(&self, store: &SharedStore, owner: &str, repo_name: &str) -> Result<()> {
        if owner.is_empty() || repo_name.is_empty() {
            return Ok(());
        }

        let token = match std::env::var("GITHUB_PERSONAL_ACCESS_TOKEN") {
            Ok(t) => t,
            Err(_) => {
                warn!("GITHUB_PERSONAL_ACCESS_TOKEN not set, skipping issue sync");
                return Ok(());
            }
        };

        let url = format!(
            "https://api.github.com/repos/{}/{}/issues?state=open",
            owner, repo_name
        );

        let client = reqwest::Client::new();
        let resp = client
            .get(&url)
            .header("Authorization", format!("Bearer {}", token))
            .header("User-Agent", "agent-nexus")
            .header("Accept", "application/vnd.github+json")
            .send()
            .await?;

        if !resp.status().is_success() {
            warn!(status = %resp.status(), "GitHub API request failed during issue sync");
            return Ok(());
        }

        let gh_issues: Vec<GitHubIssue> = resp.json().await?;

        let mut tickets: Vec<Ticket> =
            store.get_typed(KEY_TICKETS).await.unwrap_or_default();

        for issue in &gh_issues {
            if issue.pull_request.is_some() {
                continue;
            }

            let ticket_id = format!("T-{:03}", issue.number);
            if tickets.iter().any(|t| t.id == ticket_id) {
                continue;
            }

            info!(ticket_id, title = %issue.title, "Synced new ticket from GitHub issue");

            tickets.push(Ticket {
                id: ticket_id,
                title: issue.title.clone(),
                body: issue.body.clone().unwrap_or_default(),
                priority: 0,
                branch: None,
                status: TicketStatus::Open,
                issue_url: Some(issue.html_url.clone()),
                attempts: 0,
            });
        }

        store.set(KEY_TICKETS, json!(tickets)).await;
        Ok(())
    }

    async fn sync_open_prs(&self, store: &SharedStore, owner: &str, repo_name: &str) -> Result<()> {
        if owner.is_empty() || repo_name.is_empty() {
            return Ok(());
        }

        let token = match std::env::var("GITHUB_PERSONAL_ACCESS_TOKEN") {
            Ok(t) => t,
            Err(_) => {
                warn!("GITHUB_PERSONAL_ACCESS_TOKEN not set, skipping PR sync");
                return Ok(());
            }
        };

        let client = github::GithubRestClient::new(&token);
        let gh_prs = match client.list_open_prs(owner, repo_name).await {
            Ok(prs) => prs,
            Err(e) => {
                warn!(error = %e, "Failed to fetch open PRs from GitHub");
                return Ok(());
            }
        };

        let mut pending_prs: Vec<Value> =
            store.get_typed(KEY_PENDING_PRS).await.unwrap_or_default();

        let known_numbers: Vec<u64> = pending_prs
            .iter()
            .filter_map(|p| p["number"].as_u64())
            .collect();

        let mut new_prs = Vec::new();
        for pr in &gh_prs {
            if !known_numbers.contains(&pr.number) {
                let already_awaiting = pending_prs.iter().any(|p| {
                    p["number"].as_u64() == Some(pr.number)
                        && p["conflict_status"].as_str() == Some("awaiting_rework")
                });
                if already_awaiting {
                    debug!(
                        pr_number = pr.number,
                        "PR already tracked as awaiting rework — skipping re-add"
                    );
                    continue;
                }
                info!(
                    pr_number = pr.number,
                    ticket_id = ?pr.ticket_id,
                    title = %pr.title,
                    "Discovered untracked open PR on GitHub — adding to pending_prs"
                );
                new_prs.push(pr);
                pending_prs.push(json!({
                    "number": pr.number,
                    "ticket_id": pr.ticket_id,
                    "head_sha": pr.head_sha,
                    "head_branch": pr.head_branch,
                    "base_branch": pr.base_branch,
                    "title": pr.title,
                    "mergeable": pr.mergeable,
                    "has_conflicts": pr.has_conflicts(),
                }));
            }
        }

        let before_count = pending_prs.len();
        pending_prs.retain(|p| {
            let pr_num = p["number"].as_u64().unwrap_or(0);
            if pr_num == 0 {
                return false;
            }
            let still_open = gh_prs.iter().any(|gh| gh.number == pr_num);
            if !still_open {
                info!(pr_number = pr_num, "PR no longer open on GitHub — removing from pending_prs");
            }
            still_open
        });

        let prs_changed = pending_prs.len() != known_numbers.len()
            || pending_prs.len() != before_count;

        if prs_changed {
            store.set(KEY_PENDING_PRS, json!(pending_prs)).await;
        }

        if !new_prs.is_empty() {
            let mut tickets: Vec<Ticket> =
                store.get_typed(KEY_TICKETS).await.unwrap_or_default();
            let mut tickets_changed = false;

            for pr in &new_prs {
                if let Some(ref tid) = pr.ticket_id {
                    if let Some(ticket) = tickets.iter_mut().find(|t| t.id == *tid) {
                        match &ticket.status {
                            TicketStatus::Failed { .. }
                            | TicketStatus::Open
                            | TicketStatus::Assigned { .. }
                            | TicketStatus::Exhausted { .. } => {
                                info!(
                                    ticket_id = tid,
                                    pr_number = pr.number,
                                    old_status = ?ticket.status,
                                    "Ticket has open PR but inconsistent status — correcting to Completed(pr_opened)"
                                );
                                ticket.status = TicketStatus::Completed {
                                    worker_id: String::from("nexus-reconciliation"),
                                    outcome: "pr_opened".to_string(),
                                };
                                tickets_changed = true;
                            }
                            _ => {}
                        }
                    }
                }
            }

            if tickets_changed {
                store.set(KEY_TICKETS, json!(tickets)).await;
            }
        }

        Ok(())
    }

    async fn load_persona(&self) -> Result<AgentPersona> {
        let content = tokio::fs::read_to_string(&self.persona_path).await?;
        Ok(AgentPersona {
            id: "nexus".to_string(),
            role: "orchestrator".to_string(),
            system_prompt: content,
        })
    }

    async fn sync_registry(&self, store: &SharedStore) -> Result<()> {
        if !self.registry_path.exists() {
            return Ok(());
        }

        let registry = Registry::load(&self.registry_path)?;
        let mut slots: HashMap<String, WorkerSlot> =
            store.get_typed(KEY_WORKER_SLOTS).await.unwrap_or_default();

        let mut changed = false;

        for slot_id in registry.forge_slots() {
            if !slots.contains_key(&slot_id) {
                info!(slot = slot_id, "Adding new worker slot from registry");
                slots.insert(
                    slot_id.clone(),
                    WorkerSlot {
                        id: slot_id,
                        status: WorkerStatus::Idle,
                    },
                );
                changed = true;
            }
        }

        if changed {
            store.set(KEY_WORKER_SLOTS, json!(slots)).await;
        }

        Ok(())
    }

    async fn check_ci_readiness(
        &self,
        store: &SharedStore,
        owner: &str,
        repo_name: &str,
    ) -> CiReadiness {
        let current: Option<CiReadiness> = store.get_typed(KEY_CI_READINESS).await;
        if let Some(ref readiness) = current {
            if matches!(readiness, CiReadiness::SetupInProgress) {
                return CiReadiness::SetupInProgress;
            }
        }

        if owner.is_empty() || repo_name.is_empty() {
            return CiReadiness::Ready;
        }

        let token = match std::env::var("GITHUB_PERSONAL_ACCESS_TOKEN") {
            Ok(t) => t,
            Err(_) => {
                warn!("GITHUB_PERSONAL_ACCESS_TOKEN not set, assuming CI is ready");
                return CiReadiness::Ready;
            }
        };

        let client = github::GithubRestClient::new(&token);
        match client.has_workflows(owner, repo_name).await {
            Ok(true) => {
                info!("CI workflows found in repository — CI is ready");
                CiReadiness::Ready
            }
            Ok(false) => {
                info!("No CI workflows found in repository — CI setup required");
                CiReadiness::Missing
            }
            Err(e) => {
                warn!(error = %e, "Failed to check CI workflows, assuming ready");
                CiReadiness::Ready
            }
        }
    }

    fn ensure_ci_setup_ticket(&self, _store: &SharedStore, tickets: &mut Vec<Ticket>, readiness: &CiReadiness) {
        if !matches!(readiness, CiReadiness::Missing) {
            return;
        }

        if has_ci_setup_ticket(tickets) {
            info!("CI setup ticket already exists, skipping injection");
            return;
        }

        info!("Injecting CI setup ticket — must be completed before any other work");

        tickets.push(Ticket {
            id: CI_SETUP_TICKET_ID.to_string(),
            title: "CI: Setup GitHub Actions workflows".to_string(),
            body: "This repository has no CI/CD workflows. Create `.github/workflows/ci.yml` \
                   with build, test, and lint checks before any other work proceeds. \
                   Without CI, VESSEL cannot validate PRs and the merge pipeline stalls."
                .to_string(),
            priority: 0,
            branch: None,
            status: TicketStatus::Open,
            issue_url: None,
            attempts: 0,
        });
    }

    fn prioritize_ci_first(tickets: &mut Vec<Ticket>) {
        tickets.sort_by(|a, b| {
            let a_is_ci = is_ci_setup_ticket(a) as u8;
            let b_is_ci = is_ci_setup_ticket(b) as u8;
            b_is_ci.cmp(&a_is_ci).then_with(|| a.priority.cmp(&b.priority))
        });
    }

    async fn recover_orphans(store: &SharedStore) -> Result<()> {
        let mut tickets: Vec<Ticket> =
            store.get_typed(KEY_TICKETS).await.unwrap_or_default();
        let mut slots: HashMap<String, WorkerSlot> =
            store.get_typed(KEY_WORKER_SLOTS).await.unwrap_or_default();
        let mut changed_tickets = false;
        let mut changed_slots = false;

        for ticket in tickets.iter_mut() {
            match &ticket.status {
                TicketStatus::Assigned { worker_id }
                | TicketStatus::InProgress { worker_id } => {
                    let worker_idle = slots.get(worker_id).map_or(true, |s| {
                        matches!(s.status, WorkerStatus::Idle)
                    });
                    let worker_missing = !slots.contains_key(worker_id);
                    if worker_idle || worker_missing {
                        info!(
                            ticket_id = ticket.id,
                            worker_id,
                            "Recovering orphaned ticket — resetting to Open"
                        );
                        ticket.status = TicketStatus::Open;
                        changed_tickets = true;
                    }
                }
                _ => {}
            }
        }

        for slot in slots.values_mut() {
            match &slot.status {
                WorkerStatus::Suspended { ticket_id, .. } => {
                    let ticket_done = tickets.iter().any(|t| {
                        t.id == *ticket_id
                            && matches!(
                                t.status,
                                TicketStatus::Completed { .. } | TicketStatus::Merged { .. }
                            )
                    });
                    if ticket_done {
                        info!(
                            worker_id = slot.id,
                            ticket_id,
                            "Recovering stale worker — ticket completed, recycling to Idle"
                        );
                        slot.status = WorkerStatus::Idle;
                        changed_slots = true;
                    }
                }
                WorkerStatus::Assigned { ticket_id, .. }
                | WorkerStatus::Working { ticket_id, .. } => {
                    let ticket_open = tickets.iter().any(|t| {
                        t.id == *ticket_id && matches!(t.status, TicketStatus::Open)
                    });
                    if ticket_open {
                        info!(
                            worker_id = slot.id,
                            ticket_id,
                            "Recovering stale worker — ticket reset to Open, recycling to Idle"
                        );
                        slot.status = WorkerStatus::Idle;
                        changed_slots = true;
                    }
                }
                _ => {}
            }
        }

        if changed_tickets {
            store.set(KEY_TICKETS, json!(tickets)).await;
        }
        if changed_slots {
            store.set(KEY_WORKER_SLOTS, serde_json::to_value(slots)?).await;
        }

        Ok(())
    }

    fn reconcile(
        tickets: &[Ticket],
        worker_slots: &HashMap<String, WorkerSlot>,
        pending_prs: &[Value],
    ) -> FlowRecovery {
        let mut recovery = FlowRecovery::default();

        for pr in pending_prs {
            if let Some(obj) = pr.as_object() {
                let pr_number = obj.get("number").and_then(|v| v.as_u64());
                let ticket_id = obj.get("ticket_id").and_then(|v| v.as_str());
                if pr_number.is_some() {
                    recovery.unmerged_prs.push(UnmergedPr {
                        pr_number: pr_number.unwrap(),
                        ticket_id: ticket_id.map(|s| s.to_string()),
                    });
                }
            }
        }

        for ticket in tickets {
            match &ticket.status {
                TicketStatus::Assigned { worker_id }
                | TicketStatus::InProgress { worker_id } => {
                    let worker_exists = worker_slots.contains_key(worker_id);
                    let worker_idle = worker_slots.get(worker_id).map_or(false, |s| {
                        matches!(s.status, WorkerStatus::Idle)
                    });
                    if !worker_exists || worker_idle {
                        recovery.orphaned_tickets.push(OrphanedTicket {
                            ticket_id: ticket.id.clone(),
                            worker_id: worker_id.clone(),
                            reason: if !worker_exists {
                                "worker slot missing".to_string()
                            } else {
                                "worker is idle but ticket still assigned".to_string()
                            },
                        });
                    }
                }
                TicketStatus::Completed { outcome, .. } => {
                    if outcome == "pr_opened" {
                        let has_pending = pending_prs.iter().any(|pr| {
                            pr.get("ticket_id").and_then(|v| v.as_str()) == Some(&ticket.id)
                        });
                        if !has_pending {
                            recovery.completed_without_pr.push(ticket.id.clone());
                        }
                    }
                }
                _ => {}
            }
        }

        for slot in worker_slots.values() {
            match &slot.status {
                WorkerStatus::Assigned { ticket_id, .. }
                | WorkerStatus::Working { ticket_id, .. } => {
                    let ticket_exists = tickets.iter().any(|t| t.id == *ticket_id);
                    if !ticket_exists {
                        recovery.stale_workers.push(StaleWorker {
                            worker_id: slot.id.clone(),
                            ticket_id: ticket_id.clone(),
                            reason: "ticket no longer exists".to_string(),
                        });
                    }
                }
                WorkerStatus::Suspended { ticket_id, .. } => {
                    let ticket_completed = tickets.iter().any(|t| {
                        t.id == *ticket_id
                            && matches!(
                                t.status,
                                TicketStatus::Completed { .. } | TicketStatus::Merged { .. }
                            )
                    });
                    if ticket_completed {
                        recovery.stale_workers.push(StaleWorker {
                            worker_id: slot.id.clone(),
                            ticket_id: ticket_id.clone(),
                            reason: "ticket already completed/merged but worker still suspended"
                                .to_string(),
                        });
                    }
                }
                _ => {}
            }
        }

        recovery.has_unmerged_prs = !recovery.unmerged_prs.is_empty();
        recovery.has_orphaned_tickets = !recovery.orphaned_tickets.is_empty();
        recovery.has_stale_workers = !recovery.stale_workers.is_empty();
        recovery.has_completed_without_pr = !recovery.completed_without_pr.is_empty();
        recovery.needs_recovery = recovery.has_unmerged_prs
            || recovery.has_orphaned_tickets
            || recovery.has_stale_workers
            || recovery.has_completed_without_pr;

        recovery
    }
}

#[async_trait]
impl Node for NexusNode {
    fn name(&self) -> &str {
        "nexus"
    }

    async fn prep(&self, store: &SharedStore) -> Result<Value> {
        if let Err(e) = self.sync_registry(store).await {
            warn!("Failed to sync registry: {}", e);
        }

        let repository = store.get("repository").await.unwrap_or(json!(""));

        let (owner, repo_name) = repository
            .as_str()
            .and_then(|r| {
                let parts: Vec<&str> = r.split('/').collect();
                if parts.len() == 2 {
                    Some((parts[0].to_string(), parts[1].to_string()))
                } else {
                    None
                }
            })
            .unwrap_or((String::new(), String::new()));

        if let Err(e) = self.sync_issues(store, &owner, &repo_name).await {
            warn!("Failed to sync issues from GitHub: {}", e);
        }

        if let Err(e) = self.sync_open_prs(store, &owner, &repo_name).await {
            warn!("Failed to sync open PRs from GitHub: {}", e);
        }

        let ci_readiness = self.check_ci_readiness(store, &owner, &repo_name).await;
        store.set(KEY_CI_READINESS, json!(ci_readiness)).await;

        let mut tickets: Vec<Ticket> =
            store.get_typed(KEY_TICKETS).await.unwrap_or_default();

        self.ensure_ci_setup_ticket(store, &mut tickets, &ci_readiness);
        Self::prioritize_ci_first(&mut tickets);

        store.set(KEY_TICKETS, json!(tickets)).await;

        let tickets: Vec<Ticket> =
            store.get_typed(KEY_TICKETS).await.unwrap_or_default();

        let has_assignable = tickets.iter().any(|t| t.is_assignable());

        let mut worker_slots: HashMap<String, WorkerSlot> =
            store.get_typed(KEY_WORKER_SLOTS).await.unwrap_or_default();

        let mut recycled = false;
        if has_assignable {
            for slot in worker_slots.values_mut() {
                if matches!(slot.status, WorkerStatus::Done { .. }) {
                    info!(worker_id = slot.id, "Recycling Done worker to Idle — assignable tickets exist");
                    slot.status = WorkerStatus::Idle;
                    recycled = true;
                }
            }
        }
        if recycled {
            store.set(KEY_WORKER_SLOTS, json!(worker_slots)).await;
        }

        let worker_slots = store.get(KEY_WORKER_SLOTS).await.unwrap_or(json!({}));
        let open_prs = store.get(KEY_PENDING_PRS).await.unwrap_or(json!([]));
        let command_gate = store.get(KEY_COMMAND_GATE).await.unwrap_or(json!({}));

        let pending_prs_vec: Vec<Value> = open_prs.as_array().cloned().unwrap_or_default();
        let worker_slots_map: HashMap<String, WorkerSlot> =
            store.get_typed(KEY_WORKER_SLOTS).await.unwrap_or_default();
        let recovery = Self::reconcile(&tickets, &worker_slots_map, &pending_prs_vec);

        if recovery.needs_recovery {
            info!(
                unmerged_prs = recovery.unmerged_prs.len(),
                orphaned_tickets = recovery.orphaned_tickets.len(),
                stale_workers = recovery.stale_workers.len(),
                completed_without_pr = recovery.completed_without_pr.len(),
                "Flow recovery: inconsistencies detected"
            );
        }

        let ci_must_go_first = matches!(ci_readiness, CiReadiness::Missing)
            || (matches!(ci_readiness, CiReadiness::SetupInProgress)
                && ci_setup_ticket_active(&tickets));

        let assignable_tickets: Vec<&Ticket> = if ci_must_go_first {
            tickets.iter().filter(|t| is_ci_setup_ticket(t) && t.is_assignable()).collect()
        } else {
            tickets.iter().filter(|t| t.is_assignable()).collect()
        };

        Ok(json!({
            "tickets": tickets,
            "assignable_tickets": assignable_tickets,
            "worker_slots": worker_slots,
            "open_prs": open_prs,
            "command_gate": command_gate,
            "repository": repository,
            "owner": owner,
            "repo_name": repo_name,
            "ci_readiness": ci_readiness,
            "ci_must_go_first": ci_must_go_first,
            "flow_recovery": recovery,
        }))
    }

    async fn exec(&self, context: Value) -> Result<Value> {
        info!("Nexus calling AgentRunner for orchestration...");

        let model_backend = Registry::load(&self.registry_path)
            .ok()
            .and_then(|reg| reg.get("nexus").map(|e| e.model_backend.clone()))
            .flatten();

        let mut runner = AgentRunner::from_env_for_agent(model_backend.as_deref()).await?;
        let persona = self.load_persona().await?;

        let decision: AgentDecision = runner.run(&persona, context, 10).await?;

        Ok(json!(decision))
    }

    async fn post(&self, store: &SharedStore, result: Value) -> Result<Action> {
        let decision: AgentDecision = serde_json::from_value(result)?;

        info!(action = %decision.action, notes = %decision.notes, "Nexus decision reached");

        if decision.action == ACTION_MERGE_PRS {
            store.set(KEY_NO_WORK_COUNT, json!(0)).await;

            let pending_prs: Vec<Value> =
                store.get_typed(KEY_PENDING_PRS).await.unwrap_or_default();

            if pending_prs.is_empty() {
                info!("merge_prs action but no open PRs — reassigning to work assignment");
                return Ok(Action::new(ACTION_NO_WORK));
            }

            info!(
                pr_count = pending_prs.len(),
                "Nexus: Routing to VESSEL to merge {} pending PR(s)",
                pending_prs.len()
            );

            return Ok(Action::new(ACTION_MERGE_PRS));
        }

        if decision.action == "work_assigned" {
            store.set(KEY_NO_WORK_COUNT, json!(0)).await;

            Self::recover_orphans(store).await?;

            if let Some(worker_id) = &decision.assign_to {
                if let Some(ticket_id) = &decision.ticket_id {
                    info!(worker_id, ticket_id, "Nexus: Assigning ticket to worker");

                    let mut tickets: Vec<Ticket> =
                        store.get_typed(KEY_TICKETS).await.unwrap_or_default();
                    if let Some(ticket) = tickets.iter_mut().find(|t| t.id == *ticket_id) {
                        ticket.status = TicketStatus::Assigned {
                            worker_id: worker_id.clone(),
                        };
                        if let Some(url) = &decision.issue_url {
                            ticket.issue_url = Some(url.clone());
                        }
                    } else {
                        info!(ticket_id, "Creating new ticket in store from LLM assignment");
                        tickets.push(Ticket {
                            id: ticket_id.clone(),
                            title: decision.notes.clone(),
                            body: String::new(),
                            priority: 0,
                            branch: None,
                            status: TicketStatus::Assigned {
                                worker_id: worker_id.clone(),
                            },
                            issue_url: decision.issue_url.clone(),
                            attempts: 0,
                        });
                    }
                    store.set(KEY_TICKETS, json!(tickets)).await;

                    if ticket_id.starts_with("T-CI-") {
                        info!("CI setup ticket assigned — marking CI readiness as in-progress");
                        store.set(KEY_CI_READINESS, json!(CiReadiness::SetupInProgress)).await;
                    }

                    let mut slots: HashMap<String, WorkerSlot> =
                        store.get_typed(KEY_WORKER_SLOTS).await.unwrap_or_default();
                    if let Some(slot) = slots.get_mut(worker_id) {
                        slot.status = WorkerStatus::Assigned {
                            ticket_id: ticket_id.clone(),
                            issue_url: decision.issue_url.clone(),
                        };
                        store
                            .set(KEY_WORKER_SLOTS, serde_json::to_value(slots)?)
                            .await;
                        info!(worker_id, ticket_id, issue_url = ?decision.issue_url, "Nexus: Store updated with NEW worker assignment");
                    }
                }
            }
        }

        if decision.action == "no_work" {
            let count: u32 = store.get_typed(KEY_NO_WORK_COUNT).await.unwrap_or(0);
            let new_count = count + 1;
            store.set(KEY_NO_WORK_COUNT, json!(new_count)).await;

            if new_count >= NO_WORK_THRESHOLD {
                info!(
                    consecutive = new_count,
                    "No work found after {} consecutive checks — stopping",
                    NO_WORK_THRESHOLD
                );
                return Ok(Action::new(STOP_SIGNAL));
            }
        }

        if decision.action == "approve_command" || decision.action == "reject_command" {
            let mut gate: HashMap<String, Value> =
                store.get_typed(KEY_COMMAND_GATE).await.unwrap_or_default();
            if let Some(worker_id) = gate.keys().next().cloned() {
                info!(
                    worker = worker_id,
                    action = decision.action,
                    "CommandGate processing"
                );
                gate.remove(&worker_id);
                store.set(KEY_COMMAND_GATE, json!(gate)).await;

                let mut slots: HashMap<String, WorkerSlot> =
                    store.get_typed(KEY_WORKER_SLOTS).await.unwrap_or_default();
                if let Some(slot) = slots.get_mut(&worker_id) {
                    if decision.action == "approve_command" {
                        if let WorkerStatus::Suspended {
                            ticket_id,
                            issue_url,
                            ..
                        } = &slot.status
                        {
                            slot.status = WorkerStatus::Assigned {
                                ticket_id: ticket_id.clone(),
                                issue_url: issue_url.clone(),
                            };
                        }
                    } else {
                        slot.status = WorkerStatus::Idle;
                    }
                }
                store.set(KEY_WORKER_SLOTS, json!(slots)).await;
            }
        }

        if decision.action == "conflicts_detected" {
            store.set(KEY_NO_WORK_COUNT, json!(0)).await;

            let pending_prs: Vec<Value> =
                store.get_typed(KEY_PENDING_PRS).await.unwrap_or_default();

            for pr in &pending_prs {
                let pr_number = pr["number"].as_u64().unwrap_or(0);
                if pr_number == 0 {
                    continue;
                }
                let has_conflicts = pr["has_conflicts"].as_bool().unwrap_or(false);
                if !has_conflicts {
                    continue;
                }

                let ticket_id = pr["ticket_id"].as_str().unwrap_or("");
                if ticket_id.is_empty() {
                    continue;
                }

                info!(
                    ticket_id,
                    pr_number,
                    "Resetting conflicting ticket to Open for forge rework"
                );

                let mut tickets: Vec<Ticket> =
                    store.get_typed(KEY_TICKETS).await.unwrap_or_default();
                if let Some(ticket) = tickets.iter_mut().find(|t| t.id == ticket_id) {
                    ticket.status = TicketStatus::Open;
                    ticket.attempts += 1;
                }
                store.set(KEY_TICKETS, json!(tickets)).await;
            }

            let assignable_tickets: Vec<Ticket> =
                store.get_typed(KEY_TICKETS).await.unwrap_or_default();
            let has_assignable = assignable_tickets.iter().any(|t| t.is_assignable());

            if has_assignable {
                return Ok(Action::new(ACTION_WORK_ASSIGNED));
            }
            return Ok(Action::new(ACTION_NO_WORK));
        }

        Ok(Action::new(decision.action))
    }
}
