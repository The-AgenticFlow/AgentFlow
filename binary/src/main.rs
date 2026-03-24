// binary/src/main.rs
mod state;
mod nodes;

use anyhow::Result;
use pocketflow_core::{Flow, SharedStore, Action};
use tracing::{info};
use std::collections::HashMap;
use std::sync::Arc;

use crate::state::{
    Ticket, WorkerSlot, WorkerStatus,
    KEY_TICKETS, KEY_WORKER_SLOTS,
    ACTION_WORK_ASSIGNED, ACTION_PR_OPENED, ACTION_NO_WORK, ACTION_EMPTY, ACTION_FAILED
};
use crate::nodes::{NexusNode, ForgeNode};

#[tokio::main]
async fn main() -> Result<()> {
    // 1. Load environment and tracing
    let _ = dotenvy::dotenv();
    tracing_subscriber::fmt()
        .with_env_filter(std::env::var("RUST_LOG").unwrap_or_else(|_| "info,agent_team=debug,pocketflow_core=debug".to_string()))
        .init();

    info!("🚀 Autonomous AI Dev Team starting (Phase 3 Integration)...");

    // 2. Initialise SharedStore (Redis or In-Memory)
    let store = if let Ok(url) = std::env::var("REDIS_URL") {
        info!("Using Redis backend: {}", url);
        SharedStore::new_redis(&url).await?
    } else {
        info!("REDIS_URL not set — using in-memory store (dev mode)");
        SharedStore::new_in_memory()
    };

    // 3. Dry Run Setup: Inject a test ticket and 2 worker slots
    info!("Injecting dry-run data...");
    let test_ticket = Ticket {
        id: "T-001".to_string(),
        title: "Implement landing page glassmorphism".to_string(),
        body: "Add a new CSS class for glassmorphism and apply to the hero section.".to_string(),
        priority: 1,
        branch: None,
    };
    
    let worker_slots = HashMap::from([
        ("forge-1".to_string(), WorkerSlot { id: "forge-1".to_string(), status: WorkerStatus::Idle }),
        ("forge-2".to_string(), WorkerSlot { id: "forge-2".to_string(), status: WorkerStatus::Idle }),
    ]);

    store.set(KEY_TICKETS, serde_json::to_value(vec![test_ticket])?).await;
    store.set(KEY_WORKER_SLOTS, serde_json::to_value(worker_slots)?).await;

    // 4. Build Flow
    let nexus = Arc::new(NexusNode::new(".agent/registry.json"));
    let forge = Arc::new(ForgeNode);

    let flow = Flow::new("nexus")
        .add_node("nexus", nexus, vec![
            (ACTION_WORK_ASSIGNED, "forge"),
            (ACTION_NO_WORK,       "nexus"),
        ])
        .add_node("forge", forge, vec![
            (ACTION_PR_OPENED, "nexus"),
            (ACTION_FAILED,    "nexus"),
            (ACTION_EMPTY,     "nexus"),
        ])
        .max_steps(5); // safety cap for dry run

    // 5. Run Flow
    info!("Starting Flow execution loop...");
    let _final_action = flow.run(&store).await?;

    // 6. Results
    let final_slots: HashMap<String, WorkerSlot> = store
        .get_typed(KEY_WORKER_SLOTS)
        .await
        .unwrap_or_default();

    for slot in final_slots.values() {
        info!(worker = slot.id, status = ?slot.status, "Final worker status");
    }

    info!("🏁 Phase 3 Dry Run complete.");
    Ok(())
}
