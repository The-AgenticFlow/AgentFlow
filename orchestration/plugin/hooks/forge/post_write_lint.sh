#!/bin/bash
# Runs after every Write, Edit, MultiEdit tool call
# Validates atomic writes for shared/ artifacts and runs linter on source files
#
# Environment:
#   CLAUDE_TOOL_INPUT_FILE_PATH - the file that was written
#   SPRINTLESS_WORKTREE - the worktree directory

FILE="${CLAUDE_TOOL_INPUT_FILE_PATH}"
WORKTREE="${SPRINTLESS_WORKTREE}"

# For shared/ artifacts, ensure atomic write was used (.tmp + rename pattern)
case "$FILE" in
  */orchestration/pairs/*/shared/*)
    # Verify file was written atomically (should never see .tmp files at this point)
    if [[ "$FILE" == *.tmp ]]; then
      echo "ERROR: Temporary file leaked to filesystem: ${FILE}"
      echo "All shared/ writes must be atomic (write to .tmp, then rename)."
      exit 2
    fi
    # Validate JSON structure for specific artifact types
    case "$FILE" in
      */STATUS.json)
        if command -v python3 &> /dev/null; then
          python3 -c "import json, sys; json.load(open('$FILE'))" 2>&1
          if [ $? -ne 0 ]; then
            echo "INVALID: STATUS.json is not valid JSON"
            exit 2
          fi
        fi
        ;;
    esac
    exit 0
    ;;
esac

# Only lint source files (not config, docs, etc.)
case "$FILE" in
  *.ts|*.tsx)
    if command -v npx &> /dev/null; then
      OUTPUT=$(cd "$WORKTREE" && npx eslint "$FILE" --quiet 2>&1)
      if [ $? -ne 0 ]; then
        echo "Lint failed on ${FILE}:"
        echo "$OUTPUT"
        echo ""
        echo "Fix these lint errors before continuing."
        exit 2
      fi
    fi
    ;;
  *.rs)
    if command -v cargo &> /dev/null; then
      OUTPUT=$(cd "$WORKTREE" && cargo clippy --quiet --message-format=short 2>&1 | grep -A5 "$FILE" || true)
      if [ -n "$OUTPUT" ]; then
        echo "Clippy warnings for ${FILE}:"
        echo "$OUTPUT"
        echo ""
        echo "Fix these warnings before continuing."
        # Clippy warnings don't fail the build, but we want clean code
        # exit 2  # Uncomment to enforce zero warnings
      fi
    fi
    ;;
  *.py)
    if command -v ruff &> /dev/null; then
      OUTPUT=$(cd "$WORKTREE" && ruff check "$FILE" 2>&1)
      if [ $? -ne 0 ]; then
        echo "Ruff failed on ${FILE}:"
        echo "$OUTPUT"
        exit 2
      fi
    fi
    ;;
esac

exit 0