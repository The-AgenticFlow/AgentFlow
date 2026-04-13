# FORGE MCP Tools

## Tool Definitions

### `create_pr`

Opens a pull request on GitHub after the done contract is fulfilled.

**Parameters:**
- `title`: string - PR title
- `body`: string - PR description
- `head_branch`: string - Source branch (e.g., "forge-1/T-42")
- `base_branch`: string - Target branch (usually "main")

**Returns:**
```json
{
  "pr_url": "https://github.com/org/repo/pull/47",
  "pr_number": 47
}
```

---

### `get_issue`

Fetches a GitHub issue by number for context.

**Parameters:**
- `issue_number`: number

**Returns:**
```json
{
  "title": "Implement feature X",
  "body": "Full issue description...",
  "labels": ["enhancement"],
  "assignees": []
}
```

---

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
- `files`: array[string] (optional) - Files to lint. Empty = all changed files.

**Returns:**
```json
{
  "violations": [
    {
      "file": "src/auth.rs",
      "line": 42,
      "rule": "unused_variable",
      "message": "unused variable: `x`"
    }
  ],
  "clean": false
}
```

---

### `search_codebase`

Semantic search across the codebase.

**Parameters:**
- `query`: string - Search query
- `limit`: number (optional) - Max results (default: 10)

**Returns:**
```json
{
  "results": [
    {
      "file": "src/auth/session.rs",
      "line_start": 10,
      "line_end": 25,
      "snippet": "fn validate_token(...) {...}",
      "relevance": 0.95
    }
  ]
}
```

---

### `commit_segment`

Commits all changes for the current segment.

**Parameters:**
- `segment_name`: string - e.g., "segment-1"
- `description`: string - Brief description

**Returns:**
```json
{
  "commit_sha": "abc123...",
  "files_changed": 3
}
```

---

### `write_to_shared`

Writes an artifact to the shared directory.

**Parameters:**
- `artifact_type`: "PLAN" | "WORKLOG_ENTRY" | "HANDOFF" | "STATUS"
- `content`: string - The content to write

**Returns:** Success status

---

### `read_from_shared`

Reads an artifact from the shared directory.

**Parameters:**
- `artifact_type`: "TICKET" | "TASK" | "CONTRACT" | "segment_eval" | "final_review"

**Returns:**
```json
{
  "content": "Full artifact content..."
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
