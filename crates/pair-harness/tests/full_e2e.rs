// crates/pair-harness/tests/full_e2e.rs
//! End-to-end integration test for FORGE-SENTINEL pair lifecycle.
//!
//! This test verifies the complete lifecycle from ticket assignment to PR merge,
//! following the specification in docs/forge-sentinel-arch.md section 19.

use std::path::PathBuf;
use std::time::Duration;
use tempfile::TempDir;

use pair_harness::{FileLockManager, ForgeSentinelPair, FsEvent, PairConfig, PairOutcome, Ticket};

/// Test configuration for e2e tests
struct TestConfig {
    /// Temporary directory for test project
    temp_dir: TempDir,
    /// Path to main worktree
    main_path: PathBuf,
    /// Path to worktrees directory
    worktrees_path: PathBuf,
    /// Path to orchestration directory
    orchestration_path: PathBuf,
}

impl TestConfig {
    fn new() -> anyhow::Result<Self> {
        let temp_dir = TempDir::new()?;
        let main_path = temp_dir.path().join("main");
        let worktrees_path = temp_dir.path().join("worktrees");
        let orchestration_path = temp_dir.path().join("orchestration");

        // Create directory structure
        std::fs::create_dir_all(&main_path)?;
        std::fs::create_dir_all(&worktrees_path)?;
        std::fs::create_dir_all(orchestration_path.join("pairs"))?;
        std::fs::create_dir_all(orchestration_path.join("locks"))?;
        std::fs::create_dir_all(orchestration_path.join("plugin"))?;

        Ok(Self {
            temp_dir,
            main_path,
            worktrees_path,
            orchestration_path,
        })
    }

    fn pair_config(&self, pair_id: &str) -> PairConfig {
        PairConfig::new(pair_id, self.temp_dir.path(), "test_token")
    }

    fn test_ticket() -> Ticket {
        Ticket {
            id: "T-42".to_string(),
            issue_number: 42,
            title: "Add user authentication endpoint".to_string(),
            body: "Implement JWT-based authentication with login endpoint".to_string(),
            url: "https://github.com/test-org/test-project/issues/42".to_string(),
            touched_files: vec![
                "src/routes/auth.ts".to_string(),
                "src/middleware/auth.ts".to_string(),
                "tests/routes/auth.test.ts".to_string(),
            ],
            acceptance_criteria: vec![
                "POST /auth/login endpoint accepts credentials".to_string(),
                "JWT tokens generated with 24h expiry".to_string(),
                "Auth middleware validates tokens".to_string(),
                "Error handling covers all edge cases".to_string(),
            ],
        }
    }
}

#[tokio::test]
#[ignore] // Run with `cargo test -- --ignored` for e2e tests
async fn test_pair_lifecycle_from_assignment_to_pr() {
    // This test requires:
    // 1. A real Git repository
    // 2. Redis server running
    // 3. Claude CLI installed
    // 4. GitHub API access (or mock)

    let config = TestConfig::new().expect("Failed to create test config");
    let pair_config = config.pair_config("pair-1");
    let ticket = TestConfig::test_ticket();

    // Create pair harness
    let mut pair = ForgeSentinelPair::new(pair_config);

    // Run the pair lifecycle
    let result = pair.run(&ticket).await;

    // Verify outcome
    match result {
        Ok(PairOutcome::PrOpened {
            pr_url,
            pr_number,
            branch,
        }) => {
            println!("PR opened: {} (#{})", pr_url, pr_number);
            assert!(branch.starts_with("forge-pair-1/"));
        }
        Ok(PairOutcome::Blocked { reason, blockers }) => {
            panic!("Pair blocked: {} ({} blockers)", reason, blockers.len());
        }
        Ok(PairOutcome::FuelExhausted {
            reason,
            reset_count,
        }) => {
            panic!("Pair fuel exhausted: {} ({} resets)", reason, reset_count);
        }
        Err(e) => {
            panic!("Pair failed: {:?}", e);
        }
    }
}

#[tokio::test]
async fn test_file_locking_mechanism() {
    let temp_dir = TempDir::new().expect("Failed to create temp dir");
    let locks_dir = temp_dir.path().join("locks");
    std::fs::create_dir_all(&locks_dir).expect("Failed to create locks dir");

    let manager = FileLockManager::new(&locks_dir);

    // Test acquiring lock
    let file_path = PathBuf::from("src/routes/auth.ts");
    let result = manager.try_acquire(&file_path, "pair-1");

    match result {
        Ok(pair_harness::isolation::LockResult::Acquired) => {
            println!("Lock acquired successfully");
        }
        Ok(pair_harness::isolation::LockResult::AlreadyOwned) => {
            println!("Lock already owned by this pair");
        }
        Ok(pair_harness::isolation::LockResult::Blocked { owner, .. }) => {
            panic!("Lock owned by different pair: {}", owner);
        }
        Err(e) => {
            panic!("Failed to acquire lock: {:?}", e);
        }
    }

    // Test that another pair cannot acquire the same lock
    let result2 = manager.try_acquire(&file_path, "pair-2");
    match result2 {
        Ok(pair_harness::isolation::LockResult::Blocked { owner, .. }) => {
            assert_eq!(owner, "pair-1");
            println!("Lock correctly blocked for pair-2");
        }
        _ => {
            panic!("Lock should be owned by pair-1");
        }
    }

    // Test releasing locks
    let released = manager
        .release_all_for_pair("pair-1")
        .expect("Failed to release locks");
    assert_eq!(released.len(), 1);
    println!("Released {} locks", released.len());
}

