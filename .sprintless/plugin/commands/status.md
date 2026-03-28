# /status command

Writes the terminal STATUS.json after done contract is fulfilled.

## When to use
ONLY after:
- final-review.md exists with APPROVED verdict
- All segments are complete and approved

## Steps this command performs
1. Verify final-review.md exists and verdict is APPROVED
2. Verify all tests pass (run_tests tool)
3. Push branch to GitHub origin (git push via bash)
4. Open PR using SENTINEL's PR description (create_pr tool)
5. Write STATUS.json to shared/ (write_to_shared tool)
6. Emit event: "PR opened — T-{id} complete" (emit_event tool)
7. Exit cleanly

## Blocked if
- final-review.md does not exist
- final-review.md verdict is not APPROVED
- Tests are failing

## Output
PR opened on GitHub
Creates shared/STATUS.json with status: PR_OPENED
