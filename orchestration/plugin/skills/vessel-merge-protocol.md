---
name: merge-protocol
description: Protocol for safely merging approved PRs
---

# Merge Protocol Skill

## Pre-Merge Checklist

Before merging, verify:
- [ ] CI status: success
- [ ] SENTINEL approval: final-review.md exists with APPROVED
- [ ] No merge conflicts
- [ ] Branch up to date with main (or rebased)

## Merge Process

### Step 1: Fetch Latest
```bash
git fetch origin main
```

### Step 2: Check Divergence
```bash
BEHIND=$(git rev-list --count HEAD..origin/main)
if [ "$BEHIND" -gt 0 ]; then
  # Need rebase or merge
fi
```

### Step 3: Merge
Use `merge_pr` MCP tool with:
- Merge method: `squash` (recommended) or `merge`
- Commit message: From SENTINEL's PR description

### Step 4: Verify
- Confirm merge completed
- Confirm branch deleted (optional)

### Step 5: Report
- Emit merge event
- Update shared store with merge status

## Merge Methods

| Method | Use When |
|--------|----------|
| `squash` | Single logical change (recommended) |
| `merge` | Multiple commits should be preserved |
| `rebase` | Linear history preferred |

## Post-Merge

1. **Cleanup**: Remove worktree
2. **Notify**: Emit event for NEXUS
3. **Update**: Set worker slot to Idle

## Failure Handling

If merge fails:
1. Check for conflicts
2. Report to NEXUS with `deploy_failed`
3. Do NOT attempt to resolve conflicts automatically
