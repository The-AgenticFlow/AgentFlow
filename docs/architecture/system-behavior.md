# AgentFlow System Architecture

## Overview

AgentFlow is an autonomous AI development team that implements, validates, and merges code changes without human intervention. The system is a cyclic flow of three agents -- NEXUS (orchestrator), FORGE-SENTINEL (implementation), and VESSEL (merge gatekeeper) -- connected through a shared state store.

The key architectural principle is that **NEXUS is the orchestrator of the entire pipeline**, not just a ticket assigner. It can detect broken states at any point in the flow and resume the pipeline at the correct phase.

---

## Agent Roles

| Agent | Role | Implementation | Capabilities |
|---|---|---|---|
| **NEXUS** | Orchestrator | LLM-driven (AgentRunner) | Ticket assignment, flow recovery, command gating, pipeline routing |
| **FORGE-SENTINEL** | Implementation pair | Event-driven Claude Code processes | Code implementation, plan review, segment evaluation, PR creation |
| **VESSEL** | Merge gatekeeper | Deterministic Rust code | CI polling, squash merge, ticket closure, event emission |

---

## Flow Graph

```
                    +----------------------------------------------+
                    |                                              |
                    v                                              |
+----------+  work_assigned  +--------------+  pr_opened  +---------+
|          | --------------> |              | -----------> |         |
|  NEXUS   |                 | FORGE-SENTINEL|             | VESSEL  |
|          | <-------------- |              | <---------- |         |
+----------+  failed/        +--------------+  deployed/  +---------+
    |         suspended                           deploy_failed
    |                                              merge_blocked
    |         merge_prs --------------------------------+
    |                                                 |
    |         no_work                                 |
    +--------> (loop)                                 |
              NEXUS <---------------------------------+
```

### Routing Table

| Source | Action | Target | Meaning |
|---|---|---|---|
| NEXUS | `work_assigned` | FORGE-SENTINEL | Assign ticket to worker for implementation |
| NEXUS | `merge_prs` | VESSEL | Resume merge pipeline for pending PRs |
| NEXUS | `no_work` | NEXUS | No actionable items, loop back |
| NEXUS | `approve_command` | FORGE-SENTINEL | Approve worker's suspended command |
| NEXUS | `reject_command` | NEXUS | Reject worker's command, recycle worker |
| FORGE-SENTINEL | `pr_opened` | VESSEL | Implementation complete, PR created |
| FORGE-SENTINEL | `failed` | NEXUS | Implementation failed, retry possible |
| FORGE-SENTINEL | `suspended` | NEXUS | Worker needs command approval |
| VESSEL | `deployed` | NEXUS | PR merged successfully |
| VESSEL | `deploy_failed` | NEXUS | CI failed or merge blocked |
| VESSEL | `merge_blocked` | NEXUS | Merge conflict or other blockage |
| VESSEL | `no_work` | NEXUS | No pending PRs to process |

---

## Pipeline Phases

A ticket moves through three phases. NEXUS is responsible for ensuring every ticket completes the entire pipeline.

### Phase 1: Implementation (NEXUS -> FORGE-SENTINEL)

**Trigger:** `work_assigned` action from NEXUS
**Agent:** FORGE-SENTINEL pair (Claude Code FORGE + Claude Code SENTINEL)
**Completion signal:** PR opened on GitHub, `pr_opened` action returned

The FORGE-SENTINEL pair runs an event-driven lifecycle:
1. FORGE writes PLAN.md -> SENTINEL reviews -> CONTRACT.md agreed
2. FORGE implements segments -> SENTINEL evaluates -> segment-N-eval.md
3. SENTINEL final review -> final-review.md
4. FORGE opens PR -> STATUS.json written

**STATUS.json recognized values:**

