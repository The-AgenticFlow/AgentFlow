---
name: criteria
description: The five evaluation criteria that all segments must pass
---

# SENTINEL Evaluation Criteria

## The Five Criteria

All must pass for any approval.

### 1. Correctness

Does the implementation correctly handle all cases described in CONTRACT.md?

**Check:**
- All acceptance criteria met
- Error paths handled
- Edge cases covered
- Boundary conditions correct

**FAIL:** Any CONTRACT criterion not met, any obvious logic error

---

### 2. Test Coverage

Are all changed files covered by tests?

**Check:**
- Every new function has at least one test
- Happy path tested
- Primary error path tested
- Edge cases tested where relevant

**FAIL:** Any changed file with no tests, any new function with no test

---

### 3. Standards Compliance

Does the implementation follow team standards?

**Check:**
- `.agent/standards/CODING.md` respected
- `.agent/arch/patterns.md` patterns used
- `.agent/arch/api-contracts.md` contracts honored

**FAIL:** Any violation of the team's written standards

---

### 4. Code Quality

Is the code readable and maintainable?

**Check:**
- Names are clear and descriptive
- Complexity is justified
- No unnecessary duplication
- Comments where needed

**NOTE:** This criterion is advisory. It cannot block alone.
It informs feedback but a single quality concern is not a blocker.

---

### 5. No Regressions

Do all existing tests still pass?

**Check:**
- Run full test suite
- No previously passing test now failing
- No existing behavior changed without explicit ticket scope

**FAIL:** Any previously passing test now failing

---

## Scoring

| Criterion | Weight | Can Block |
|-----------|--------|-----------|
| Correctness | Critical | Yes |
| Test Coverage | Critical | Yes |
| Standards Compliance | Critical | Yes |
| Code Quality | Advisory | No |
| No Regressions | Critical | Yes |

All **Critical** criteria must pass for APPROVED verdict.
