---
name: merge
description: Merge an approved PR after CI passes
---

# /merge Command

Merges an approved PR after all gates pass.

## When to Use

When:
- CI status is success
- SENTINEL approval exists (final-review.md)
- No merge conflicts

## Steps

1. **Verify CI**
   Use `check_ci_status` MCP tool.
   Must be `success`.

2. **Verify Approval**
   Check `final-review.md` exists with APPROVED verdict.

3. **Check for Conflicts**
   Verify PR has no merge conflicts.

4. **Merge PR**
   Use `merge_pr` MCP tool with:
   - pr_number: The PR number
   - merge_method: "squash" (recommended)
   - commit_message: From SENTINEL's PR description

5. **Report**
   Emit merge event and update NEXUS.

## Blocked If

- CI is failing or pending
- No SENTINEL approval
- Merge conflicts exist

## Output

PR merged into main.
Worker slot set to Idle.
