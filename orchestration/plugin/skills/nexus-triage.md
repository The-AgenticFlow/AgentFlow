---
name: ticket-triage
description: Skill for analyzing and prioritizing incoming tickets
---

# Ticket Triage Skill

## Analyzing Tickets

### Extract Key Information
From each ticket, identify:
- **Type**: Bug, feature, refactor, documentation
- **Priority**: Critical, high, medium, low
- **Scope**: Files/components affected
- **Dependencies**: Blocked by other tickets?
- **Acceptance Criteria**: Definition of done

### Estimation Heuristics

| Scope | Estimated Segments |
|-------|-------------------|
| Single file, small change | 1-2 |
| Multiple files, moderate | 3-5 |
| Cross-cutting, large | 6-10 |

### Blocking Detection

A ticket is blocked if:
- Depends on unmerged PR
- Requires external API/service
- Needs clarification from stakeholder

## Prioritization Matrix

```
          | High Impact | Low Impact
---------|-------------|------------
Urgent   | Do Now      | Schedule
Not Urgent| Plan       | Backlog
```

## CI-First Override

**CI setup tickets ALWAYS take absolute priority over all other tickets.**

Before applying the normal matrix, check if the repository has CI workflows:
- If `ci_readiness` is `missing`: Only CI setup tickets should be assigned
- CI setup tickets are identified by: ID starting with `T-CI-`, or title containing "CI" + ("setup" or "pipeline" or "workflow")
- Without CI, VESSEL cannot validate PRs and will stall, wasting all worker cycles
- This override applies regardless of issue number, apparent urgency, or any other priority signal

## Assignment Considerations

- Complexity matching worker experience
- File ownership (avoid conflicts)
- Load balancing across available workers
