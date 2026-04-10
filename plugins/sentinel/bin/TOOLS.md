# SENTINEL MCP Tools

## Tool Definitions

### `run_tests`

Runs the full test suite.

**Parameters:** None

**Returns:**
```json
{
  "passed": 42,
  "failed": 0,
  "skipped": 2,
  "failures": [],
  "duration_ms": 12345
}
```

---

### `run_linter`

Runs the project linter.

**Parameters:**
- `files`: array[string] (optional) - Files to lint

**Returns:**
```json
{
  "violations": [],
  "clean": true
}
```

---

### `read_from_shared`

Reads an artifact from the shared directory.

**Parameters:**
- `artifact_type`: "TICKET" | "TASK" | "CONTRACT" | "PLAN" | "WORKLOG"

**Returns:**
```json
{
  "content": "Full artifact content..."
}
```

---

### `write_eval`

Writes an evaluation artifact.

**Parameters:**
- `segment_number`: number - Which segment is being evaluated
- `verdict`: "APPROVED" | "CHANGES_REQUESTED"
- `summary`: string
- `feedback`: array[object] (optional) - Required if CHANGES_REQUESTED

**Returns:** Success status

---

### `write_final_review`

Writes the final review after all segments approved.

**Parameters:**
- `verdict`: "APPROVED"
- `summary`: string
- `pr_description`: string - Will be used as PR body

**Returns:** Success status

---

### `get_changed_files`

Returns list of files changed in the current segment.

**Parameters:** None

**Returns:**
```json
{
  "files": [
    {
      "path": "src/auth.rs",
      "status": "modified",
      "additions": 25,
      "deletions": 5
    }
  ]
}
```
