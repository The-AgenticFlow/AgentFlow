#!/bin/bash
# SENTINEL Pre-Bash Readonly Guard
# Ensures SENTINEL cannot modify source files

CMD="${CLAUDE_TOOL_INPUT_COMMAND}"
WORKTREE="${SPRINTLESS_WORKTREE:-.}"
SHARED="${SPRINTLESS_SHARED:-.sprintless/shared}"

# Patterns that indicate file modification
WRITE_PATTERNS="sed -i|awk.*>|tee |> [^/]|echo.*>.*src|echo.*>.*tests|cat.*>|truncate|dd.*of="

if echo "$CMD" | grep -qE "$WRITE_PATTERNS"; then
  echo ""
  echo "=== BLOCKED: Write Operation ==="
  echo "SENTINEL cannot modify source files."
  echo "Write your evaluation to ${SHARED}/segment-N-eval.md"
  exit 2
fi

# Allow reading and running tests
exit 0
