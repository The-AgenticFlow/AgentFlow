#!/bin/bash
# Runs on Stop - FORGE cannot exit without a terminal artifact
# This ensures FORGE always leaves a clear state for the harness
#
# Environment:
#   SPRINTLESS_SHARED - the shared directory

SHARED="${SPRINTLESS_SHARED}"

# Accept: STATUS.json written (done or blocked)
if [ -f "${SHARED}/STATUS.json" ]; then
  # Validate it has required fields
  if command -v python3 &> /dev/null; then
    VALID=$(python3 -c "
import json, sys
try:
    s = json.load(open('${SHARED}/STATUS.json'))
    required = ['status','pair','ticket_id','files_changed']
    missing = [k for k in required if k not in s]
    valid_statuses = ['PR_OPENED','BLOCKED','FUEL_EXHAUSTED','PENDING_REVIEW']
    # Note: IMPLEMENTATION_COMPLETE and COMPLETED are NOT valid terminal statuses
    # FORGE must push and create PR (PR_OPENED) or explicitly block
    if missing:
        print(f'missing: {missing}')
        sys.exit(1)
    if s['status'] not in valid_statuses:
        print(f'invalid status: {s[\"status\"]}')
        sys.exit(1)
except Exception as e:
    print(str(e))
    sys.exit(1)
" 2>&1)
    if [ $? -ne 0 ]; then
      echo "STATUS.json exists but is invalid: ${VALID}"
      echo "Fix STATUS.json before exiting."
      echo ""
      echo "Required fields: status, pair, ticket_id, files_changed"
      echo "Valid statuses: PR_OPENED, BLOCKED, FUEL_EXHAUSTED, PENDING_REVIEW"
      exit 2
    fi
  fi
  exit 0
fi

# Accept: HANDOFF.md written (context reset in progress)
if [ -f "${SHARED}/HANDOFF.md" ]; then
  # Verify HANDOFF.md has required sections
  if grep -q "## Exact next step" "${SHARED}/HANDOFF.md"; then
    exit 0
  else
    echo "HANDOFF.md is incomplete. It must contain '## Exact next step'."
    echo ""
    echo "Required sections in HANDOFF.md:"
    echo "  - ## Completed Segments"
    echo "  - ## Decisions"
    echo "  - ## Files Changed"
    echo "  - ## Exact next step"
    exit 2
  fi
fi

# Neither exists - block exit
echo "=============================================="
echo "  BLOCKED: Cannot exit without terminal artifact"
echo "=============================================="
echo ""
echo "You must write either:"
echo "  - ${SHARED}/STATUS.json  (if done or blocked)"
echo "  - ${SHARED}/HANDOFF.md   (if context reset needed)"
echo ""
echo "Use /status command if your work is complete."
echo "Use /handoff command if you need a context reset."
echo ""

exit 2