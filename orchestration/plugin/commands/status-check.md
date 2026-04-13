---
name: status-check
description: Check the current system status
---

# /status-check Command

Returns the current system status including workers, tickets, and PRs.

## Output

```json
{
  "workers": {
    "idle": 1,
    "assigned": 2,
    "working": 0,
    "suspended": 0,
    "done": 0
  },
  "tickets": {
    "pending": 5,
    "in_progress": 2
  },
  "open_prs": 1,
  "pending_approvals": 0
}
```
