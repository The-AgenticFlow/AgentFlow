# Security Guidelines (SECURITY.md)

## 1. Command Approval Gate
All agents must propose "dangerous" bash commands to NEXUS before execution. Dangerous commands include:
- `rm -rf` or any un-scoped deletion.
- `chmod 777` or any permissive permission change.
- `git push --force`.
- `curl | sh` or any external script execution.
- Writing to files outside the assigned worker slot or the `.agent` directory.

## 2. Secrets Management
- NEVER commit API keys or plain text secrets to the repository.
- Use environment variables (`.env`).
- Agents must NOT read `.env` unless explicitly required by their role (e.g., LiteLLM config).

## 3. Input Sanitization
- Sanitize all user-provided strings before using them in shell commands or file paths.
- Prevent shell injection by using array-based process spawning instead of string interpolation where possible.
