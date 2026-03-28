# /plan command

Generates a structured PLAN.md for the current ticket.

## When to use
After reading TICKET.md and TASK.md, before writing any code.

## Steps this command performs
1. Read TICKET.md acceptance criteria
2. Read TASK.md constitution references
3. Search codebase for relevant existing code (search_codebase tool)
4. Draft PLAN.md with full segment breakdown
5. Write PLAN.md to shared/ via write_to_shared tool
6. Notify: "Plan written. Waiting for SENTINEL review."

## Output
Creates shared/PLAN.md
