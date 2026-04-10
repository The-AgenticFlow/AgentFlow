# VESSEL MCP Tools

## Tool Definitions

### `get_open_prs`

Returns open PRs from forge branches.

**Parameters:** None

**Returns:**
```json
{
  "prs": [
    {
      "number": 47,
      "title": "[T-42] Implement feature X",
      "head_branch": "forge-1/T-42",
      "author": "forge-1",
      "created_at": "2024-01-15T10:00:00Z"
    }
  ]
}
```

---

### `check_ci_status`

Checks CI status for a PR.

**Parameters:**
- `pr_number`: number

**Returns:**
```json
{
  "status": "success" | "failure" | "pending" | "unknown",
  "checks": [
    {
      "name": "build",
      "status": "success",
      "conclusion": "success"
    },
    {
      "name": "test",
      "status": "completed",
      "conclusion": "success"
    }
  ]
}
```

---

### `merge_pr`

Merges an approved PR.

**Parameters:**
- `pr_number`: number
- `merge_method`: "merge" | "squash" | "rebase" (default: "squash")
- `commit_message`: string (optional) - For squash merge

**Returns:**
```json
{
  "merged": true,
  "merge_sha": "abc123...",
  "message": "PR #47 merged successfully"
}
```

---

### `check_final_review`

Checks if SENTINEL has approved the PR.

**Parameters:**
- `pr_number`: number

**Returns:**
```json
{
  "approved": true,
  "pr_description": "From SENTINEL's final review..."
}
```

---

### `emit_event`

Emits an event to the event stream.

**Parameters:**
- `event_type`: string
- `message`: string
- `data`: object (optional)

**Returns:** Success status
