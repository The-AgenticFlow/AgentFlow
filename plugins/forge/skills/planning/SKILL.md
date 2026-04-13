---
name: planning
description: Skill for creating implementation plans with segment breakdowns
---

# FORGE Planning Skill

## Writing PLAN.md

Use the `/plan` command to structure your plan correctly.

### Plan Structure

```markdown
# Plan for T-{id}: {Title}

## Understanding
{Your understanding of the ticket in your own words}

## Technical Approach
{How you will implement this, following .agent/arch/patterns.md}

## Segment Breakdown

### Segment 1: {Name}
- Files: file1.rs, file2.rs
- Purpose: {Single clear purpose}
- Done when: {Specific, verifiable criteria}

### Segment 2: {Name}
...

## Files to Create/Modify
- src/new_file.rs (create)
- src/existing_file.rs (modify)

## Risk Areas
- {Things you are uncertain about}
- {Potential blockers}

## Questions for SENTINEL
- {Clarifications needed before starting}
```

## Segment Sizing

### Good Segment
- Touches 1-3 files
- Has a single clear purpose
- Can be tested in isolation
- Takes 20-40 minutes to implement

### Too Large (Split It)
- Touches more than 5 files
- Has multiple unrelated concerns
- Cannot be independently verified

## Contract Negotiation

1. SENTINEL reviews your plan
2. If objections raised, read carefully
3. Update PLAN.md addressing each objection
4. Do not argue - accept feedback or ask clarifying question
5. Proceed only when SENTINEL approves
