#!/bin/bash
# SENTINEL Post-Write Validate Hook
# Validates evaluation files on write

FILE="${CLAUDE_TOOL_INPUT_FILE_PATH}"

# Only validate evaluation artifacts
case "$FILE" in
  *segment-*-eval.md)
    # Must have Verdict section
    if ! grep -q "^## Verdict" "$FILE" 2>/dev/null; then
      echo ""
      echo "=== INVALID: Missing Verdict ==="
      echo "Segment evaluation must have '## Verdict' section."
      exit 2
    fi
    # CHANGES_REQUESTED must have specific feedback
    if grep -q "CHANGES_REQUESTED" "$FILE" 2>/dev/null; then
      if ! grep -q "^## Specific feedback" "$FILE" 2>/dev/null; then
        echo ""
        echo "=== INVALID: Missing Specific Feedback ==="
        echo "CHANGES_REQUESTED requires '## Specific feedback' section"
        echo "with file:line:problem:fix for each item."
        exit 2
      fi
    fi
    ;;
  *final-review.md)
    if ! grep -q "^## Verdict" "$FILE" 2>/dev/null; then
      echo ""
      echo "=== INVALID: Missing Verdict ==="
      echo "Final review must have '## Verdict' section."
      exit 2
    fi
    if grep -q "APPROVED" "$FILE" 2>/dev/null; then
      if ! grep -q "^## PR description" "$FILE" 2>/dev/null; then
        echo ""
        echo "=== INVALID: Missing PR Description ==="
        echo "APPROVED final-review must include '## PR description' section."
        exit 2
      fi
    fi
    ;;
esac

exit 0
