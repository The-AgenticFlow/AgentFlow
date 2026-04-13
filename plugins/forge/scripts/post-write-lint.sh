#!/bin/bash
# FORGE Post-Write Lint Hook
# Runs linter after every Write, Edit, or MultiEdit tool call

FILE="${CLAUDE_TOOL_INPUT_FILE_PATH}"
WORKTREE="${SPRINTLESS_WORKTREE:-.}"

# Only lint source files
case "$FILE" in
  *.rs)
    if command -v cargo &>/dev/null; then
      OUTPUT=$(cd "$WORKTREE" && cargo clippy --quiet 2>&1)
      if [ $? -ne 0 ]; then
        echo ""
        echo "=== CLIPPY FAILED ==="
        echo "$OUTPUT"
        echo ""
        echo "Fix these warnings before continuing."
        exit 2
      fi
    fi
    ;;
  *.ts|*.tsx)
    if command -v npx &>/dev/null; then
      OUTPUT=$(cd "$WORKTREE" && npx eslint "$FILE" 2>&1)
      if [ $? -ne 0 ]; then
        echo ""
        echo "=== ESLINT FAILED ==="
        echo "$OUTPUT"
        echo ""
        echo "Fix these lint errors before continuing."
        exit 2
      fi
    fi
    ;;
  *.py)
    if command -v ruff &>/dev/null; then
      OUTPUT=$(cd "$WORKTREE" && ruff check "$FILE" 2>&1)
      if [ $? -ne 0 ]; then
        echo ""
        echo "=== RUFF FAILED ==="
        echo "$OUTPUT"
        exit 2
      fi
    fi
    ;;
esac

exit 0
