---
name: final-review
description: Write final review after all segments approved
---

# /final-review Command

Writes the final review after all segments are approved.

## When to Use

After all segments have been evaluated and approved.

## Steps

1. **Verify All Segments Approved**
   Check all `segment-N-eval.md` files have APPROVED verdict.

2. **Run Full Test Suite**
   Use `run_tests` MCP tool. All must pass.

3. **Run Full Linter**
   Use `run_linter` MCP tool. Must be clean.

4. **Verify CONTRACT Criteria**
   Check every criterion in CONTRACT.md is satisfied.

5. **Write Final Review**
   Use `write_to_shared` MCP tool with:
   - artifact_type: `final_review`
   - content:
     ```markdown
     # Final Review for T-{id}

     ## Verdict
     APPROVED

     ## Summary
     {Overall assessment}

     ## PR Description
     {Detailed description for the PR}

     ## Test Results
     - All tests passing
     - Coverage: X%

     ## Standards Compliance
     - CODING.md: Compliant
     - patterns.md: Compliant
     ```

## Output

Creates `shared/final-review.md` with APPROVED verdict.
FORGE can now use `/status` to open PR.
