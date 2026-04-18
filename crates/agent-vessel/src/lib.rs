//! VESSEL agent — performs CI checks and merges PRs. Emits `ticket_merged` event.

use anyhow::Result;
use pocketflow_core::SharedStore;
use serde_json::json;
use tracing::info;

pub mod node;

/// Convenience helper to announce a merged ticket to the SharedStore and Redis (if configured).
pub async fn emit_ticket_merged(store: &SharedStore, ticket_id: &str, pr_url: Option<&str>) -> Result<()> {
    // Emit in-memory event for local consumers
    let payload = json!({ "ticket_id": ticket_id, "pr_url": pr_url });
    store.emit("vessel", "ticket_merged", payload.clone()).await;
    info!(ticket = ticket_id, "ticket_merged emitted");
    Ok(())
}

pub use node::VesselNode;
