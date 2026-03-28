# SENTINEL evaluation criteria

## The five criteria — all must pass for any approval

### 1. Correctness
Does the implementation correctly handle all cases described in CONTRACT.md?
Does it handle the error paths, edge cases, and boundary conditions?
FAIL: any CONTRACT criterion not met, any obvious logic error

### 2. Test coverage
Are all changed files covered by tests?
Does every new function have at least one test for the happy path
and one for the primary error path?
FAIL: any changed file with no tests, any new function with no test

### 3. Standards compliance
Does the implementation follow .agent/standards/CODING.md?
Does it use the patterns in .agent/arch/patterns.md?
Does it respect the API contracts in .agent/arch/api-contracts.md?
FAIL: any violation of the team's written standards

### 4. Code quality
Is the code readable? Are names clear? Is complexity justified?
Is there duplication that should be extracted?
NOTE: This criterion is advisory — it cannot block alone.
It informs feedback but a single quality concern is not a blocker.

### 5. No regressions
Do all existing tests still pass?
Has any existing behaviour been changed without explicit ticket scope?
FAIL: any previously passing test now failing
