use anyhow::Result;
use config::state::{KEY_TICKETS, KEY_WORKER_SLOTS, WorkerSlot, WorkerStatus};
use pocketflow_core::{BatchNode, SharedStore};
use serde_json::json;
use std::sync::atomic::Ordering;
use tempfile::tempdir;

use pair_harness::SIMULATED_MAX_ACTIVE_PAIRS;
use agent_forge::ForgePairNode;

#[tokio::test]
async fn test_two_pairs_run_concurrently() -> Result<()> {
    // Configure test-mode pair simulation and concurrency
    std::env::set_var("AGENT_FLOW_TEST_PAIR_DELAY_MS", "300");
    std::env::set_var("AGENT_FORGE_MAX_CONCURRENCY", "2");

    // Create node and in-memory store
    let workspace = tempdir()?;
    let node = ForgePairNode::new(workspace.path(), "ghp_test_token");
    let store = SharedStore::new_in_memory();

    // Prepare two assigned worker slots
    let mut slots = std::collections::HashMap::new();

    slots.insert(
        "forge-1".to_string(),
        WorkerSlot {
            id: "forge-1".to_string(),
            status: WorkerStatus::Assigned {
                ticket_id: "T-1".to_string(),
                issue_url: None,
            },
        },
    );

    slots.insert(
        "forge-2".to_string(),
        WorkerSlot {
            id: "forge-2".to_string(),
            status: WorkerStatus::Assigned {
                ticket_id: "T-2".to_string(),
                issue_url: None,
            },
        },
    );

    store.set(KEY_WORKER_SLOTS, json!(slots)).await;
    store.set(KEY_TICKETS, json!([])).await;

    // Run the batch — this should process two workers concurrently (simulated)
    let action = node.run_batch(&store).await?;

    // Ensure the batch completed with expected action (pr_opened or similar)
    // We primarily assert concurrency via the simulated max counter.
    let max_active = SIMULATED_MAX_ACTIVE_PAIRS.load(Ordering::SeqCst);
    assert!(max_active >= 2, "Expected at least 2 concurrent pairs, got {}", max_active);

    // Cleanup test env
    std::env::remove_var("AGENT_FLOW_TEST_PAIR_DELAY_MS");
    std::env::remove_var("AGENT_FORGE_MAX_CONCURRENCY");

    // Basic sanity: action should not be EMPTY
    assert_ne!(action.as_str(), "empty");

    Ok(())
}
