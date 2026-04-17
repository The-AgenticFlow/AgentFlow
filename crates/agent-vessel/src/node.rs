use anyhow::Result;
use async_trait::async_trait;
use pocketflow_core::{Action, Node, SharedStore};
use serde_json::{json, Value};
use std::path::PathBuf;
use tracing::info;

pub struct VesselNode {
    pub workspace_root: PathBuf,
}

impl VesselNode {
    pub fn new(workspace_root: impl Into<PathBuf>) -> Self {
        Self {
            workspace_root: workspace_root.into(),
        }
    }
}

#[async_trait]
impl Node for VesselNode {
    fn name(&self) -> &str {
        "vessel"
    }

    async fn prep(&self, _store: &SharedStore) -> Result<Value> {
        Ok(json!({ "note": "VESSEL node ready" }))
    }

    async fn exec(&self, prep_result: Value) -> Result<Value> {
        // In a full implementation this node would check CI and perform the merge.
        Ok(json!({ "action": "merge", "details": prep_result }))
    }

    async fn post(&self, store: &SharedStore, result: Value) -> Result<Action> {
        // If the exec produced a merge, emit ticket_merged
        if result["action"].as_str() == Some("merge") {
            let ticket_id = result["ticket_id"].as_str().unwrap_or("");
            let pr_url = result["pr_url"].as_str();
            if !ticket_id.is_empty() {
                let _ = crate::emit_ticket_merged(store, ticket_id, pr_url).await;
            }
        }

        info!("Vessel post complete");
        Ok(Action::new("merged"))
    }
}
