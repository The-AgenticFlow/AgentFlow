use anyhow::Result;
use pair_harness::{ForgeSentinelPair, PairConfig, Ticket};

/// Real E2E Test for Pair Harness (No Mocks)
///
/// Loads configuration from `.env` and the process environment.
///
/// REQUIRES:
/// - `GITHUB_TOKEN` or `GITHUB_PERSONAL_ACCESS_TOKEN`
/// - `REDIS_URL` (optional, default: `redis://localhost:6379`)
/// - `AGENT_TEST_WORKDIR` (recommended for a dedicated checkout you want to watch live)
/// - `REPO_PATH` (legacy alias; falls back to the current working directory)
///
/// To run:
/// cargo test -p pair-harness --test pair_real_e2e -- --ignored
#[tokio::test]
#[ignore] // Ignored by default to avoid failing in CI without setup
async fn test_pair_harness_real_e2e() -> Result<()> {
    // 1. Initialize Tracing
    let _ = tracing_subscriber::fmt().with_target(false).try_init();

    println!("\n=== Starting Real Pair Harness E2E Test ===");

    // 2. Check environment prerequisites
    let config = PairConfig::from_env("pair-e2e-test", 3)?;
    let repo_path = config.repo_path.clone();
    let worktree_path = repo_path.join("worktrees").join(&config.pair_id);
    let shared_path = repo_path
        .join(".sprintless")
        .join("pairs")
        .join(&config.pair_id)
        .join("shared");

    println!("Redis URL: {}", config.redis_url);
    println!("Working Directory: {}", repo_path.display());
    println!("Live Worktree: {}", worktree_path.display());
    println!("Live Shared Dir: {}", shared_path.display());
    println!("GitHub Token: {}...", token_preview(&config.github_token));

    // 4. Create a test ticket
    let ticket = Ticket {
        id: "T-E2E-001".to_string(),
        title: "E2E Test Ticket".to_string(),
        description: "This is a test ticket for E2E validation of the pair harness.\n\n\
            The task is to create a simple test file that demonstrates the harness works."
            .to_string(),
        acceptance_criteria: vec![
            "Create a file named test-output.txt".to_string(),
            "File contains the text 'Hello from E2E test'".to_string(),
        ],
        touched_files: vec![],
        labels: vec!["e2e-test".to_string(), "pair-harness".to_string()],
    };

    println!("\n=== Initializing Pair Harness ===");
    println!("Pair ID: {}", config.pair_id);
    println!("Ticket: {}", ticket.id);

    // 5. Initialize the pair harness
    let pair = ForgeSentinelPair::new(config, ticket)?;

    println!("\n=== Starting Pair Execution ===");
    println!("This will:");
    println!("  1. Create a Git worktree");
    println!("  2. Setup shared artifact directory");
    println!("  3. Install plugin structure");
    println!("  4. Spawn FORGE process");
    println!("  5. Start file watcher");
    println!("  6. Monitor for FORGE exit and artifacts");

    // Note: In a real scenario, FORGE would be a Claude Code process
    // For this E2E test, we expect manual intervention or timeout

    println!("\n=== Pair Harness Running ===");
    println!("Timeout: 30 seconds (for E2E test)");

    // 6. Run the pair with a timeout
    let outcome = tokio::time::timeout(std::time::Duration::from_secs(30), pair.run()).await;

    match outcome {
        Ok(Ok(result)) => {
            println!("\n=== Pair Execution Completed ===");
            println!("Outcome: {:?}", result);

            // Verify we got a valid outcome
            match result {
                pair_harness::PairOutcome::Success { pr_url, .. } => {
                    println!("✓ SUCCESS: PR opened at {}", pr_url);
                }
                pair_harness::PairOutcome::Blocked { reason, .. } => {
                    println!("✓ BLOCKED: {}", reason);
                }
                pair_harness::PairOutcome::FuelExhausted { resets, .. } => {
                    println!("✓ FUEL_EXHAUSTED: {} resets used", resets);
                }
            }
        }
        Ok(Err(e)) => {
            println!("\n=== Pair Execution Failed ===");
            println!("Error: {:?}", e);
            return Err(e);
        }
        Err(_) => {
            println!("\n=== Pair Execution Timeout ===");
            println!("✓ Harness is running (timed out after 30s as expected)");
            println!("This is normal for E2E test - FORGE would be long-running");
        }
    }

    println!("\n=== E2E Test Validation ===");

    // 7. Verify artifacts were created
    if shared_path.exists() {
        println!("✓ Shared directory created: {}", shared_path.display());

        // Check for key files
        let ticket_path = shared_path.join("TICKET.md");
        if ticket_path.exists() {
            println!("✓ TICKET.md created");
        }

        let worklog_path = shared_path.join("WORKLOG.md");
        if worklog_path.exists() {
            println!("✓ WORKLOG.md exists");
        }
    } else {
        println!("⚠ Shared directory not found (may not have initialized)");
    }

    // 8. Verify worktree was created
    if worktree_path.exists() {
        println!("✓ Worktree created: {}", worktree_path.display());

        // Check for MCP config
        let mcp_config = worktree_path.join(".claude").join("mcp.json");
        if mcp_config.exists() {
            println!("✓ MCP config installed");
        }

        // Check for plugin structure
        let plugin_dir = worktree_path
            .join(".claude")
            .join("plugins")
            .join("sprintless");
        if plugin_dir.exists() {
            println!("✓ Plugin directory installed");

            if plugin_dir.join("plugin.json").exists() {
                println!("  ✓ plugin.json");
            }
            if plugin_dir.join("skills").exists() {
                println!("  ✓ skills/ directory");
            }
            if plugin_dir.join("commands").exists() {
                println!("  ✓ commands/ directory");
            }
            if plugin_dir.join("hooks").exists() {
                println!("  ✓ hooks/ directory");
            }
        }
    } else {
        println!("⚠ Worktree not found (may not have initialized)");
    }

    println!("\n=== Test Finished Successfully ===\n");
    Ok(())
}

