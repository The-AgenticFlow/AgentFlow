---
name: update-changelog
description: Add entry to CHANGELOG.md
---

# /update-changelog Command

Adds an entry to CHANGELOG.md for a merged PR.

## Steps

1. **Read PR Description**
   Get the PR description from the merge.

2. **Categorize Change**
   Determine category:
   - Added: New features
   - Changed: Modified behavior
   - Fixed: Bug fixes
   - Security: Security fixes

3. **Write Entry**
   Add to CHANGELOG.md under [Unreleased]:
   ```markdown
   ### Added
   - New feature X (#42)
   ```

4. **Commit**
   Commit with message:
   ```
   docs: update CHANGELOG for T-{id}
   ```

## Output

CHANGELOG.md updated with new entry.
