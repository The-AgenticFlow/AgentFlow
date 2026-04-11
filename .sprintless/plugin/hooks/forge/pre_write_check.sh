#!/bin/bash
# Runs before every Write, Edit, MultiEdit tool call
# Enforces dynamic file ownership locking via flock
# 
# Environment:
#   CLAUDE_TOOL_INPUT_FILE_PATH - the file being written
#   SPRINTLESS_PAIR_ID - the current pair ID
#   SPRINTLESS_SHARED - the shared directory

FILE="${CLAUDE_TOOL_INPUT_FILE_PATH}"
PAIR_ID="${SPRINTLESS_PAIR_ID}"
LOCKS_DIR="${SPRINTLESS_SHARED}/../locks"

# Skip lock check for shared/ artifacts - those are pair-scoped already
case "$FILE" in
  */.sprintless/pairs/*/shared/*)
    exit 0
    ;;
esac

# Also skip if FILE is relative and matches shared pattern
case "$FILE" in
  shared/*|./shared/*)
    exit 0
    ;;
esac

# Create locks directory if needed
mkdir -p "$LOCKS_DIR" 2>/dev/null

# Generate lock filename (sha256 hash of filepath to avoid path issues)
LOCK_HASH=$(echo -n "$FILE" | sha256sum | cut -d' ' -f1)
LOCK_FILE="${LOCKS_DIR}/${LOCK_HASH}.lock"
LOCK_JSON="${LOCKS_DIR}/${LOCK_HASH}.json"

# Attempt atomic lock acquisition using flock
{
  # Acquire exclusive lock on .lock file (non-blocking)
  flock -x -n 200 || {
    echo "BLOCKED: Another process is currently locking ${FILE}"
    echo "Waiting for lock to be released..."
    exit 2
  }
  
  # Check if lock JSON exists and who owns it
  if [ -f "$LOCK_JSON" ]; then
    OWNER=$(cat "$LOCK_JSON" | grep -o '"pair"[[:space:]]*:[[:space:]]*"[^"]*"' | head -1 | cut -d'"' -f4)
    if [ "$OWNER" != "$PAIR_ID" ]; then
      echo "BLOCKED: ${FILE} is currently locked by ${OWNER}."
      echo ""
      echo "This file is being modified by another pair."
      echo ""
      echo "Options:"
      echo "  1. Find an alternative implementation that avoids this file"
      echo "  2. Wait for ${OWNER} to complete and release the lock"
      echo "  3. Set STATUS.json to BLOCKED with reason FILE_LOCK_CONFLICT"
      echo ""
      echo "Lock details in: ${LOCK_JSON}"
      
      exit 2
    fi
    # Lock belongs to us - proceed
  else
    # No lock exists - acquire it
    cat > "$LOCK_JSON" << EOF
{
  "pair": "${PAIR_ID}",
  "file": "${FILE}",
  "acquired_at": "$(date -u +%Y-%m-%dT%H:%M:%SZ)"
}
EOF
  fi
  
} 200>"$LOCK_FILE"

# Lock acquired or already owned by us - proceed with write
exit 0