| STATUS | PairOutcome | Flow Path |
|---|---|---|
| `PR_OPENED`, `COMPLETE`, `complete`, `completed`, `COMPLETED`, `SEGMENTS_COMPLETE`, `SEGMENT_COMPLETE_AWAITING_REVIEW` | PrOpened (if pr_url present) or Blocked | -> VESSEL or -> NEXUS |
| `IMPLEMENTATION_COMPLETE` | Blocked ("needs push/PR creation") | -> NEXUS (forge attempts auto-push) |
| `BLOCKED` | Blocked | -> NEXUS (suspended) |
| `FUEL_EXHAUSTED` | FuelExhausted | -> NEXUS (failed, possibly retryable) |
| Any other string | FuelExhausted ("Unknown status: X") | -> NEXUS (failed) |

### Phase 2: Merge (VESSEL)

**Trigger:** `pr_opened` action from FORGE-SENTINEL, or `merge_prs` action from NEXUS
**Agent:** VESSEL (deterministic Rust code, no LLM)
**Completion signal:** `deployed` action (merged) or `deploy_failed` action (failed)

VESSEL processes each entry in `pending_prs` from the shared store:
1. Fetch PR details from GitHub API
2. If CI workflows exist: poll CI status until terminal (success/failure/timeout, 10s interval, 10min timeout)
3. If CI green (or no CI): squash merge with ticket reference
4. Update ticket status to Merged, remove from pending_prs, recycle worker to Idle

**Input (from shared store):**
- `pending_prs`: Array of `{number, ticket_id, branch, worker_id}`
- `repository`: "owner/repo" string
- `ci_readiness`: Ready / Missing / SetupInProgress

### Phase 3: Done (back to NEXUS)

VESSEL returns `deployed` or `deploy_failed` -> NEXUS re-evaluates state for next action.

---

## Shared State Store

All agents communicate through a SharedStore (in-memory or Redis-backed).

### Store Keys

| Key | Type | Written By | Read By |
|---|---|---|---|
| `tickets` | Vec of Ticket | NEXUS, FORGE, VESSEL | NEXUS, FORGE, VESSEL |
| `worker_slots` | HashMap of WorkerSlot | NEXUS, FORGE, VESSEL | NEXUS, FORGE |
| `pending_prs` | Vec of Value | FORGE | VESSEL, NEXUS |
| `command_gate` | HashMap of Value | FORGE | NEXUS |
| `ci_readiness` | CiReadiness enum | NEXUS | NEXUS, VESSEL |
| `repository` | String | Init code | NEXUS, VESSEL |
| `_no_work_count` | u32 | NEXUS | NEXUS |
| `_forge_batch_workers` | Vec of String | FORGE prep | FORGE post |

### Ticket Lifecycle

```
                    +--------------------------------------+
                    |                                      |
                    v                                      |
  Open --> Assigned --> InProgress --> Completed --> Merged
    |          |                         (pr_opened)
    |          |                              |
    |          v                              |
    |        Failed <-------------------------+
    |       (attempts < 3)
    |          |
    |          v
    |        Exhausted
    |       (attempts >= 3)
    |
    +--------> (re-assignable)
```

| Status | Meaning | Assignable? |
|---|---|---|
| Open | Unassigned, ready for work | Yes |
| Assigned { worker_id } | Assigned to a worker | No |
| InProgress { worker_id } | Actively being worked on | No |
| Completed { worker_id, outcome } | Implementation done (outcome e.g. "pr_opened") | No |
| Merged { worker_id, pr_number } | PR merged, fully complete | No |
| Failed { worker_id, reason, attempts } | Failed, retryable if attempts < 3 | Yes (if attempts < 3) |
| Exhausted { worker_id, attempts } | Max retries reached, terminal | No |

### Worker Lifecycle

```
  Idle --> Assigned --> Working --> Done --> (recycled to Idle)
              |                         |
              v                         v
          Suspended                 (recycled to Idle
          (command gate)             when assignable tickets exist)
```

| Status | Meaning |
|---|---|
| Idle | Available for assignment |
| Assigned { ticket_id, issue_url } | Assigned but not started |
| Working { ticket_id, issue_url } | Actively working |
| Done { ticket_id, outcome } | Completed work (auto-recycled to Idle when assignable tickets exist) |
| Suspended { ticket_id, reason, issue_url } | Waiting for command approval |

