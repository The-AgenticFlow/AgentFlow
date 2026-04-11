# /segment-done Command

Signal that the current segment is complete and ready for review.

## Usage

```
/segment-done [notes]
```

## What it does

1. Updates WORKLOG.md with segment completion entry
2. Commits all changes with segment marker
3. Writes to shared/ that segment is done (triggers SENTINEL spawn)
4. Waits for SENTINEL evaluation

## WORKLOG Entry

```markdown
## Segment {N} Complete - {timestamp}

### Changes
- {file1}: {description}
- {file2}: {description}

### Tests Added
- {test1}: {purpose}
- {test2}: {purpose}

### Notes
{any notes passed to command}

### Status
WAITING_REVIEW
```

## After SENTINEL Review

Check the eval file for verdict:

- **APPROVED**: Proceed to next segment or `/status` if done
- **NEEDS_WORK**: Fix issues listed, then `/segment-done` again

## Example

```
/segment-done Fixed edge case in auth flow
```

## Important

- All tests must pass before calling `/segment-done`
- Run linters and fix all warnings
- The segment is not truly "done" until SENTINEL approves
- If SENTINEL returns NEEDS_WORK, address all issues before re-calling