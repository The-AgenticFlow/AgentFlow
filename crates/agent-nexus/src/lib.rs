// crates/agent-nexus/src/lib.rs
use agent_client::{AgentDecision, AgentPersona, AgentRunner};
use anyhow::Result;
use async_trait::async_trait;
use config::{
    state::{KEY_COMMAND_GATE, KEY_WORKER_SLOTS},
    Registry, WorkerSlot, WorkerStatus,
};
use pocketflow_core::{Action, Node, SharedStore};
use serde_json::{json, Value};
use std::collections::HashMap;
use std::path::PathBuf;
use tracing::{info, warn};

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

        // Add new slots from registry
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

        // TODO: Handle removal of slots if needed

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
        // Reload registry first
        if let Err(e) = self.sync_registry(store).await {
            warn!("Failed to sync registry: {}", e);
        }

        // Read everything the orchestrator needs to see
        let tickets = store.get("tickets").await.unwrap_or(json!([]));
        let worker_slots = store.get(KEY_WORKER_SLOTS).await.unwrap_or(json!({}));
        let open_prs = store.get("open_prs").await.unwrap_or(json!([]));
        let command_gate = store.get(KEY_COMMAND_GATE).await.unwrap_or(json!({}));
        let repository = store.get("repository").await.unwrap_or(json!(""));

        // Pre-parse repository into owner/repo for easier use
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

        Ok(json!({
            "tickets": tickets,
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

        let mut runner = AgentRunner::from_env().await?;
        let persona = self.load_persona().await?;

        // The runner drives the tool-calling loop (Anthropic + MCP)
        let decision: AgentDecision = runner.run(&persona, context, 10).await?;

        Ok(json!(decision))
    }

    async fn post(&self, store: &SharedStore, result: Value) -> Result<Action> {
        let decision: AgentDecision = serde_json::from_value(result)?;

        info!(action = %decision.action, notes = %decision.notes, "Nexus decision reached");

        if decision.action == "work_assigned" {
            if let Some(worker_id) = &decision.assign_to {
                if let Some(ticket_id) = &decision.ticket_id {
                    info!(worker_id, ticket_id, "Nexus: Assigning ticket to worker");
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

        // Handle CommandGate approval/rejection
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

                // Update worker status to Idle or Working (to be re-processed by Forge)
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
