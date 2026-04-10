use agent_forge::ForgeNode;
use agent_nexus::NexusNode;
use anyhow::Result;
use config::{
    Ticket, TicketStatus, WorkerSlot, ACTION_EMPTY, ACTION_FAILED, ACTION_NO_WORK, ACTION_PR_OPENED,
    ACTION_WORK_ASSIGNED, KEY_TICKETS, KEY_WORKER_SLOTS,
};
use dotenvy;
use pocketflow_core::{Flow, SharedStore};
use std::collections::HashMap;
use std::sync::Arc;
use tracing::{info, Level};
use tracing_subscriber::FmtSubscriber;

#[tokio::main]
async fn main() -> Result<()> {
    match dotenvy::dotenv() {
        Ok(path) => eprintln!("Loaded environment from {}", path.display()),
        Err(dotenvy::Error::Io(err)) if err.kind() == std::io::ErrorKind::NotFound => {}
        Err(err) => return Err(err.into()),
    }
    // 1. Setup logging
    let subscriber = FmtSubscriber::builder()
        .with_max_level(Level::INFO)
        .finish();
    tracing::subscriber::set_global_default(subscriber)?;

    info!("Starting Autonomous Agent Team Demo...");

    // 2. Setup Environment for Mocks
    // If you have real keys, comment these out!
    std::env::set_var("ANTHROPIC_API_KEY", "test-key");
    std::env::set_var("ANTHROPIC_API_URL", "http://localhost:8082"); // assume mock server if running, else we need a local mock
    std::env::set_var("GITHUB_MCP_CMD", "python3 scripts/mock_mcp.py");
    std::env::set_var("GITHUB_PERSONAL_ACCESS_TOKEN", "test-token");

    // Ensure scripts/mock_claude.py is in PATH as 'claude'
    let repo_root = std::env::current_dir()?;
    let bin_dir = repo_root.join("target/debug/test_bin");
    std::fs::create_dir_all(&bin_dir)?;
    std::fs::copy("scripts/mock_claude.py", bin_dir.join("claude"))?;
    let old_path = std::env::var("PATH").unwrap_or_default();
    std::env::set_var("PATH", format!("{}:{}", bin_dir.display(), old_path));

    // 3. Initialize Store
    let store = SharedStore::new_in_memory();

    // Inject a sample ticket if list_issues is empty (our mock returns one, but let's be safe)
    let ticket = Ticket {
        id: "T-101".to_string(),
        title: "Implement auth middleware".to_string(),
        body: "We need a JWT middleware in src/auth.rs".to_string(),
        priority: 1,
        branch: None,
        status: TicketStatus::Open,
        issue_url: None,
        attempts: 0,
    };
    store
        .set(KEY_TICKETS, serde_json::json!(vec![ticket]))
        .await;

    // 4. Build Nodes
    let nexus = Arc::new(NexusNode::new(
        ".agent/agents/nexus.agent.md",
        ".agent/registry.json",
    ));
    let forge = Arc::new(ForgeNode::new(".", ".agent/agents/forge.agent.md"));

    // 5. Build Flow
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
        .max_steps(5);

    // 6. Run Flow manually to show store updates
    let current_node = "nexus".to_string();
    for step in 0..5 {
        info!("--- STEP {}: Node {} ---", step, current_node);

        // Print store BEFORE
        let slots: HashMap<String, WorkerSlot> =
            store.get_typed(KEY_WORKER_SLOTS).await.unwrap_or_default();
        info!("Worker Slots BEFORE: {:?}", slots);

        // Run the node once (logic simplified from Flow::run)
        let _action = if current_node == "nexus" {
            // We need to access the node directly, but Flow abstracts it.
            // For the demo, let's just use the flow.run but with tracing on.
            break; // We'll just use flow.run and let the background info logs show it
        } else {
            break;
        };
    }

    // Actually, flow.run already has good tracing. Let's just use it and add more info level logs.
    info!("Running flow...");
    flow.run(&store).await?;

    info!("Demo completed.");
    let final_slots: HashMap<String, WorkerSlot> =
        store.get_typed(KEY_WORKER_SLOTS).await.unwrap_or_default();
    info!("Final Worker Slots: {:?}", final_slots);

    Ok(())
}
