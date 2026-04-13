---
name: plan
description: Create an implementation plan for the current ticket
---

# /plan Command

Generates a structured PLAN.md for the current ticket.

## When to Use

After reading TICKET.md and TASK.md, before writing any code.

## Steps

1. **Read Context**
   - Read `TICKET.md` for acceptance criteria
   - Read `TASK.md` for constitution references
   - Read `CONTRACT.md` if it exists

2. **Search Codebase**
   Use `search_codebase` MCP tool to find:
   - Existing patterns to follow
   - Similar implementations
   - Utility functions to reuse

3. **Draft Plan**
   Structure your plan with:
   - Understanding of the ticket
   - Technical approach
   - Segment breakdown (1-3 files per segment)
   - Definition of done per segment
   - Files to create/modify
   - Risk areas
   - Questions for SENTINEL

4. **Write PLAN.md**
   Use `write_to_shared` MCP tool with:
   - artifact_type: `PLAN`
   - content: The plan content

5. **Notify**
   "Plan written. Waiting for SENTINEL review."

## Output

Creates `shared/PLAN.md`
