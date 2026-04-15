---
name: document-pr
description: Update documentation for a merged PR
---

# /document-pr Command

Updates documentation for a merged PR.

## Steps

1. **Get PR Details**
   Fetch the PR description and changed files.

2. **Analyze Changes**
   Determine what documentation needs updating:
   - README.md for user-facing changes
   - docs/ for architectural changes
   - API docs for interface changes

3. **Update Documentation**
   Write or update relevant documentation files.

4. **Commit Changes**
   Commit with message:
   ```
   docs: update documentation for {feature}
   ```

## Output

Documentation updated and committed.
