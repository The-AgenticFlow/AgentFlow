use agent_forge::ForgeNode;
use agent_nexus::NexusNode;
use anyhow::Result;
use config::{
    ACTION_FAILED, ACTION_NO_WORK, ACTION_PR_OPENED, ACTION_WORK_ASSIGNED, KEY_TICKETS,
    KEY_WORKER_SLOTS,
};
use pocketflow_core::{Flow, SharedStore};
use std::path::PathBuf;
use std::sync::Arc;
use tracing::info;

#[tokio::main]
async fn main() -> Result<()> {
    dotenvy::dotenv().ok();
    tracing_subscriber::fmt::init();

    info!("Starting REAL End-to-End Orchestration (No Mocks)");

    // 1. Validate Environment
    let _token = std::env::var("GITHUB_PERSONAL_ACCESS_TOKEN")
        .expect("GITHUB_PERSONAL_ACCESS_TOKEN must be set");
    let repo = std::env::var("GITHUB_REPOSITORY")
        .expect("GITHUB_REPOSITORY must be set (e.g. owner/repo)");

    // Ensure LLM provider is set for AgentRunner
    if std::env::var("LLM_PROVIDER").is_err() {
        std::env::set_var("LLM_PROVIDER", "openai");
    }

    let workspace_root = configured_test_workdir()?;
    let persona_path = workspace_root
        .join(".agent")
        .join("agents")
        .join("nexus.agent.md");
    let registry_path = workspace_root.join(".agent").join("registry.json");

    info!(
        "Using agent test working directory: {}",
        workspace_root.display()
    );

    // 2. Initialize Nodes
    let nexus = Arc::new(NexusNode::new(persona_path, registry_path));
    let forge = Arc::new(ForgeNode::new(&workspace_root));

    // 3. Setup Flow with Routing
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
                ("suspended", "nexus"),
            ],
        );

    // 4. Initialize Shared Store
    let store = SharedStore::new_in_memory();
    store.set("repository", serde_json::json!(repo)).await;

    // Initial tickets list - Nexus will fetch from GitHub if this is empty
    store.set(KEY_TICKETS, serde_json::json!([])).await;
    store.set(KEY_WORKER_SLOTS, serde_json::json!({})).await;

    // 5. Run Flow
    info!("Running orchestration loop for repository: {}", repo);

    let final_action = flow.run(&store).await?;

    info!("Orchestration flow halted with action: {}", final_action);

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
        .expect("binary manifest should live directly under the workspace root")
        .to_path_buf()
}
