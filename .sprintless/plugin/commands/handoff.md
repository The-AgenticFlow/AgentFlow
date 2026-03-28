# /handoff command

Writes a complete handoff and exits for context reset.

## When to use
When the PreCompact hook fires, or when you choose to reset
context at a natural segment boundary.

## Steps this command performs
1. Read WORKLOG.md to summarise completed segments
2. Scan worktree for in-progress files
3. Collect all decisions made (from WORKLOG.md decisions sections)
4. Write complete HANDOFF.md to shared/ (write_to_shared tool)
5. Emit event: "Context reset — handoff written" (emit_event tool)
6. Exit cleanly

## Output
Creates shared/HANDOFF.md
Agent exits — harness spawns fresh session
