//! VESSEL agent — performs CI checks and merges PRs. Emits `ticket_merged` event.

use anyhow::{Context, Result};
use pocketflow_core::{node::Node, Action, SharedStore};
use serde_json::json;
use std::path::PathBuf;
use tracing::info;

pub mod node;

/// Convenience helper to announce a merged ticket to the SharedStore and Redis (if configured).
pub async fn emit_ticket_merged(store: &SharedStore, ticket_id: &str, pr_url: Option<&str>) -> Result<()> {
    // Emit in-memory event for local consumers
    let payload = json!({ "ticket_id": ticket_id, "pr_url": pr_url });
    store.emit("vessel", "ticket_merged", payload.clone()).await;

    // If REDIS_URL present, also write to Redis list (RPUSH) for cross-process consumers.
    if let Ok(url) = std::env::var("REDIS_URL") {
        use fred::prelude::*;

        let cfg = Config::from_url(&url).context("Invalid REDIS_URL")?;
        let client = Builder::from_config(cfg).build()?;
        client.init().await?;

        // Append JSON payload to a well-known list 'ticket_merged'
        let s = serde_json::to_string(&payload)?;
        // RPUSH returns the new length of the list (integer). Specify `i64` as
        // the expected return type so fred can resolve its generic `R`.
        let _ = client.rpush::<i64, _, _>("ticket_merged", s).await;
        // Best-effort: ignore errors here
    }

    info!(ticket = ticket_id, "ticket_merged emitted");
    Ok(())
}

pub use node::VesselNode;
