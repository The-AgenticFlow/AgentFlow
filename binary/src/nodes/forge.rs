// binary/src/nodes/forge.rs
use anyhow::Result;
use async_trait::async_trait;
use pocketflow_core::{BatchNode, SharedStore, Action};
use serde_json::{json, Value};
use tracing::{info, warn};
use std::collections::HashMap;

use crate::state::{
    WorkerSlot, WorkerStatus,
    KEY_WORKER_SLOTS, ACTION_PR_OPENED, ACTION_FAILED, ACTION_EMPTY
};

pub struct ForgeNode;

#[async_trait]
impl BatchNode for ForgeNode {
    fn name(&self) -> &str { "forge" }

    async fn prep_batch(&self, store: &SharedStore) -> Result<Vec<Value>> {
        let slots: HashMap<String, WorkerSlot> = store
            .get_typed(KEY_WORKER_SLOTS)
            .await
            .unwrap_or_default();

        // Find slots that are Assigned or Working
        let active_workers: Vec<Value> = slots.values()
            .filter(|s| matches!(s.status, WorkerStatus::Assigned { .. } | WorkerStatus::Working { .. }))
            .map(|s| json!(s))
            .collect();
        
        Ok(active_workers)
    }

    async fn exec_one(&self, item: Value) -> Result<Value> {
        let slot: WorkerSlot = serde_json::from_value(item)?;
        let worker_id = slot.id.clone();
        
        info!(worker = worker_id, "Forge worker starting work");

        let ticket_id = match &slot.status {
            WorkerStatus::Assigned { ticket_id } => ticket_id.clone(),
            WorkerStatus::Working { ticket_id }  => ticket_id.clone(),
            _ => return Ok(json!({"outcome": "idle", "worker_id": worker_id})),
        };

        // 2. Mocking Claude Code execution
        info!(worker = worker_id, ticket = ticket_id, "Spawning Claude Code (Mock)");
        
        // Simulating work
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;

        // 3. Return outcome
        Ok(json!({
            "worker_id": worker_id,
            "ticket_id": ticket_id,
            "outcome": "pr_opened",
            "pr_url": format!("https://github.com/test/repo/pull/1")
        }))
    }

    async fn post_batch(
        &self,
        store: &SharedStore,
        results: Vec<Result<Value>>,
    ) -> Result<Action> {
        let res_list = results;

        let mut slots: HashMap<String, WorkerSlot> = store
            .get_typed(KEY_WORKER_SLOTS)
            .await
            .unwrap_or_default();

        let mut all_success = true;

        for res_opt in &res_list {
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
            let outcome   = res["outcome"].as_str().unwrap_or("failed");

            if let Some(slot) = slots.get_mut(worker_id) {
                if outcome == "pr_opened" {
                    info!(worker = worker_id, ticket = ticket_id, "Work completed successfully");
                    slot.status = WorkerStatus::Done { 
                        ticket_id: ticket_id.to_string(), 
                        outcome: outcome.to_string() 
                    };
                } else if outcome != "idle" {
                    warn!(worker = worker_id, ticket = ticket_id, "Work failed");
                    slot.status = WorkerStatus::Idle;
                    all_success = false;
                }
            }
        }

        store.set(KEY_WORKER_SLOTS, json!(slots)).await;

        if all_success && !res_list.is_empty() {
            Ok(Action::new(ACTION_PR_OPENED))
        } else if res_list.is_empty() {
            Ok(Action::new(ACTION_EMPTY))
        } else {
            Ok(Action::new(ACTION_FAILED))
        }
    }
}
