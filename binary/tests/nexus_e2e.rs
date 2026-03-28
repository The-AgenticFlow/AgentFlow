use agent_nexus::NexusNode;
use anyhow::Result;
use mockito::Server;
use pocketflow_core::{Node, SharedStore};
use serde_json::json;
use std::path::PathBuf;
use std::sync::Arc;

#[tokio::test]
async fn test_nexus_e2e_mocked() -> Result<()> {
    let _ = dotenvy::dotenv();

    // 1. Start Mockito server to mock Anthropic API
    let mut server = Server::new_async().await;
    let url = format!("{}/v1/messages", server.url());
    let workspace_root = configured_test_workdir()?;
    let mock_mcp_path = workspace_root.join("scripts").join("mock_mcp.py");
    let persona_path = workspace_root
        .join(".agent")
        .join("agents")
        .join("nexus.agent.md");
    let registry_path = workspace_root.join(".agent").join("registry.json");

    println!(
        "Nexus mocked test working directory: {}",
        workspace_root.display()
    );

    // Mock Anthropic response
    // First turn: Tool use (list_issues)
    let _m1 = server
        .mock("POST", "/v1/messages")
        .with_status(200)
        .with_header("content-type", "application/json")
        .with_body(
            json!({
                "id": "msg_123",
                "type": "message",
                "role": "assistant",
                "content": [{
                    "id": "call_1",
                    "type": "tool_use",
                    "name": "list_issues",
                    "input": {}
                }],
                "stop_reason": "tool_use"
            })
            .to_string(),
        )
        .create_async()
        .await;

    // Second turn: Final decision
    let _m2 = server.mock("POST", "/v1/messages")
        .with_status(200)
        .with_header("content-type", "application/json")
        .with_body(json!({
            "id": "msg_456",
            "type": "message",
            "role": "assistant",
            "content": [{
                "type": "text",
                "text": "I see the issues. Assigning T-001.\n{\"action\": \"work_assigned\", \"notes\": \"Assigned T-001\"}"
            }],
            "stop_reason": "end_turn"
        }).to_string())
        .create_async().await;

    // 2. Setup environment variables for AgentRunner
    std::env::set_var("ANTHROPIC_API_KEY", "test-key");
    std::env::set_var("ANTHROPIC_API_URL", &url);
    std::env::set_var(
        "GITHUB_MCP_CMD",
        format!("python3 {}", mock_mcp_path.display()),
    );
    std::env::set_var("GITHUB_PERSONAL_ACCESS_TOKEN", "test-token");

    // 3. Initialize SharedStore
    let store = SharedStore::new_in_memory();

    // Inject initial worker slots
    let slots = json!({
        "forge-1": { "id": "forge-1", "status": { "type": "idle" } }
    });
    store.set("worker_slots", slots).await;

    // 4. Run NexusNode
    let nexus = Arc::new(NexusNode::new(persona_path, registry_path));

    let action = nexus.run(&store).await?;
    assert_eq!(action.as_str(), "work_assigned");

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
