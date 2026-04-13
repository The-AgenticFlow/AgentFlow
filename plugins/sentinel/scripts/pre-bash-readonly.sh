#!/bin/bash
# SENTINEL Pre-Bash Readonly Guard
# Ensures SENTINEL cannot modify source files

CMD="${CLAUDE_TOOL_INPUT_COMMAND}"
WORKTREE="${SPRINTLESS_WORKTREE:-.}"
SHARED="${SPRINTLESS_SHARED:-.sprintless/shared}"

# Patterns that indicate file modification or code execution
# Covers redirection, common tools with write flags, and script interpreters
WRITE_PATTERNS="(>|>>)|(sed -i|awk.*>|tee |truncate|dd.*of=)|(python[23]?|perl|ruby|php|node|bash|sh|zsh) (-c|-e|--)"

if echo "$CMD" | grep -qE "$WRITE_PATTERNS"; then
  echo ""
  echo "=== BLOCKED: Potential Write/Execute Operation ==="
  echo "SENTINEL cannot modify source files or run arbitrary scripts."
  echo "Write your evaluation to ${SHARED}/segment-N-eval.md"
  exit 2
fi

# Allow reading and running tests
exit 0
