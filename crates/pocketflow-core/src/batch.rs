// crates/pocketflow-core/src/batch.rs
//
// BatchNode — used by ForgeWorkerPool.
// prep_batch returns one item per available worker slot.
// exec_one runs concurrently across all items via tokio::spawn.
// post_batch collects all results and writes to the store.

use anyhow::Result;
use async_trait::async_trait;
use futures::future::join_all;
use serde_json::Value;
use tracing::info;

use crate::{Action, SharedStore};

#[async_trait]
pub trait BatchNode: Send + Sync {
    fn name(&self) -> &str;

    /// Return one work item per available parallel slot.
    async fn prep_batch(&self, store: &SharedStore) -> Result<Vec<Value>>;

    /// Process one item — runs concurrently. Must be safe to call in parallel.
    async fn exec_one(&self, item: Value) -> Result<Value>;

    /// Collect all results, write to store, return routing Action.
    async fn post_batch(&self, store: &SharedStore, results: Vec<Result<Value>>) -> Result<Action>;

    /// Orchestrated run — returns immediately if no items to process.
    async fn run_batch(&self, store: &SharedStore) -> Result<Action> {
        let name = self.name();

        store
            .emit(name, "batch_prep_started", serde_json::json!({}))
            .await;

        let items = self.prep_batch(store).await?;
        let item_count = items.len();

        if item_count == 0 {
            info!(node = name, "batch has no items — skipping exec");
            store.emit(name, "batch_empty", serde_json::json!({})).await;
            // Return a no-tickets action to let the orchestrator loop
            return Ok(Action::new(crate::action::Action::NO_TICKETS));
        }

        store
            .emit(
                name,
                "batch_exec_started",
                serde_json::json!({ "items": item_count }),
            )
            .await;

        // Run all items concurrently using futures (no tokio::spawn — avoids 'static bound)
        let futures: Vec<_> = items.into_iter().map(|item| self.exec_one(item)).collect();

        let results: Vec<Result<Value>> = join_all(futures).await;

        store
            .emit(
                name,
                "batch_exec_done",
                serde_json::json!({ "items": item_count }),
            )
            .await;

        let action = self.post_batch(store, results).await?;

        store
            .emit(
                name,
                "batch_done",
                serde_json::json!({ "action": action.as_str() }),
            )
            .await;
        info!(
            node = name,
            action = action.as_str(),
            items = item_count,
            "batch completed"
        );

        Ok(action)
    }
}

/// Blanket implementation: any BatchNode can be treated as a regular Node
/// by executing its batch orchestration logic.
#[async_trait]
impl<T: BatchNode + ?Sized> crate::Node for T {
    fn name(&self) -> &str {
        self.name()
    }

    async fn prep(&self, _store: &SharedStore) -> Result<Value> {
        Ok(Value::Null) // Not used for BatchNode wrapper
    }

    async fn exec(&self, _prep: Value) -> Result<Value> {
        Ok(Value::Null) // Not used for BatchNode wrapper
    }

    async fn post(&self, _store: &SharedStore, _res: Value) -> Result<Action> {
        Ok(Action::new("error")) // Not used for BatchNode wrapper
    }

    /// Override the default run() to use run_batch()
    async fn run(&self, store: &SharedStore) -> Result<Action> {
        self.run_batch(store).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::SharedStore;

    struct DoubleNode;

    #[async_trait]
    impl BatchNode for DoubleNode {
        fn name(&self) -> &str {
            "double"
        }

        async fn prep_batch(&self, _store: &SharedStore) -> Result<Vec<Value>> {
            Ok(vec![
                serde_json::json!(1),
                serde_json::json!(2),
                serde_json::json!(3),
            ])
        }

        async fn exec_one(&self, item: Value) -> Result<Value> {
            let n = item.as_i64().unwrap_or(0);
            Ok(serde_json::json!(n * 2))
        }

        async fn post_batch(
            &self,
            store: &SharedStore,
            results: Vec<Result<Value>>,
        ) -> Result<Action> {
            let values: Vec<i64> = results
                .into_iter()
                .filter_map(|r| r.ok())
                .filter_map(|v| v.as_i64())
                .collect();
            store.set("doubled", serde_json::json!(values)).await;
            Ok(Action::new("done"))
        }
    }

    #[tokio::test]
    async fn test_batch_execution() {
        let store = SharedStore::new_in_memory();
        let node = DoubleNode;

        let action = node.run_batch(&store).await.unwrap();
        assert_eq!(action.as_str(), "done");

        let result = store.get("doubled").await.unwrap();
        let mut values: Vec<i64> = serde_json::from_value(result).unwrap();
        values.sort();
        assert_eq!(values, vec![2, 4, 6]);
    }
}
