#!/bin/bash
# FORGE Pre-Bash Guard Hook
# Blocks dangerous commands and access to other workers' directories

CMD="${CLAUDE_TOOL_INPUT_COMMAND}"
PAIR_ID="${SPRINTLESS_PAIR_ID:-forge-1}"

# Block direct git push (must use /status command)
if echo "$CMD" | grep -qE "^git push|git push "; then
  echo ""
  echo "=== BLOCKED: Direct git push ==="
  echo "Direct git push is not allowed."
  echo "When ready to push, use the /status command which will"
  echo "verify the done contract and push via MCP tool."
  exit 2
fi

# Block writes to other pairs' worktrees
if echo "$CMD" | grep -qE "worktrees/pair-[0-9]+/|forge-[0-9]+/"; then
  REFERENCED=$(echo "$CMD" | grep -oE "pair-[0-9]+|forge-[0-9]+" | head -1)
  if [ -n "$REFERENCED" ] && [ "$REFERENCED" != "${PAIR_ID}" ] && [ "$REFERENCED" != "forge-${PAIR_ID#forge-}" ]; then
    echo ""
    echo "=== BLOCKED: Cross-pair access ==="
    echo "Cannot access ${REFERENCED}'s worktree."
    echo "You are ${PAIR_ID}."
    exit 2
  fi
fi

# Block checkout to main
if echo "$CMD" | grep -qE "checkout main|checkout origin/main|switch main"; then
  echo ""
  echo "=== BLOCKED: Main branch checkout ==="
  echo "Cannot checkout main. Work on your branch only."
  exit 2
fi

# Block force push
if echo "$CMD" | grep -qE "push.*--force|push.*-f"; then
  echo ""
  echo "=== BLOCKED: Force push ==="
  echo "Force push is not allowed."
  echo "If you need this, request approval via the command gate."
  exit 2
fi

# Block dangerous file operations
if echo "$CMD" | grep -qE "rm -rf /|rm -rf ~|rm -rf \*"; then
  echo ""
  echo "=== BLOCKED: Dangerous file operation ==="
  echo "This command could cause irreversible damage."
  exit 2
fi

exit 0
