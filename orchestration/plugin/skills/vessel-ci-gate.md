---
name: ci-gate
description: Skill for checking CI status and validating merge readiness
---

# CI Gate Skill

## CI Status Check

Use `check_ci_status` MCP tool to verify:
- All required checks passed
- No failing checks
- No pending checks (unless configured to allow)

## Required Checks

Typical required checks:
- Build: Compiles successfully
- Tests: All tests pass
- Lint: No lint errors
- Security: No vulnerabilities detected

## CI Status Values

| Status | Action |
|--------|--------|
| `success` | All checks passed |
| `failure` | One or more checks failed |
| `pending` | Checks still running |
| `unknown` | No CI configured |

## Merge Readiness

PR is ready to merge when:
1. CI status is `success`
2. SENTINEL approval exists (final-review.md)
3. No merge conflicts
4. Branch is up to date with main

## Handling Failures

If CI fails:
1. Do NOT merge
2. Report failure to NEXUS
3. Return `deploy_failed` action with details

## Handling Pending

If CI is pending:
1. Wait and poll
2. Timeout after configured duration
3. Report timeout if exceeded
