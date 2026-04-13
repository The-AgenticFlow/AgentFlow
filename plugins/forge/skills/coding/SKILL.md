---
name: coding
description: Core coding skill with testing and quality standards
---

# FORGE Coding Skill

## Before Writing Code

1. **Read Context Files**
   - `TICKET.md`: What you are building
   - `CONTRACT.md`: Definition of done
   - `PLAN.md`: Your implementation approach

2. **Search Codebase**
   Use `search_codebase` MCP tool to find:
   - Existing patterns to follow
   - Similar implementations
   - Utility functions to reuse

3. **Read Standards**
   - `.agent/standards/CODING.md`: Coding conventions
   - `.agent/arch/patterns.md`: Architecture patterns
   - `.agent/arch/api-contracts.md`: API contracts

## Coding Standards

### Error Handling
- Never throw raw `Error`
- Use `AppError` from `src/errors/`
- Every async function must have explicit error handling
- Network calls must have timeout and retry

### Code Style
- Follow existing patterns in the codebase
- Keep functions focused and small
- Use descriptive names
- Add comments for complex logic

### Security
- Validate all inputs
- Sanitize outputs
- Never hardcode secrets
- Use environment variables for configuration

## Testing Discipline

### Requirements
- Every new function needs a test
- Every changed file needs updated tests
- Test both happy path and error paths

### Before Submitting
1. Run tests: `run_tests` MCP tool
2. All tests must pass
3. No test can be skipped without reason

## Segment Completion Checklist

- [ ] Code follows standards
- [ ] Tests written and passing
- [ ] Linter clean
- [ ] No regressions
- [ ] Documentation updated (if needed)