---

## Flow Recovery (NEXUS as Orchestrator)

The critical design feature is that **NEXUS can detect and resume the flow at any point**. This handles:

- Network failures between agents
- Agent crashes (FORGE process dies, VESSEL timeout)
- Unrecognized STATUS.json values that cause fuel exhaustion
- Process restarts (tickets/workers left in intermediate states)

### Reconciliation (NexusNode::reconcile())

On every NEXUS cycle, prep() runs reconcile() which scans the shared store for inconsistencies:

| Detection | Condition | Root Cause |
|---|---|---|
| **Unmerged PRs** | pending_prs has entries but VESSEL never ran | FORGE crashed after creating PR, network failure prevented vessel routing, STATUS.json unrecognized |
| **Orphaned tickets** | Ticket in Assigned/InProgress but worker is Idle or missing | FORGE crashed before updating ticket status, process restart lost worker state |
| **Stale workers** | Worker in Assigned/Working but ticket is Open (was reset by recovery) | Cross-state inconsistency after partial recovery |
| **Stale suspended workers** | Worker in Suspended but ticket is already Completed/Merged | Command gate was never cleared after ticket completed |
| **Completed without PR** | Ticket Completed{outcome:"pr_opened"} but no matching entry in pending_prs | PR data was lost from the store (rare) |

### FlowRecovery Data Structure

```rust
pub struct FlowRecovery {
    pub unmerged_prs: Vec<UnmergedPr>,          // PRs in pending_prs awaiting merge
    pub orphaned_tickets: Vec<OrphanedTicket>,  // Tickets assigned to idle/missing workers
    pub stale_workers: Vec<StaleWorker>,         // Workers stuck on non-existent/completed tickets
    pub completed_without_pr: Vec<String>,       // Completed tickets missing from pending_prs
    pub has_unmerged_prs: bool,
    pub has_orphaned_tickets: bool,
    pub has_stale_workers: bool,
    pub has_completed_without_pr: bool,
    pub needs_recovery: bool,
}
```

This structure is computed in prep() and passed to the NEXUS LLM as `flow_recovery` context. The persona uses it to prioritize recovery over new work assignment.

### Automatic Recovery (NexusNode::recover_orphans())

When NEXUS returns `work_assigned`, the post() method automatically runs recover_orphans() which:

1. **Resets orphaned tickets**: Tickets in Assigned/InProgress whose worker is Idle or missing are reset to Open (re-assignable)
2. **Recycles stale suspended workers**: Workers in Suspended whose ticket is already Completed/Merged are recycled to Idle
3. **Recycles stale assigned workers**: Workers in Assigned/Working whose ticket was reset to Open by recovery are recycled to Idle

### NEXUS Decision Priority

The NEXUS persona enforces this strict priority order:

1. **Unmerged PR recovery** -> `merge_prs` action -> routes to VESSEL
2. **Command gate** -> `approve_command` / `reject_command`
3. **CI-first rule** -> assign CI setup ticket if ci_readiness is Missing
4. **New work** -> `work_assigned` action -> routes to FORGE-SENTINEL
5. **No work** -> `no_work` action -> loops back to NEXUS

This ensures that completed work (PRs awaiting merge) is always processed before new work is started.

---

## NEXUS Cycle (Detailed)

Each NEXUS cycle follows the prep -> exec -> post pattern:

### prep() -- Gather Context

1. **sync_registry** -- Load registry.json, add new WorkerSlots as Idle
2. **Parse repository** -- Split store["repository"] into owner/repo_name
3. **sync_issues** -- Call GitHub API, filter out PRs, create Ticket objects
4. **check_ci_readiness** -- Call GitHub API for workflow files
5. **ensure_ci_setup_ticket** -- Inject synthetic CI ticket if needed
6. **prioritize_ci_first** -- Sort tickets so CI tickets come first
7. **Recycle Done workers** -- Done -> Idle when assignable tickets exist
8. **reconcile** -- Detect flow inconsistencies (unmerged PRs, orphaned tickets, stale workers)
9. **Build context** -- All state + recovery data passed to LLM