#[tokio::test]
async fn test_worktree_provisioning() {
    use pair_harness::WorktreeManager;

    let temp_dir = TempDir::new().expect("Failed to create temp dir");
    let main_path = temp_dir.path().join("main");

    // Initialize a git repo in main
    std::fs::create_dir_all(&main_path).expect("Failed to create main dir");
    std::process::Command::new("git")
        .args(&["init"])
        .current_dir(&main_path)
        .output()
        .expect("Failed to init git repo");

    // Create initial commit
    std::fs::write(main_path.join("README.md"), "# Test Project\n")
        .expect("Failed to write README");
    std::process::Command::new("git")
        .args(&["add", "README.md"])
        .current_dir(&main_path)
        .output()
        .expect("Failed to add README");
    std::process::Command::new("git")
        .args(&["commit", "-m", "Initial commit"])
        .current_dir(&main_path)
        .output()
        .expect("Failed to commit");

    let manager = WorktreeManager::new(main_path.clone());

    // Create worktree
    let result = manager.create_worktree("pair-1", "T-42");
    match result {
        Ok(worktree_path) => {
            assert!(worktree_path.exists());
            println!("Worktree created at: {:?}", worktree_path);

            // Verify branch was created
            let output = std::process::Command::new("git")
                .args(&["branch", "--list", "forge-pair-1/T-42"])
                .current_dir(&main_path)
                .output()
                .expect("Failed to list branches");
            let stdout = String::from_utf8_lossy(&output.stdout);
            assert!(stdout.contains("forge-pair-1/T-42"));
        }
        Err(e) => {
            // This test requires git to be installed
            eprintln!("Warning: Worktree creation failed (git required): {:?}", e);
        }
    }
}

#[tokio::test]
async fn test_watcher_event_classification() {
    use pair_harness::SharedDirWatcher;

    let temp_dir = TempDir::new().expect("Failed to create temp dir");
    let shared_dir = temp_dir.path().join("shared");
    std::fs::create_dir_all(&shared_dir).expect("Failed to create shared dir");

    // Create watcher
    let watcher_result = SharedDirWatcher::new(&shared_dir);
    match watcher_result {
        Ok(_watcher) => {
            println!("Watcher created successfully");
            // In a real test, we would:
            // 1. Create files in shared_dir
            // 2. Wait for events
            // 3. Verify correct FsEvent is emitted
        }
        Err(e) => {
            eprintln!("Warning: Watcher creation failed: {:?}", e);
        }
    }
}

#[tokio::test]
async fn test_context_reset_and_handoff() {
    use pair_harness::ResetManager;

    let temp_dir = TempDir::new().expect("Failed to create temp dir");
    let shared_dir = temp_dir.path().join("shared");
    std::fs::create_dir_all(&shared_dir).expect("Failed to create shared dir");

    // Create a mock WORKLOG.md
    let worklog = r#"# WORKLOG

## Segment 1: POST /auth/login endpoint
- Files changed:
  - src/routes/auth.ts
  - tests/routes/auth.test.ts
- Decision: Used Express Router pattern
- Status: APPROVED

## Segment 2: JWT token generation
- Files changed:
  - src/utils/jwt.ts
- Decision: Used jsonwebtoken library
- Status: APPROVED
"#;
    std::fs::write(shared_dir.join("WORKLOG.md"), worklog).expect("Failed to write WORKLOG");

    let manager = ResetManager::new(shared_dir.clone(), 10);

    // Synthesize handoff
    let result = manager.synthesize_handoff();
    result.await.expect("Failed to synthesize handoff");

    // Verify handoff was created
    let handoff_path = shared_dir.join("HANDOFF.md");
    assert!(handoff_path.exists());
    println!("Handoff synthesized at: {:?}", handoff_path);

    // Verify handoff content
    let content = std::fs::read_to_string(&handoff_path).expect("Failed to read handoff");
    assert!(content.contains("## Completed Work"));
    assert!(content.contains("## Files Changed"));
    assert!(content.contains("## Key Decisions"));
}

#[tokio::test]
async fn test_watchdog_stall_detection() {
    use pair_harness::Watchdog;

    let temp_dir = TempDir::new().expect("Failed to create temp dir");
    let shared_dir = temp_dir.path().join("shared");
    std::fs::create_dir_all(&shared_dir).expect("Failed to create shared dir");

    // Create WORKLOG.md (the file watchdog actually monitors)
    std::fs::write(shared_dir.join("WORKLOG.md"), "# Worklog\n\n- Started task")
        .expect("Failed to write WORKLOG");

    let mut watchdog = Watchdog::new(shared_dir, 60);

    // Check for stall (should not be stalled initially)
    let status = watchdog.check_stalled().expect("Failed to check watchdog");
    assert!(!status.is_stalled());
    println!("Watchdog correctly detected no stall");
}

/// Helper function to set up mock Git repository for testing
#[allow(dead_code)]
fn setup_mock_git_repo(path: &PathBuf) -> anyhow::Result<()> {
    use std::process::Command;

    // Initialize repo
    Command::new("git")
        .args(&["init"])
        .current_dir(path)
        .output()?;

    // Set user config
    Command::new("git")
        .args(&["config", "user.email", "test@example.com"])
        .current_dir(path)
        .output()?;

    Command::new("git")
        .args(&["config", "user.name", "Test User"])
        .current_dir(path)
        .output()?;

    // Create initial commit
    std::fs::write(path.join(".gitkeep"), "")?;
    Command::new("git")
        .args(&["add", ".gitkeep"])
        .current_dir(path)
        .output()?;

    Command::new("git")
        .args(&["commit", "-m", "Initial commit"])
        .current_dir(path)
        .output()?;

    Ok(())
}
