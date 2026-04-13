---
name: status
description: Write terminal STATUS.json after done contract fulfilled
---

# /status Command

Writes the terminal STATUS.json after the done contract is fulfilled.

## When to Use

ONLY after:
- `final-review.md` exists with APPROVED verdict
- All segments are complete and approved

## Steps

1. **Verify Final Review**
   Check `final-review.md` exists and verdict is `APPROVED`.

2. **Run Tests**
   Use `run_tests` MCP tool.
   All tests must pass.

3. **Push Branch**
   ```bash
   git push origin forge-{worker_id}/{ticket_id}
   ```

4. **Open PR**
   Use `create_pr` MCP tool with:
   - title: From ticket title
   - body: From SENTINEL's PR description in final-review.md
   - head_branch: forge-{worker_id}/{ticket_id}
   - base_branch: main

5. **Write STATUS.json**
   Use `write_to_shared` MCP tool with:
   - artifact_type: `STATUS`
   - content:
     ```json
     {
       "status": "PR_OPENED",
       "pair": "forge-1",
       "ticket_id": "T-42",
       "pr_url": "https://github.com/org/repo/pull/47",
       "pr_number": 47,
       "files_changed": ["src/file1.rs", "src/file2.rs"]
     }
     ```

6. **Emit Event**
   Use `emit_event` MCP tool:
   - event_type: "pr_opened"
   - message: "PR opened - T-{id} complete"

7. **Exit**
   Exit cleanly. NEXUS will read STATUS.json.

## Blocked If

- `final-review.md` does not exist
- `final-review.md` verdict is not APPROVED
- Tests are failing

## Output

PR opened on GitHub.
Creates `shared/STATUS.json` with status: PR_OPENED
Agent exits - NEXUS reads STATUS.json
