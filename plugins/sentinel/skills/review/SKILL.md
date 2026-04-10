---
name: review
description: Code review skill with specific evaluation criteria
---

# SENTINEL Review Skill

## Review Protocol

### Step 1: Run Tests
Use `run_tests` MCP tool. All tests must pass.

### Step 2: Run Linter
Use `run_linter` MCP tool. Zero warnings allowed.

### Step 3: Read Changed Files
Check every changed file against CONTRACT criteria.

### Step 4: Check Error Handling
Verify every error path is covered:
- Network failures
- Invalid inputs
- Edge cases
- Boundary conditions

### Step 5: Check Test Coverage
- Is every new function tested?
- Are both happy path and error paths covered?
- Are edge cases tested?

### Step 6: Check Standards Compliance
- CODING.md respected?
- patterns.md followed?
- api-contracts.md adhered to?

## Writing Evaluations

### Evaluation File Structure
```markdown
# Segment N Evaluation

## Verdict
APPROVED | CHANGES_REQUESTED

## Summary
{Brief summary of what was reviewed}

## Specific Feedback (if CHANGES_REQUESTED)

### Issue 1
- File: src/auth/session.ts
- Line: 47
- Problem: Throws raw Error
- Required Fix: throw new AppError('SESSION_EXPIRED', 401) per CODING.md rule 3

### Issue 2
...
```

## Feedback Guidelines

### DO
- Be specific: file, line, problem, required fix
- Be constructive: explain why and how to fix
- Reference standards: cite specific rules

### DON'T
- Vague feedback: "improve error handling"
- Personal criticism: "you did this wrong"
- Skip details: "fix the tests"

## Final Review

When all segments approved, write `final-review.md`:

```markdown
# Final Review for T-{id}

## Verdict
APPROVED

## Summary
{Overall assessment of the implementation}

## PR Description
{Detailed PR description that will be used for the actual PR}

## Test Results
- All tests passing
- Coverage: X%

## Standards Compliance
- CODING.md: Compliant
- patterns.md: Compliant
```
