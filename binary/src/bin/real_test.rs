use anyhow::Result;
use pocketflow_core::{Flow, SharedStore};
use agent_nexus::NexusNode;
use agent_forge::ForgePairNode;  // Use the event-driven pair node
use pair_harness::WorkspaceManager;
use config::{KEY_WORKER_SLOTS, KEY_TICKETS, ACTION_WORK_ASSIGNED, ACTION_PR_OPENED, ACTION_FAILED, ACTION_NO_WORK};
use std::sync::Arc;
use tracing::info;

#[tokio::main]
async fn main() -> Result<()> {
    dotenvy::dotenv().ok();
    tracing_subscriber::fmt::init();

    info!("Starting REAL End-to-End Orchestration (Event-Driven FORGE-SENTINEL Pairs)");

    // 1. Validate Environment
    let github_token = std::env::var("GITHUB_PERSONAL_ACCESS_TOKEN")
        .expect("GITHUB_PERSONAL_ACCESS_TOKEN must be set");
    let repo = std::env::var("GITHUB_REPOSITORY")
        .expect("GITHUB_REPOSITORY must be set (e.g. owner/repo)");
    
    // Ensure LLM provider is set for AgentRunner
    if std::env::var("LLM_PROVIDER").is_err() {
        std::env::set_var("LLM_PROVIDER", "openai");
    }

    // 2. Clone/Update the target repository workspace
    // Use ~/.agentflow/workspaces as base directory for all workspaces
    let home = std::env::var("HOME")
        .or_else(|_| std::env::var("USERPROFILE"))
        .expect("Could not determine home directory");
    let workspaces_base = std::path::PathBuf::from(home).join(".agentflow").join("workspaces");
    
    let workspace_manager = WorkspaceManager::new(&workspaces_base, &repo);
    let workspace_dir = workspace_manager.ensure_workspace(&github_token).await?;
    
    info!(workspace = %workspace_dir.display(), "Target repository workspace ready");

    // 3. Initialize Nodes
    // NEXUS: Orchestrator that assigns work
    // ForgePairNode: Event-driven FORGE-SENTINEL pair with full review lifecycle
    let orchestrator_dir = std::env::current_dir()?;
    let persona_path = orchestrator_dir.join(".agent").join("agents").join("nexus.agent.md");
    let registry_path = orchestrator_dir.join(".agent").join("registry.json");
    
    let nexus = Arc::new(NexusNode::new(persona_path, registry_path));
    let forge_pair = Arc::new(ForgePairNode::new(&workspace_dir, &github_token));
    
    // 4. Setup Flow with Routing
    // The ForgePairNode handles the full FORGE-SENTINEL lifecycle:
    // - FORGE writes PLAN.md -> SENTINEL reviews -> CONTRACT.md
    // - FORGE implements segments -> SENTINEL evaluates -> segment-N-eval.md
    // - SENTINEL final review -> final-review.md
    // - FORGE opens PR -> STATUS.json
    let flow = Flow::new("nexus")
        .add_node("nexus", nexus, vec![
            (ACTION_WORK_ASSIGNED, "forge_pair"),
            (ACTION_NO_WORK, "nexus"),
            ("approve_command", "forge_pair"),
            ("reject_command", "nexus"),
        ])
        .add_node("forge_pair", forge_pair, vec![
            (ACTION_PR_OPENED, "nexus"),
            (ACTION_FAILED, "nexus"),
            ("suspended", "nexus"),
        ]);

    // 5. Initialize Shared Store
    let store = SharedStore::new_in_memory();
    store.set("repository", serde_json::json!(repo)).await;
    
    // Initial tickets list - Nexus will fetch from GitHub if this is empty
    store.set(KEY_TICKETS, serde_json::json!([])).await;
    store.set(KEY_WORKER_SLOTS, serde_json::json!({})).await;

    // 6. Run Flow
    info!("Running orchestration loop for repository: {}", repo);
    info!("Each worker will use event-driven FORGE-SENTINEL pair with:");
    info!("  - PLAN.md -> CONTRACT.md (plan review)");
    info!("  - WORKLOG.md -> segment-N-eval.md (segment evaluation)");
    info!("  - final-review.md (final approval)");
    info!("  - STATUS.json (completion status)");
    
    let final_action = flow.run(&store).await?;
    
    info!("Orchestration flow halted with action: {}", final_action);

    Ok(())
}
