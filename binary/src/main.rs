// binary/src/main.rs
mod nodes;
mod state;

use anyhow::Result;
use pocketflow_core::{Flow, SharedStore};
use std::collections::HashMap;
use std::sync::Arc;
use tracing::info;

use crate::nodes::{ForgeNode, NexusNode};
use crate::state::{
    Ticket, WorkerSlot, WorkerStatus, ACTION_EMPTY, ACTION_FAILED, ACTION_NO_WORK,
    ACTION_PR_OPENED, ACTION_WORK_ASSIGNED, KEY_TICKETS, KEY_WORKER_SLOTS,
};

#[tokio::main]
async fn main() -> Result<()> {
    dotenvy::dotenv().ok();
    tracing_subscriber::fmt::init();
    // 1. Load environment and tracing
    // The original tracing setup was more elaborate, but the instruction simplifies it.
    // let _ = dotenvy::dotenv(); // Replaced by dotenvy::dotenv().ok();
    // tracing_subscriber::fmt()
    //     .with_env_filter(std::env::var("RUST_LOG").unwrap_or_else(|_| "info,agent_team=debug,pocketflow_core=debug".to_string()))
    //     .init();

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
        (
            "forge-1".to_string(),
            WorkerSlot {
                id: "forge-1".to_string(),
                status: WorkerStatus::Idle,
            },
        ),
        (
            "forge-2".to_string(),
            WorkerSlot {
                id: "forge-2".to_string(),
                status: WorkerStatus::Idle,
            },
        ),
    ]);

    store
        .set(KEY_TICKETS, serde_json::to_value(vec![test_ticket])?)
        .await;
    store
        .set(KEY_WORKER_SLOTS, serde_json::to_value(worker_slots)?)
        .await;

    // 4. Build Flow
    let nexus = Arc::new(NexusNode::new(
        ".agent/agents/nexus.agent.md",
        ".agent/registry.json",
    ));
    let forge = Arc::new(ForgeNode::new("."));

    let flow = Flow::new("nexus")
        .add_node(
            "nexus",
            nexus,
            vec![
                (ACTION_WORK_ASSIGNED, "forge"),
                (ACTION_NO_WORK, "nexus"),
                ("approve_command", "forge"),
                ("reject_command", "nexus"),
            ],
        )
        .add_node(
            "forge",
            forge,
            vec![
                (ACTION_PR_OPENED, "nexus"),
                (ACTION_FAILED, "nexus"),
                (ACTION_EMPTY, "nexus"),
                ("suspended", "nexus"),
            ],
        )
        .max_steps(20); // allow more steps for real logic

    // 5. Run Flow
    info!("Starting Flow execution loop...");
    let _final_action = flow.run(&store).await?;

    // 6. Results
    let final_slots: HashMap<String, WorkerSlot> =
        store.get_typed(KEY_WORKER_SLOTS).await.unwrap_or_default();

    for slot in final_slots.values() {
        info!(worker = slot.id, status = ?slot.status, "Final worker status");
    }

    info!("🏁 Phase 3 Dry Run complete.");
    Ok(())
}
