# FORGE-SENTINEL Pair Harness Integration

## Overview

The `ForgePairNode` integrates the full event-driven FORGE-SENTINEL lifecycle into the PocketFlow workflow engine.

**Note:** This implementation uses **filesystem-based state** by default, eliminating the need for Redis. State is stored in the shared directory at `orchestration/pairs/{pair-id}/shared/state.json`.

## Architecture

### Component Relationship

```
PocketFlow (Workflow Engine)
    ↓
ForgePairNode (BatchNode)
    ↓
ForgeSentinelPair (Event-Driven Harness)
    ↓
├── FORGE (Long-Running Process)
├── SENTINEL (Ephemeral Evaluator)
└── SharedDirWatcher (inotify/FSEvents)
```

### Key Components

| Component | Location | Purpose |
|-----------|----------|---------|
| `ForgePairNode` | [`crates/agent-forge/src/lib.rs`](../crates/agent-forge/src/lib.rs) | PocketFlow integration |
| `ForgeSentinelPair` | [`crates/pair-harness/src/pair.rs`](../crates/pair-harness/src/pair.rs) | Lifecycle orchestrator |
| `SharedDirWatcher` | [`crates/pair-harness/src/watcher.rs`](../crates/pair-harness/src/watcher.rs) | Event detection |
| `ProcessManager` | [`crates/pair-harness/src/process.rs`](../crates/pair-harness/src/process.rs) | Process spawning |
| `SentinelNode` | [`crates/agent-sentinel/src/lib.rs`](../crates/agent-sentinel/src/lib.rs) | Code review evaluator |

## Comparison: ForgeNode vs ForgePairNode

### ForgeNode (Old - Simplified)

```rust
pub struct ForgeNode {
    pub workspace_root: PathBuf,
    pub persona_path: PathBuf,
}
```

**Flow:**
1. Create worktree
2. Spawn Claude Code (FORGE) as one-shot process
3. Wait for process exit
4. Read STATUS.json
5. Return outcome

**Limitations:**
- ❌ No SENTINEL review
- ❌ No event-driven architecture
- ❌ No context reset support
- ❌ Polls for STATUS.json
- ❌ One-shot execution model

### ForgePairNode (New - Event-Driven)

```rust
pub struct ForgePairNode {
    pub workspace_root: PathBuf,
    pub github_token: String,
}
```

**Note:** No Redis required - uses filesystem-based state stored in the shared directory.

**Flow:**
1. Create `PairConfig` and `Ticket`
2. Instantiate `ForgeSentinelPair`
3. Run event-driven lifecycle:
   - FORGE writes PLAN.md → SENTINEL reviews → CONTRACT.md
   - FORGE implements segment → writes WORKLOG.md
   - SENTINEL evaluates → writes segment-N-eval.md
   - Repeat for all segments
   - SENTINEL final review → final-review.md
   - FORGE opens PR → writes STATUS.json
4. Handle context resets via HANDOFF.md
5. Return `PairOutcome`

**Advantages:**
- ✅ Full SENTINEL review lifecycle
- ✅ Event-driven (inotify/FSEvents)
- ✅ Automatic context resets
- ✅ Long-running FORGE process
- ✅ Ephemeral SENTINEL spawns

## Usage in PocketFlow

### Basic Integration

```rust
use agent_forge::ForgePairNode;
use pocketflow_core::{Flow, BatchNode};

// Create the node (no Redis required)
let forge_pair = ForgePairNode::new(
    "/path/to/workspace",
    "ghp_your_github_token",
);

// Add to flow
let mut flow = Flow::new("my_flow");
flow.add_node(Box::new(forge_pair));
```

### Outcome Handling

The node returns outcomes mapped to worker status:

| `PairOutcome` | Worker Status | Description |
|---------------|---------------|-------------|
| `PrOpened` | `Done` | PR successfully opened |
| `Blocked` | `Suspended` | Needs human intervention |
| `FuelExhausted` | `Idle` | Max resets exceeded |

### Configuration

The `PairConfig` is automatically created with:

```rust
PairConfig {
    pair_id: worker_id,          // e.g., "forge-1"
    worktree: workspace_root/worktrees/forge-1/,
    shared: workspace_root/orchestration/pairs/forge-1/shared/,
    redis_url: "redis://localhost:6379",
    github_token: "ghp_...",
    max_resets: 10,              // Default
    watchdog_timeout_secs: 1200, // 20 minutes
}
```

## Event-Driven Lifecycle

### Filesystem Events

The `SharedDirWatcher` monitors the shared directory for:

| File | Event | Action |
|------|-------|--------|
| `PLAN.md` | Written | Spawn SENTINEL for plan review |
| `CONTRACT.md` | Written | Resume FORGE (if AGREED) |
| `WORKLOG.md` | Modified | Spawn SENTINEL for segment eval |
| `segment-N-eval.md` | Written | Resume FORGE |
| `final-review.md` | Written | Resume FORGE (if APPROVED) |
| `STATUS.json` | Written | Terminal state - return outcome |
| `HANDOFF.md` | Written | Kill FORGE, respawn fresh |

