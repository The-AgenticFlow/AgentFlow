#!/bin/bash
# SENTINEL Stop Hook
# Ensures SENTINEL writes an eval file before exiting
#
# Environment:
#   SPRINTLESS_SHARED - the shared directory
#   SPRINTLESS_SEGMENT - segment number (set by harness)

SHARED="${SPRINTLESS_SHARED}"
SEGMENT="${SPRINTLESS_SEGMENT:-1}"

# Determine expected eval file
if [ "${SEGMENT}" = "final" ]; then
  EVAL_FILE="${SHARED}/final-review.md"
else
  EVAL_FILE="${SHARED}/segment-${SEGMENT}-eval.md"
fi

# Check if eval file exists
if [ -f "$EVAL_FILE" ]; then
  # Validate it has a verdict
  if grep -q "## Verdict" "$EVAL_FILE"; then
    VERDICT=$(grep -A1 "## Verdict" "$EVAL_FILE" | tail -1 | tr -d ' ')
    
    if [ "$VERDICT" = "APPROVED" ] || [ "$VERDICT" = "NEEDS_WORK" ]; then
      echo "Evaluation complete: ${VERDICT}"
      exit 0
    else
      echo "ERROR: Invalid verdict in ${EVAL_FILE}"
      echo "Verdict must be APPROVED or NEEDS_WORK, got: ${VERDICT}"
      exit 2
    fi
  else
    echo "ERROR: ${EVAL_FILE} missing ## Verdict section"
    exit 2
  fi
fi

# No eval file - block exit
echo "=============================================="
echo "  BLOCKED: Cannot exit without evaluation"
echo "=============================================="
echo ""
echo "You must write your evaluation to:"
echo "  ${EVAL_FILE}"
echo ""
echo "Required sections:"
echo "  - ## Summary"
echo "  - ## Tests Run"
echo "  - ## Issues Found (if any)"
echo "  - ## Verdict (APPROVED or NEEDS_WORK)"
echo ""
echo "If NEEDS_WORK, include:"
echo "  - ## Required Fixes (specific, actionable items)"
echo ""

exit 2