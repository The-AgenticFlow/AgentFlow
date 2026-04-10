# NEXUS MCP Tools

## Tool Definitions

### `get_worker_slots`

Returns the current status of all worker slots.

**Parameters:** None

**Returns:**
```json
{
  "slots": [
    {
      "id": "forge-1",
      "status": "Idle"
    },
    {
      "id": "forge-2",
      "status": {
        "Assigned": {
          "ticket_id": "T-42",
          "issue_url": "https://github.com/org/repo/issues/42"
        }
      }
    }
  ]
}
```

---

### `assign_worker`

Assigns a ticket to a worker.

**Parameters:**
- `worker_id`: string - The worker to assign (e.g., "forge-1")
- `ticket_id`: string - The ticket ID (e.g., "T-42")
- `issue_url`: string (optional) - GitHub issue URL

**Returns:** Success/failure status

---

### `get_command_gate`

Returns pending dangerous command approvals.

**Parameters:** None

**Returns:**
```json
{
  "pending": [
    {
      "worker_id": "forge-1",
      "command": "npm publish",
      "reason": "Need to publish new version"
    }
  ]
}
```

---

### `approve_command`

Approves a pending command.

**Parameters:**
- `worker_id`: string - The worker whose command is approved

**Returns:** Success status

---

### `reject_command`

Rejects a pending command.

**Parameters:**
- `worker_id`: string - The worker whose command is rejected
- `reason`: string - Why the command was rejected

**Returns:** Success status

---

### `emit_event`

Emits an event to the event stream.

**Parameters:**
- `event_type`: string
- `message`: string
- `data`: object (optional)

**Returns:** Success status
