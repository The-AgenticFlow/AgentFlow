// crates/agent-nexus/src/lib.rs
use agent_client::{AgentDecision, AgentPersona, AgentRunner};
use anyhow::Result;
use async_trait::async_trait;
use config::{
    state::{KEY_COMMAND_GATE, KEY_TICKETS, KEY_WORKER_SLOTS},
    Registry, Ticket, TicketStatus, WorkerSlot, WorkerStatus,
};
use pocketflow_core::{node::STOP_SIGNAL, Action, Node, SharedStore};
use serde::Deserialize;
use serde_json::{json, Value};
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};
use tracing::{info, warn};

const NO_WORK_THRESHOLD: u32 = 3;
const KEY_NO_WORK_COUNT: &str = "_no_work_count";

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
    /// Cursor into SharedStore event ring buffer — used to consume ticket_merged events.
    last_event_cursor: AtomicUsize,
}

impl NexusNode {
    pub fn new(persona_path: impl Into<PathBuf>, registry_path: impl Into<PathBuf>) -> Self {
        Self {
            persona_path: persona_path.into(),
            registry_path: registry_path.into(),
            last_event_cursor: AtomicUsize::new(0),
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

        let mut tickets: Vec<Ticket> = store.get_typed(KEY_TICKETS).await.unwrap_or_default();

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
                depends_on: vec![],
            });
        }

        store.set(KEY_TICKETS, json!(tickets)).await;
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

        // Load tickets and process any ticket_merged events from the shared store.
        let mut tickets: Vec<Ticket> = store.get_typed(KEY_TICKETS).await.unwrap_or_default();

        // Consume in-memory events emitted by agents (VESSEL will emit "ticket_merged").
        let cursor = self.last_event_cursor.load(Ordering::SeqCst);
        let events = store.get_events_since(cursor).await;
        if !events.is_empty() {
            for ev in &events {
                if ev.event_type == "ticket_merged" {
                    // Expect payload: { "ticket_id": "T-001", "pr_url": "..." }
                    if let Some(tid) = ev.payload.get("ticket_id").and_then(|v| v.as_str()) {
                        if let Some(ticket) = tickets.iter_mut().find(|t| t.id == tid) {
                            // Mark as merged so dependent tickets can be re-evaluated.
                            let pr_url = ev.payload.get("pr_url").and_then(|v| v.as_str()).map(|s| s.to_string());
                            ticket.status = TicketStatus::Merged { worker_id: "vessel".to_string(), pr_url };
                        }
                    }
                }
            }
            // advance cursor
            let new_cursor = cursor + events.len();
            self.last_event_cursor.store(new_cursor, Ordering::SeqCst);
            // persist updated tickets
            store.set(KEY_TICKETS, json!(tickets)).await;
        }
        let tickets: Vec<Ticket> = store.get_typed(KEY_TICKETS).await.unwrap_or_default();
        let worker_slots = store.get(KEY_WORKER_SLOTS).await.unwrap_or(json!({}));
        let open_prs = store.get("open_prs").await.unwrap_or(json!([]));
        let command_gate = store.get(KEY_COMMAND_GATE).await.unwrap_or(json!({}));

        let assignable_tickets: Vec<&Ticket> =
            tickets.iter().filter(|t| t.is_assignable()).collect();
        // Build a quick lookup of ticket statuses for dependency checks.
        let status_map: HashMap<String, TicketStatus> = tickets
            .iter()
            .map(|t| (t.id.clone(), t.status.clone()))
            .collect();

        let mut changed = false;
        // Return owned clones of assignable tickets to avoid holding references
        // into `tickets` which would prevent moving/serializing `tickets` later.
        let mut assignable_tickets: Vec<Ticket> = Vec::new();

        for t in &mut tickets {
            // If ticket declares dependencies, ensure they are all merged before allowing assignment.
            if !t.depends_on.is_empty() {
                let all_merged = t.depends_on.iter().all(|dep| {
                    status_map
                        .get(dep)
                        .map(|s| matches!(s, TicketStatus::Merged { .. }))
                        .unwrap_or(false)
                });

                if !all_merged {
                    // Move to WAITING_ON_DEPENDENCY if not already
                    match &t.status {
                        TicketStatus::WaitingOnDependency { .. } => {}
                        _ => {
                            t.status = TicketStatus::WaitingOnDependency { depends_on: t.depends_on.clone() };
                            changed = true;
                        }
                    }
                    continue; // not assignable
                } else {
                    // All dependencies are merged; if we were previously waiting,
                    // transition back to Open so the ticket can be assigned.
                    if matches!(t.status, TicketStatus::WaitingOnDependency { .. }) {
                        t.status = TicketStatus::Open;
                        changed = true;
                    }
                }
            }

            if t.is_assignable() {
                assignable_tickets.push(t.clone());
            }
        }

        if changed {
            store.set(KEY_TICKETS, json!(tickets)).await;
        }

        Ok(json!({
            "tickets": tickets,
            "assignable_tickets": assignable_tickets,
            "worker_slots": worker_slots,
            "open_prs": open_prs,
            "command_gate": command_gate,
            "repository": repository,
            "owner": owner,
            "repo_name": repo_name,
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

        if decision.action == "work_assigned" {
            store.set(KEY_NO_WORK_COUNT, json!(0)).await;

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
                        info!(
                            ticket_id,
                            "Creating new ticket in store from LLM assignment"
                        );
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
                            depends_on: vec![],
                        });
                    }
                    store.set(KEY_TICKETS, json!(tickets)).await;

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
                    "No work found after {} consecutive checks — stopping", NO_WORK_THRESHOLD
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

        Ok(Action::new(decision.action))
    }
}