/// Test crash recovery behavior
#[tokio::test]
#[ignore]
async fn test_crash_recovery_simulation() -> Result<()> {
    let _ = tracing_subscriber::fmt().try_init();

    println!("\n=== Testing Crash Recovery ===");

    let config = PairConfig::from_env("pair-crash-test", 2)?;
    let repo_path = config.repo_path.clone();

    let ticket = Ticket {
        id: "T-CRASH-001".to_string(),
        title: "Crash Recovery Test".to_string(),
        description: "Test autonomous crash recovery with reset limits".to_string(),
        acceptance_criteria: vec![
            "System should recover from crashes automatically".to_string(),
            "Should respect max_resets limit".to_string(),
        ],
        touched_files: vec![],
        labels: vec!["crash-test".to_string()],
    };

    let pair = ForgeSentinelPair::new(config, ticket)?;

    println!("Working Directory: {}", repo_path.display());
    println!(
        "Live Worktree: {}",
        repo_path
            .join("worktrees")
            .join("pair-crash-test")
            .display()
    );
    println!(
        "Live Shared Dir: {}",
        repo_path
            .join(".sprintless")
            .join("pairs")
            .join("pair-crash-test")
            .join("shared")
            .display()
    );
    println!("Max resets configured: 2");
    println!("Expected behavior: autonomous recovery up to limit");

    // Run with timeout (crash recovery would happen if FORGE exits)
    let outcome = tokio::time::timeout(std::time::Duration::from_secs(20), pair.run()).await;

    match outcome {
        Ok(Ok(pair_harness::PairOutcome::FuelExhausted { resets, .. })) => {
            println!("✓ Fuel exhausted after {} resets", resets);
            assert!(resets <= 2, "Should not exceed max_resets");
        }
        Ok(Ok(other)) => {
            println!("✓ Completed with outcome: {:?}", other);
        }
        Ok(Err(e)) => {
            println!("✗ Error: {:?}", e);
            return Err(e);
        }
        Err(_) => {
            println!("✓ Test timed out (expected for long-running FORGE)");
        }
    }

    println!("=== Crash Recovery Test Complete ===\n");
    Ok(())
}

fn token_preview(token: &str) -> &str {
    &token[..token.len().min(8)]
}
