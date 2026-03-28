# /segment-done command

Submits the current segment to SENTINEL for evaluation.

## When to use
When you believe a segment is complete and ready for review.

## Steps this command performs
1. Run full test suite (run_tests tool) — must pass before continuing
2. Run linter on changed files (run_linter tool) — must be clean
3. Commit the segment (commit_segment tool)
4. Append to WORKLOG.md (write_to_shared tool)
5. Emit event: "Segment N submitted for evaluation" (emit_event tool)
6. Wait state: "Submitted. Waiting for SENTINEL segment-N-eval.md"

## Blocked if
- Any tests are failing
- Any lint warnings exist
- No files have been changed since last commit

## Output
Updates WORKLOG.md, commits segment, notifies SENTINEL