### exec() -- LLM Decision

The AgentRunner invokes the LLM with the nexus persona and context. The LLM returns an AgentDecision with action, notes, and optional assign_to/ticket_id/issue_url fields.

### post() -- Apply Decision

| Action | Store Effects | Flow Route |
|---|---|---|
| `merge_prs` | Resets no_work counter | -> VESSEL (via ACTION_MERGE_PRS route) |
| `work_assigned` | Resets no_work counter; runs recover_orphans(); sets ticket to Assigned; sets worker to Assigned; injects CI ticket if needed | -> FORGE-SENTINEL |
| `no_work` | Increments no_work counter; returns STOP_SIGNAL after 3 consecutive | -> NEXUS (loop) |
| `approve_command` | Removes from command_gate; transitions worker Suspended -> Assigned | -> FORGE-SENTINEL |
| `reject_command` | Removes from command_gate; transitions worker to Idle | -> NEXUS (loop) |

---

## Failure Scenarios and Recovery

### Scenario 1: FORGE crashes after PR creation (the original bug)

**Before the fix:** FORGE writes STATUS.json with "COMPLETED" (uppercase) -> pair harness doesn't recognize it -> FuelExhausted -> forge returns "failed" -> routes to NEXUS -> NEXUS sees no assignable tickets and unmerged PRs but has no `merge_prs` action -> loops on `no_work` until stop.

**After the fix:**
1. Pair harness recognizes "COMPLETED" as a completion status -> PrOpened -> forge returns "pr_opened" -> routes to VESSEL (happy path)
2. Even if pair harness still fails (unknown status), FORGE's post_batch() checks GitHub for an existing PR -> if found, adds to pending_prs -> returns "pr_opened" -> routes to VESSEL
3. If both mechanisms fail and it still routes as "failed" back to NEXUS, reconcile() detects pending_prs has entries -> flow_recovery.has_unmerged_prs = true -> NEXUS returns `merge_prs` -> routes to VESSEL

### Scenario 2: Network failure between FORGE and VESSEL

FORGE successfully creates PR and returns "pr_opened", but the flow crashes before VESSEL runs.

**Recovery:** On restart, reconcile() finds entries in pending_prs -> NEXUS returns merge_prs -> VESSEL processes them.

### Scenario 3: VESSEL times out on CI polling

VESSEL returns "deploy_failed" -> routes to NEXUS. The PR stays in pending_prs.

**Recovery:** On the next NEXUS cycle, reconcile() detects the PR is still in pending_prs -> NEXUS returns merge_prs -> VESSEL retries the merge.

### Scenario 4: FORGE process killed mid-implementation

Ticket stays in Assigned/InProgress, worker stays in Working.

**Recovery:** On the next NEXUS cycle, reconcile() detects orphaned ticket (worker might be Idle after crash) -> recover_orphans() resets ticket to Open -> NEXUS can re-assign it.

### Scenario 5: Process restart with stale state

Tickets in Assigned, workers in Working, pending_prs with unmerged PRs.

**Recovery:** reconcile() detects all inconsistencies at once. NEXUS prioritizes merge_prs first (unmerged PRs), then recover_orphans() resets orphaned tickets so they can be re-assigned.

---

## Key Source Files

| Component | File |
|---|---|
| NEXUS node | crates/agent-nexus/src/lib.rs |
| NEXUS persona | orchestration/agent/agents/nexus.agent.md |
| FORGE-SENTINEL pair node | crates/agent-forge/src/lib.rs |
| Pair harness (STATUS.json) | crates/pair-harness/src/pair.rs |
| VESSEL node | crates/agent-vessel/src/lib.rs |
| State types and constants | crates/config/src/state.rs |
| Flow definition (production) | binary/src/bin/real_test.rs |
| Flow definition (dev/dry-run) | binary/src/main.rs |
| Agent registry | orchestration/agent/registry.json |
