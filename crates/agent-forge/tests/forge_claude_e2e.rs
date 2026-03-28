use agent_forge::ForgeNode;
use anyhow::Result;
use config::{WorkerSlot, WorkerStatus};
use pocketflow_core::{BatchNode, SharedStore};
use serde_json::json;
use std::collections::HashMap;
use std::path::PathBuf;

#[tokio::test]
async fn test_forge_dangerous_command_suspends() -> Result<()> {
    let _ = dotenvy::dotenv();

    // 1. Setup SharedStore
    let store = SharedStore::new_in_memory();

    // Inject a worker slot with "danger" in the ticket ID (our mock uses this)
    let worker_id = "forge-1";
    let ticket_id = "T-DANGER-001";
    let slots = HashMap::from([(
        worker_id.to_string(),
        WorkerSlot {
            id: worker_id.to_string(),
            status: WorkerStatus::Working {
                ticket_id: ticket_id.to_string(),
                issue_url: None,
            },
        },
    )]);
    store.set("worker_slots", json!(slots)).await;

    // 2. Setup ForgeNode with a mock claude
    // We'll point the PATH to our mock script
    let workspace_root = configured_test_workdir()?;
    let scripts_dir = workspace_root.join("scripts");
    let worker_dir = workspace_root.join("forge").join("workers").join(worker_id);

    println!("Forge test working directory: {}", workspace_root.display());
    println!("Forge live worker dir: {}", worker_dir.display());

    // Create a temporary PATH with our mock script as 'claude'
    let mock_claude_path = scripts_dir.join("mock_claude.py");
    // Make it executable
    std::process::Command::new("chmod")
        .arg("+x")
        .arg(&mock_claude_path)
        .spawn()?
        .wait()?;

    // We'll use a hack: set an env var CLAUDE_CMD for our node to use if we update it
    // Or just symlink it in a temp dir. Let's update ForgeNode to use an env var for the binary name.

    // Actually, I'll just use the mock script path directly in a wrapper if needed,
    // but for now let's assume we can mock it via PATH.
    let temp_dir = tempfile::tempdir()?;
    let bin_dir = temp_dir.path().join("bin");
    std::fs::create_dir(&bin_dir)?;
    std::fs::copy(&mock_claude_path, bin_dir.join("claude"))?;

    let old_path = std::env::var("PATH").unwrap_or_default();
    let new_path = format!("{}:{}", bin_dir.display(), old_path);
    std::env::set_var("PATH", new_path);

    // 3. Run ForgeNode
    let forge = ForgeNode::new(&workspace_root);

    // Prep
    let items = forge.prep_batch(&store).await?;
    assert_eq!(items.len(), 1);

    // Exec
    let result = forge.exec_one(items[0].clone()).await?;
    assert_eq!(result["outcome"], "suspended");
    assert_eq!(result["reason"], "dangerous_command");

    // Post
    let action = forge.post_batch(&store, vec![Ok(result)]).await?;
    assert_eq!(action.as_str(), "suspended");

    // 4. Verify Store
    let final_slots: HashMap<String, WorkerSlot> = store
        .get_typed("worker_slots")
        .await
        .ok_or_else(|| anyhow::anyhow!("No worker_slots in store"))?;
    let slot = final_slots.get(worker_id).unwrap();
    assert!(matches!(slot.status, WorkerStatus::Suspended { .. }));

    // Cleanup PATH
    std::env::set_var("PATH", old_path);

    Ok(())
}

fn configured_test_workdir() -> Result<PathBuf> {
    let root = std::env::var("AGENT_TEST_WORKDIR")
        .ok()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
        .map(PathBuf::from)
        .unwrap_or_else(default_workspace_root);

    Ok(root.canonicalize().unwrap_or(root))
}

fn default_workspace_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(|path| path.parent())
        .expect("agent-forge manifest should live under crates/")
        .to_path_buf()
}
