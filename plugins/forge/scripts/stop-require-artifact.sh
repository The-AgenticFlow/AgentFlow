#!/bin/bash
# FORGE Stop Hook
# Requires a terminal artifact before exit

SHARED="${SPRINTLESS_SHARED:-.sprintless/shared}"

# Accept: STATUS.json written (done or blocked)
if [ -f "${SHARED}/STATUS.json" ]; then
  # Validate required fields
  if command -v python3 &>/dev/null; then
    VALID=$(python3 -c "
import json, sys
try:
    s = json.load(open('${SHARED}/STATUS.json'))
    required = ['status','pair','ticket_id','files_changed']
    missing = [k for k in required if k not in s]
    valid_statuses = ['PR_OPENED','BLOCKED','FUEL_EXHAUSTED']
    if missing:
        print(f'missing fields: {missing}')
        sys.exit(1)
    if s['status'] not in valid_statuses:
        print(f'invalid status: {s[\"status\"]}')
        sys.exit(1)
except Exception as e:
    print(str(e))
    sys.exit(1)
" 2>&1)
    if [ $? -ne 0 ]; then
      echo ""
      echo "=== INVALID STATUS.json ==="
      echo "STATUS.json exists but is invalid: ${VALID}"
      echo "Fix STATUS.json before exiting."
      exit 2
    fi
  fi
  exit 0
fi

# Accept: HANDOFF.md written (context reset in progress)
if [ -f "${SHARED}/HANDOFF.md" ]; then
  if grep -q "## Exact next step" "${SHARED}/HANDOFF.md"; then
    exit 0
  else
    echo ""
    echo "=== INCOMPLETE HANDOFF.md ==="
    echo "HANDOFF.md is incomplete. It must contain '## Exact next step'."
    exit 2
  fi
fi

# Neither exists - block exit
echo ""
echo "=== BLOCKED: No terminal artifact ==="
echo "Cannot exit without writing either:"
echo "  - ${SHARED}/STATUS.json  (if done or blocked)"
echo "  - ${SHARED}/HANDOFF.md   (if context reset)"
echo ""
echo "Use /status command if done."
echo "Use /handoff command if you need a context reset."

exit 2
