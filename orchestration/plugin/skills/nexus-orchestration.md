---
name: orchestration
description: Core orchestration skill for assigning work and managing workers
---

# NEXUS Orchestration Skill

## Worker Assignment Protocol

### Step 1: Check Worker Availability
Read `KEY_WORKER_SLOTS` from shared store. Look for workers with status `Idle`.

### Step 2: Prioritize Tickets
- High priority: Security fixes, blocking issues
- Medium priority: Features, enhancements
- Low priority: Refactors, documentation

### Step 3: Match Worker to Ticket
Consider:
- Worker capacity (some workers may have specializations)
- Ticket complexity vs worker history
- Load balancing across workers

### Step 4: Assign Work
Update worker slot:
```json
{
  "id": "forge-1",
  "status": {
    "Assigned": {
      "ticket_id": "T-42",
      "issue_url": "https://github.com/org/repo/issues/42"
    }
  }
}
```

### Step 5: Emit Event
Log assignment to event stream for observability.

## Command Gate Protocol

### Evaluating Dangerous Commands

Workers may request approval for commands like:
- `rm -rf` (file deletion)
- `git push --force` (force push)
- `npm publish` (package publishing)
- Database migrations

### Decision Framework

1. **Necessity**: Is this command required for the ticket?
2. **Risk**: What could go wrong?
3. **Reversibility**: Can we undo the effects?
4. **Alternatives**: Is there a safer approach?

### Approval Response
Set worker status from `Suspended` back to `Assigned`.

### Rejection Response
Set worker status to `Idle` with rejection reason.
