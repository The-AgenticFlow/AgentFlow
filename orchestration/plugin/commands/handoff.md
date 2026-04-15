---
name: handoff
description: Write a complete handoff for context reset
---

# /handoff Command

Writes a complete handoff and exits for context reset.

## When to Use

When the PreCompact hook fires, or when you choose to reset
context at a natural segment boundary.

## Steps

1. **Read Worklog**
   Read `WORKLOG.md` to summarize completed segments.

2. **Scan In-Progress**
   Check worktree for any in-progress files.

3. **Collect Decisions**
   Gather all decisions made (from WORKLOG.md).

4. **Write Handoff**
   Use `write_to_shared` MCP tool with:
   - artifact_type: `HANDOFF`
   - content: Must include:
     - Completed segments summary
     - In-progress work
     - Decisions made
     - **Exact next step** (required)
     - Files modified
     - Context needed for continuation

5. **Emit Event**
   Use `emit_event` MCP tool:
   - event_type: "context_reset"
   - message: "Context reset - handoff written"

6. **Exit**
   Exit cleanly. Harness will spawn fresh session.

## Handoff Template

```markdown
# Handoff for T-{id}

## Completed Segments
- Segment 1: {summary}
- Segment 2: {summary}

## In-Progress
- {What was being worked on}

## Decisions Made
- {Decision 1}: {rationale}
- {Decision 2}: {rationale}

## Exact Next Step
{Specific, actionable next step}

## Files Modified
- src/file1.rs
- src/file2.rs

## Context Needed
- {Any context the next session needs}
```

## Output

Creates `shared/HANDOFF.md`
Agent exits - harness spawns fresh session.
