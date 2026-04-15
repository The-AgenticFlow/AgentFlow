---
name: segment-done
description: Submit the current segment for SENTINEL review
---

# /segment-done Command

Submits the current segment to SENTINEL for evaluation.

## When to Use

When you believe a segment is complete and ready for review.

## Steps

1. **Run Tests**
   Use `run_tests` MCP tool.
   - All tests must pass before continuing
   - If any fail, fix them first

2. **Run Linter**
   Use `run_linter` MCP tool.
   - Must be clean (zero warnings)
   - Fix any issues first

3. **Commit Segment**
   Use `commit_segment` MCP tool with:
   - segment_name: e.g., "segment-1"
   - description: Brief description of changes

4. **Update Worklog**
   Use `write_to_shared` MCP tool with:
   - artifact_type: `WORKLOG_ENTRY`
   - content: What was done, decisions made

5. **Emit Event**
   Use `emit_event` MCP tool:
   - event_type: "segment_submitted"
   - message: "Segment N submitted for evaluation"

## Blocked If

- Any tests are failing
- Any lint warnings exist
- No files have been changed since last commit

## Output

Updates WORKLOG.md, commits segment, notifies SENTINEL.
Wait for `segment-N-eval.md` from SENTINEL.
