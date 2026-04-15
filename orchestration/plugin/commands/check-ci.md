---
name: check-ci
description: Check CI status for a PR
---

# /check-ci Command

Checks CI status for a pull request.

## Steps

1. **Get PR Number**
   Identify the PR to check.

2. **Query CI Status**
   Use `check_ci_status` MCP tool with:
   - pr_number: The PR number

3. **Report Status**
   - success: All checks passed
   - failure: One or more checks failed
   - pending: Checks still running

## Output

Returns CI status for the PR.
