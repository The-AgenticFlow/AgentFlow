---
name: gate-approve
description: Approve or reject a pending dangerous command
---

# /gate-approve Command

Reviews and approves/rejects a pending dangerous command from a worker.

## When to Use

When `KEY_COMMAND_GATE` contains pending approvals.

## Steps

1. **Read Pending Command**
   Check `KEY_COMMAND_GATE` for the command details:
   - worker_id: Which worker requested
   - command: The command they want to run
   - reason: Why they need it

2. **Evaluate Risk vs Necessity**
   - Is this command required for the ticket?
   - What could go wrong?
   - Can we undo the effects?
   - Is there a safer alternative?

3. **Make Decision**
   - Approve: Worker can proceed
   - Reject: Worker must find alternative

4. **Update State**
   - Remove from command gate
   - Update worker status accordingly

## Output

- Approved: Worker status set back to `Assigned`
- Rejected: Worker status set to `Idle` with reason