### Review Cycle

```mermaid
graph TD
    A[FORGE: Write PLAN.md] --> B[Event: PlanWritten]
    B --> C[Spawn SENTINEL PlanReview]
    C --> D[SENTINEL: Write CONTRACT.md]
    D --> E{Status?}
    E -->|AGREED| F[FORGE: Implement Segment 1]
    E -->|ISSUES| A
    F --> G[FORGE: Write WORKLOG.md]
    G --> H[Event: WorklogUpdated]
    H --> I[Spawn SENTINEL SegmentEval]
    I --> J[SENTINEL: Write segment-1-eval.md]
    J --> K{Verdict?}
    K -->|APPROVED| L[Continue to next segment]
    K -->|CHANGES_REQUESTED| F
    L --> M[All segments done?]
    M -->|Yes| N[Spawn SENTINEL FinalReview]
    M -->|No| F
    N --> O[SENTINEL: Write final-review.md]
    O --> P{Verdict?}
    P -->|APPROVED| Q[FORGE: Open PR]
    P -->|REJECTED| R[FORGE: Fix issues]
    Q --> S[FORGE: Write STATUS.json]
    S --> T[Event: StatusJsonWritten]
    T --> U[Return PairOutcome::PrOpened]
```

## Context Resets

When FORGE exhausts its context window:

1. FORGE writes `HANDOFF.md` with current state
2. Event: `HandoffWritten`
3. Harness kills FORGE process
4. Harness spawns fresh FORGE with HANDOFF.md context
5. FORGE resumes from checkpoint

**Reset Limit:** Configurable via `PairConfig.max_resets` (default: 10)

## Thread Safety

The `ForgePairNode` spawns the pair lifecycle in a `spawn_blocking` task because:

- `SharedDirWatcher` uses `std::sync::mpsc::Receiver` (not `Send + Sync`)
- The event loop is synchronous with timeout-based polling
- The pair lifecycle needs a dedicated runtime

```rust
let outcome = tokio::task::spawn_blocking(move || {
    let rt = tokio::runtime::Runtime::new()?;
    rt.block_on(async move {
        let mut pair = ForgeSentinelPair::new(config);
        pair.run(&ticket).await
    })
})
.await??;
```

## Testing

### Unit Test

```rust
#[tokio::test]
async fn test_forge_pair_node() {
    let workspace = tempdir().unwrap();
    let node = ForgePairNode::new(
        workspace.path(),
        "ghp_test_token",
    );
    
    let slot = WorkerSlot {
        id: "forge-1".to_string(),
        status: WorkerStatus::Assigned {
            ticket_id: "T-123".to_string(),
            issue_url: Some("https://github.com/owner/repo/issues/123".to_string()),
        },
    };
    
    let result = node.exec_one(json!(slot)).await.unwrap();
    assert_eq!(result["outcome"], "pr_opened");
}
```

### E2E Test

See [`crates/pair-harness/tests/full_e2e.rs`](../crates/pair-harness/tests/full_e2e.rs) for full lifecycle testing.

## Migration Guide

### From ForgeNode to ForgePairNode

1. **Update imports:**
   ```rust
   // Old
   use agent_forge::ForgeNode;
   
   // New
   use agent_forge::ForgePairNode;
   ```

2. **Update construction:**
   ```rust
   // Old
   let forge = ForgeNode::new(workspace_root, persona_path);
   
   // New (no Redis required)
   let forge = ForgePairNode::new(
       workspace_root,
       github_token,
   );
   ```

3. **Update flow:**
   - No changes needed - same `BatchNode` interface
   - Returns same action types: `pr_opened`, `suspended`, `failed`

4. **Update configuration:**
   - Add GitHub token to environment
   - Ensure `orchestration/pairs/` directory exists
   - No Redis setup required

### Benefits of Migration

| Aspect | Improvement |
|--------|-------------|
| Code Quality | SENTINEL reviews every segment |
| Reliability | Automatic retries on CHANGES_REQUESTED |
| Efficiency | Zero-polling event detection |
| Scalability | Long-running process model |
| Observability | Detailed segment evaluations |

## Future Enhancements

- [ ] **Parallel Segment Evaluation**: Evaluate multiple segments concurrently
- [ ] **Smart Checkpointing**: More granular HANDOFF.md state
- [ ] **Dynamic Watchdog**: Adjust timeout based on segment complexity
- [ ] **Review Metrics**: Track SENTINEL approval rates
- [ ] **Adaptive Resets**: Learn optimal reset thresholds per project

## References

- [FORGE-SENTINEL Architecture](./forge-sentinel-arch.md)
- [Pair Harness Crate](../crates/pair-harness/)
- [PocketFlow Core](../crates/pocketflow-core/)
