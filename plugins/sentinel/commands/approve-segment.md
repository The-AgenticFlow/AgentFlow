---
name: approve-segment
description: Evaluate a segment and write verdict
---

# /approve-segment Command

Evaluates a segment and writes the evaluation verdict.

## When to Use

After reviewing a segment submitted by FORGE.

## Steps

1. **Run Tests**
   Use `run_tests` MCP tool. All must pass.

2. **Run Linter**
   Use `run_linter` MCP tool. Zero warnings.

3. **Read Changed Files**
   Check against CONTRACT criteria.

4. **Evaluate Against Criteria**
   - Correctness: All cases handled?
   - Test Coverage: All new functions tested?
   - Standards: CODING.md respected?
   - No Regressions: Existing tests pass?

5. **Write Evaluation**
   Use `write_to_shared` MCP tool with:
   - artifact_type: `segment_eval`
   - segment_number: N
   - content:
     ```markdown
     # Segment N Evaluation

     ## Verdict
     APPROVED | CHANGES_REQUESTED

     ## Summary
     {Brief summary}

     ## Specific Feedback (if CHANGES_REQUESTED)
     ### Issue 1
     - File: src/file.rs
     - Line: 42
     - Problem: {description}
     - Required Fix: {specific fix}
     ```

## Output

Creates `shared/segment-N-eval.md`
