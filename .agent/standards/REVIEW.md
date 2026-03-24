# Review Standards (REVIEW.md)

## 1. PR Gatekeeping
- **No tests = No merge**: Any PR missing tests for changed logic must be blocked.
- **No inline comments = Incomplete review**: SENTINEL must post comments on exact line numbers.
- **Architecture check**: PRs should not violate established patterns in `.agent/arch/`.

## 2. Feedback Tone
- Feedback must be **actionable and specific**. Avoid vague summaries like "code looks okay but needs improvement".
- Provide code snippets or exact suggestions for fixes.

## 3. Security Review
- Check for hardcoded secrets.
- Check for insecure file permissions.
- Validate that dependencies added are approved or follow system patterns.
