use anyhow::Result;
use pocketflow_core::{Flow, SharedStore};
use agent_nexus::NexusNode;
use agent_forge::ForgeNode;
use config::{KEY_WORKER_SLOTS, KEY_TICKETS, ACTION_WORK_ASSIGNED, ACTION_PR_OPENED, ACTION_FAILED, ACTION_NO_WORK};
use std::sync::Arc;
use tracing::info;

#[tokio::main]
async fn main() -> Result<()> {
    dotenvy::dotenv().ok();
    tracing_subscriber::fmt::init();

    info!("Starting REAL End-to-End Orchestration (No Mocks)");

    // 1. Validate Environment
    let _token = std::env::var("GITHUB_PERSONAL_ACCESS_TOKEN").expect("GITHUB_PERSONAL_ACCESS_TOKEN must be set");
    let repo = std::env::var("GITHUB_REPOSITORY").expect("GITHUB_REPOSITORY must be set (e.g. owner/repo)");
    
    // Ensure LLM provider is set for AgentRunner
    if std::env::var("LLM_PROVIDER").is_err() {
        std::env::set_var("LLM_PROVIDER", "openai");
    }

    let workspace_root = std::env::current_dir()?;
    let persona_path = workspace_root.join(".agent").join("agents").join("nexus.agent.md");
    let registry_path = workspace_root.join(".agent").join("registry.json");

    // 2. Initialize Nodes
    let nexus = Arc::new(NexusNode::new(persona_path, registry_path));
    let forge = Arc::new(ForgeNode::new(&workspace_root));
    
    // 3. Setup Flow with Routing
    let flow = Flow::new("nexus")
        .add_node("nexus", nexus, vec![
            (ACTION_WORK_ASSIGNED, "forge"),
            (ACTION_NO_WORK, "nexus"),
            ("approve_command", "forge"),
            ("reject_command", "nexus"),
        ])
        .add_node("forge", forge, vec![
            (ACTION_PR_OPENED, "nexus"),
            (ACTION_FAILED, "nexus"),
            ("suspended", "nexus"),
        ]);

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
