// binary/src/nodes/nexus.rs
use anyhow::Result;
use async_trait::async_trait;
use pocketflow_core::{Node, SharedStore, Action};
use serde_json::{json, Value};
use std::path::PathBuf;
use std::collections::HashMap;
use tracing::{info};

use crate::state::{
    Ticket, WorkerSlot, WorkerStatus, 
    KEY_TICKETS, KEY_WORKER_SLOTS,
    ACTION_WORK_ASSIGNED, ACTION_NO_WORK
};

pub struct NexusNode {
    pub registry_path: PathBuf,
}

impl NexusNode {
    pub fn new(registry_path: impl Into<PathBuf>) -> Self {
        Self { registry_path: registry_path.into() }
    }
}

#[async_trait]
impl Node for NexusNode {
    fn name(&self) -> &str { "nexus" }

    /// Phase 1: Read tickets and slots from store.
    async fn prep(&self, store: &SharedStore) -> Result<Value> {
        let tickets: Vec<Ticket> = store
            .get_typed(KEY_TICKETS)
            .await
            .unwrap_or_default();

        let slots: HashMap<String, WorkerSlot> = store
            .get_typed(KEY_WORKER_SLOTS)
            .await
            .unwrap_or_default();

        Ok(json!({
            "tickets": tickets,
            "slots": slots,
        }))
    }

    /// Phase 2: Compute assignments.
    async fn exec(&self, prep_result: Value) -> Result<Value> {
        info!("Nexus orchestrating assignments...");
        
        let mut tickets: Vec<Ticket> = serde_json::from_value(prep_result["tickets"].clone())?;
        let mut slots: HashMap<String, WorkerSlot> = serde_json::from_value(prep_result["slots"].clone())?;
        
        let mut assigned_any = false;

        // FIFO Assignment
        for slot in slots.values_mut() {
            if matches!(slot.status, WorkerStatus::Idle) {
                if let Some(ticket) = tickets.iter().find(|t| t.branch.is_none()) {
                    let ticket_id = ticket.id.clone();
                    info!(worker = slot.id, ticket = ticket_id, "Assigning ticket");
                    slot.status = WorkerStatus::Assigned { ticket_id: ticket_id.clone() };
                    
                    if let Some(t) = tickets.iter_mut().find(|t| t.id == ticket_id) {
                        t.branch = Some(format!("forge/{}/{}", slot.id, ticket_id));
                    }
                    assigned_any = true;
                }
            }
        }

        Ok(json!({
            "assigned_any": assigned_any,
            "tickets": tickets,
            "slots": slots,
        }))
    }

    /// Phase 3: Update store and decide next Action.
    async fn post(&self, store: &SharedStore, exec_result: Value) -> Result<Action> {
        let assigned_any = exec_result["assigned_any"].as_bool().unwrap_or(false);
        let tickets      = &exec_result["tickets"];
        let slots        = &exec_result["slots"];

        store.set(KEY_TICKETS, tickets.clone()).await;
        store.set(KEY_WORKER_SLOTS, slots.clone()).await;

        if assigned_any {
            Ok(Action::new(ACTION_WORK_ASSIGNED))
        } else {
            let slots_map: HashMap<String, WorkerSlot> = serde_json::from_value(slots.clone())?;
            let working = slots_map.values().any(|s| {
                matches!(s.status, WorkerStatus::Working { .. } | WorkerStatus::Assigned { .. })
            });
            
            if working {
                // If workers are active, we move to Working state but with no NEW work assigned
                Ok(Action::new(ACTION_WORK_ASSIGNED))
            } else {
                Ok(Action::new(ACTION_NO_WORK))
            }
        }
    }
}
