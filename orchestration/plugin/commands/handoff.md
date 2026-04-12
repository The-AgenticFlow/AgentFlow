# /handoff Command

Create a handoff document for context reset. This enables a fresh FORGE session to continue seamlessly.

## Usage

```
/handoff
```

## When to Use

- Context window is approaching limit (PreCompact hook triggers)
- You've completed multiple segments and need a clean slate
- Complex work that would benefit from fresh perspective

## What it does

1. Reviews WORKLOG.md for completed segments
2. Reviews current segment state
3. Synthesizes HANDOFF.md with all essential context
4. Updates WORKLOG.md with handoff marker
5. Exits cleanly (the harness will spawn a fresh FORGE)

## HANDOFF.md Structure

```markdown
# Handoff: T-{id}

## Context
- Pair: pair-{N}
- Ticket: T-{id}
- Branch: forge-{N}/T-{id}
- Segments completed: {N}
- Current segment: {M}

## Completed Segments

### Segment 1: {title}
- Files changed: [list]
- Key decisions: [list]
- Tests: [status]

### Segment 2: {title}
...

## Current Segment State

### What was being worked on
{description}

### Files modified so far
- {file}: {what was done}
- {file}: {what was done}

### Partial changes
{any incomplete work - be specific about what's done and what's not}

## Decisions Made
1. {decision} - Reason: {why}
2. {decision} - Reason: {why}

## Files Changed (All Segments)
- src/auth.rs: Added OAuth2 support
- tests/auth_test.rs: Added OAuth2 tests
- ...

## Exact Next Step

The next step is:
{specific, actionable instruction}

Start by:
1. {first action}
2. {second action}

Then continue with segment {M} until `/segment-done`.

## Blockers / Dependencies
{any blockers or dependencies the next session should know}

## References
- PLAN.md: {relevant sections}
- Ticket: {key requirements}
```

## After Handoff

The harness will:
1. Detect HANDOFF.md was written
2. Spawn a fresh FORGE session
3. The new FORGE reads HANDOFF.md and continues

## Important

- Be specific about the exact next step - the new session has no other context
- List ALL files changed across all segments
- Include any partial work that needs completion
- The handoff is the ONLY link between sessions - make it complete