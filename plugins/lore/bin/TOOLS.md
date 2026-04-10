# LORE MCP Tools

## Tool Definitions

### `get_merged_prs`

Returns recently merged PRs that need documentation.

**Parameters:**
- `since`: string (optional) - ISO date string, default: last 24 hours

**Returns:**
```json
{
  "prs": [
    {
      "number": 47,
      "title": "[T-42] Implement feature X",
      "merged_at": "2024-01-15T12:00:00Z",
      "files_changed": ["src/auth.rs", "src/session.rs"]
    }
  ]
}
```

---

### `get_pr_details`

Gets full details of a merged PR.

**Parameters:**
- `pr_number`: number

**Returns:**
```json
{
  "number": 47,
  "title": "[T-42] Implement feature X",
  "body": "PR description...",
  "files": [
    {
      "path": "src/auth.rs",
      "additions": 25,
      "deletions": 5
    }
  ],
  "labels": ["enhancement"]
}
```

---

### `update_file`

Updates a documentation file.

**Parameters:**
- `path`: string - File path
- `content`: string - New content
- `message`: string - Commit message

**Returns:**
```json
{
  "success": true,
  "commit_sha": "abc123..."
}
```

---

### `read_changelog`

Reads the current CHANGELOG.md.

**Parameters:** None

**Returns:**
```json
{
  "content": "Full changelog content..."
}
```

---

### `update_changelog`

Updates CHANGELOG.md with new entry.

**Parameters:**
- `category`: "added" | "changed" | "fixed" | "security" | "deprecated" | "removed"
- `entry`: string - The changelog entry
- `pr_number`: number - For reference link

**Returns:**
```json
{
  "success": true,
  "commit_sha": "abc123..."
}
```
