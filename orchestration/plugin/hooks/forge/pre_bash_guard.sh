#!/bin/bash
# Runs before every Bash tool call
# Blocks dangerous commands and access to other pairs' worktrees
#
# Environment:
#   CLAUDE_TOOL_INPUT_COMMAND - the command being executed
#   SPRINTLESS_PAIR_ID - the current pair ID

CMD="${CLAUDE_TOOL_INPUT_COMMAND}"

# Block direct git push to non-forge branches - must use MCP tools for PR creation
if echo "$CMD" | grep -qE '^git push|^git push '; then
  # Allow pushing to the pair's own branch
  BRANCH_PATTERN="forge-${SPRINTLESS_PAIR_ID}"
  if echo "$CMD" | grep -q "$BRANCH_PATTERN"; then
    # Allow pushing own branch
    exit 0
  fi
  echo "BLOCKED: Cannot push to branches other than your own."
  echo ""
  echo "Your branch: forge-${SPRINTLESS_PAIR_ID}/${SPRINTLESS_TICKET_ID}"
  echo ""
  echo "After pushing, create a PR using GitHub MCP tools:"
  echo "  1. Use create_pull_request from github MCP server"
  echo "  2. Write STATUS.json with PR_OPENED status and PR URL"
  exit 2
fi

# Block writes to other pairs' worktrees
if echo "$CMD" | grep -qE 'worktrees/pair-[0-9]+/' ; then
  REFERENCED=$(echo "$CMD" | grep -oE 'pair-[0-9]+' | head -1)
  if [ "$REFERENCED" != "${SPRINTLESS_PAIR_ID}" ]; then
    echo "BLOCKED: Cannot access ${REFERENCED}'s worktree."
    echo "You are ${SPRINTLESS_PAIR_ID}."
    echo ""
    echo "Each pair works in isolation. You cannot read or write"
    echo "to another pair's worktree."
    exit 2
  fi
fi

# Block writes to main branch
if echo "$CMD" | grep -qE 'checkout main|checkout origin/main'; then
  echo "BLOCKED: Cannot checkout main. Work on your branch only."
  echo ""
  echo "Your branch: forge-${SPRINTLESS_PAIR_ID}/${SPRINTLESS_TICKET_ID}"
  exit 2
fi

# Block dangerous commands
DANGEROUS_PATTERNS="rm -rf /|sudo rm|:(){ :|:& };:|mkfs|dd if="
if echo "$CMD" | grep -qE "$DANGEROUS_PATTERNS"; then
  echo "BLOCKED: Dangerous command detected."
  echo "This command is not allowed for safety reasons."
  exit 2
fi

# Block network operations (MCP tools should be used instead)
NETWORK_PATTERNS="^curl |^wget |^nc |^ncat |^telnet |^ssh "
if echo "$CMD" | grep -qE "$NETWORK_PATTERNS"; then
  echo "BLOCKED: Network commands are not allowed."
  echo ""
  echo "Use MCP tools instead:"
  echo "  - For GitHub API: use github MCP server"
  echo "  - For HTTP requests: use appropriate MCP tool"
  exit 2
fi

# Block package installation (could introduce unreviewed dependencies)
INSTALL_PATTERNS="npm install |yarn add |pip install |cargo install |go get "
if echo "$CMD" | grep -qE "$INSTALL_PATTERNS"; then
  echo "BLOCKED: Package installation is not allowed."
  echo ""
  echo "If a new dependency is needed:"
  echo "  1. Document it in PLAN.md"
  echo "  2. Get SENTINEL approval"
  echo "  3. Have a human add it to package.json/Cargo.toml"
  exit 2
fi

exit 0