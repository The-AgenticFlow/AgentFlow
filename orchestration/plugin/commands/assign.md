---
name: assign
description: Assign a ticket to an available worker
---

# /assign Command

Assigns a ticket to an available FORGE worker.

## When to Use

When you have identified:
- An idle worker slot
- A ticket to assign

## Steps

1. **Verify Worker is Idle**
   Check `KEY_WORKER_SLOTS` for workers with status `Idle`.

2. **Get Ticket Details**
   - ticket_id: The ticket identifier (e.g., T-42)
   - issue_url: GitHub issue URL (optional)

3. **Update Worker Status**
   Set worker status to `Assigned`:
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

4. **Emit Event**
   Log the assignment for observability.

## Output

Worker slot updated with assignment.
Action: `work_assigned`
