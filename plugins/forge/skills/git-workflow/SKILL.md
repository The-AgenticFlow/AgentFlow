---
name: git-workflow
description: Git workflow skill for branch management and commits
---

# Git Workflow Skill

## Branch Naming

Your branch follows the pattern: `forge-{worker_id}/{ticket_id}`

Example: `forge-1/T-42`

## Commit Protocol

### Segment Commits
After each segment, commit with:
```bash
git add -A
git commit -m "[T-{id}] segment-N: {description}"
```

### Commit Message Format
```
[T-42] segment-1: Add authentication module

- Implement JWT validation
- Add refresh token logic
- Write unit tests
```

## Push Restrictions

- NEVER push directly to main
- NEVER force push
- ALWAYS push to your feature branch only

Use the `/status` command when ready to push and open PR.

## Before Push

1. Ensure all tests pass
2. Ensure linter is clean
3. Ensure SENTINEL approved (final-review.md exists)
4. Check for divergence from main:
   ```bash
   git fetch origin main
   BEHIND=$(git rev-list --count HEAD..origin/main)
   if [ "$BEHIND" -gt 10 ]; then
     git rebase origin/main
     # Re-run tests after rebase
   fi
   ```

## Handling Conflicts

If rebase produces conflicts:
1. Do NOT attempt to resolve automatically
2. Set STATUS.json to `BLOCKED` with reason `REBASE_CONFLICT`
3. Exit and let NEXUS handle escalation
