use agent_nexus::NexusNode;
use anyhow::Result;
use pocketflow_core::SharedStore;
use pocketflow_core::Node;
use serde_json::json;
use config::{KEY_TICKETS, Ticket, TicketStatus};

#[tokio::test]
async fn test_dependency_chain_waits_and_releases() -> Result<()> {
    // In-memory store for deterministic test
    let store = SharedStore::new_in_memory();

    // Create two tickets: T-001 and T-002 where T-002 depends on T-001
    let t1 = Ticket {
        id: "T-001".to_string(),
        title: "First".to_string(),
        body: "".to_string(),
        priority: 1,
        branch: None,
        status: TicketStatus::Open,
        issue_url: None,
        attempts: 0,
        depends_on: vec![],
    };

    let t2 = Ticket {
        id: "T-002".to_string(),
        title: "Second".to_string(),
        body: "".to_string(),
        priority: 1,
        branch: None,
        status: TicketStatus::Open,
        issue_url: None,
        attempts: 0,
        depends_on: vec!["T-001".to_string()],
    };

    store.set(KEY_TICKETS, json!([t1, t2])).await;

    let nexus = NexusNode::new("orchestration/agent/agents/nexus.agent.md", "orchestration/agent/registry.json");

    // First prep: should mark T-002 as waiting and only T-001 assignable
    let v = nexus.prep(&store).await?;

    // Inspect stored tickets
    let tickets: Vec<Ticket> = store.get_typed(KEY_TICKETS).await.unwrap();
    let t1 = tickets.iter().find(|t| t.id == "T-001").unwrap();
    let t2 = tickets.iter().find(|t| t.id == "T-002").unwrap();

    // T-001 should remain open and T-002 should be waiting_on_dependency
    assert!(matches!(t1.status, TicketStatus::Open));
    match &t2.status {
        TicketStatus::WaitingOnDependency { depends_on } => {
            assert_eq!(depends_on, &vec!["T-001".to_string()]);
        }
        _ => panic!("T-002 should be WaitingOnDependency"),
    }

    // returned assignable_tickets should contain only T-001
    let assignable = v["assignable_tickets"].as_array().unwrap();
    assert_eq!(assignable.len(), 1);
    assert_eq!(assignable[0]["id"].as_str().unwrap(), "T-001");

    // Simulate VESSEL merging T-001 by emitting ticket_merged event
    store.emit("vessel", "ticket_merged", json!({ "ticket_id": "T-001" })).await;

    // Second prep: T-002 should become assignable now
    let v2 = nexus.prep(&store).await?;
    let assignable2 = v2["assignable_tickets"].as_array().unwrap();
    // Should now contain T-002 as assignable (and possibly T-001 depending on logic)
    assert!(assignable2.iter().any(|t| t["id"].as_str() == Some("T-002")));

    Ok(())
}
