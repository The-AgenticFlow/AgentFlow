#!/bin/bash
# SENTINEL Pre-Bash Guard Hook
# Enforces read-only mode - SENTINEL cannot modify source files
#
# Environment:
#   SPRINTLESS_WORKTREE - the worktree path (source code)
#   SPRINTLESS_SHARED - the shared directory (allowed writes)
#   CLAUDE_BASH_COMMAND - the command about to run (injected by Claude Code)

WORKTREE="${SPRINTLESS_WORKTREE:-}"
SHARED="${SPRINTLESS_SHARED:-}"
CMD="${CLAUDE_BASH_COMMAND:-}"

# If no command, allow
if [ -z "$CMD" ]; then
  exit 0
fi

# Blocked commands - SENTINEL cannot use these at all
BLOCKED_PATTERNS=(
  "git "
  "git"
  "rm "
  "rm"
  "sudo "
  "npm install"
  "pip install"
  "cargo install"
  "mv "
  "cp "
  "chmod"
  "chown"
)

for pattern in "${BLOCKED_PATTERNS[@]}"; do
  if [[ "$CMD" == *"$pattern"* ]]; then
    echo "=============================================="
    echo "  BLOCKED: Command not allowed for SENTINEL"
    echo "=============================================="
    echo ""
    echo "Command: ${CMD}"
    echo ""
    echo "SENTINEL is read-only. You cannot:"
    echo "  - Use git commands"
    echo "  - Delete files (rm)"
    echo "  - Move or copy files"
    echo "  - Install packages"
    echo "  - Modify permissions"
    echo ""
    echo "You CAN:"
    echo "  - Read files (cat, head, tail, less)"
    echo "  - Run tests and linters"
    echo "  - Search code (grep, find)"
    echo "  - Write to shared/ directory"
    echo ""
    exit 2
  fi
done

# Check if command writes to source tree
if [ -n "$WORKTREE" ]; then
  # Patterns that write files
  WRITE_PATTERNS=(
    " > "
    " >> "
    " 2> "
    " | tee "
    "sed -i"
    "awk -i"
    "truncate"
    "dd if="
  )
  
  for pattern in "${WRITE_PATTERNS[@]}"; do
    if [[ "$CMD" == *"$pattern"* ]]; then
      # Check if the redirect target is in worktree
      # Extract the file path after the redirect
      REDIRECT_FILE=$(echo "$CMD" | grep -oP '(?<=[<>])\s*\S+' | head -1 | tr -d ' ')
      
      if [ -n "$REDIRECT_FILE" ]; then
        # Get absolute path
        if [[ "$REDIRECT_FILE" != /* ]]; then
          REDIRECT_FILE="${WORKTREE}/${REDIRECT_FILE}"
        fi
        
        # Check if it's inside worktree but NOT in shared
        if [[ "$REDIRECT_FILE" == "$WORKTREE"* ]] && [[ "$REDIRECT_FILE" != *"$SHARED"* ]]; then
          echo "=============================================="
          echo "  BLOCKED: Cannot write to source tree"
          echo "=============================================="
          echo ""
          echo "Command: ${CMD}"
          echo ""
          echo "SENTINEL can only write to: ${SHARED}"
          echo "Attempted write to: ${REDIRECT_FILE}"
          echo ""
          echo "All your outputs (eval files, reviews) go in shared/."
          echo ""
          exit 2
        fi
      fi
    fi
  done
fi

# Allow the command
exit 0