#!/bin/bash
# SENTINEL Session Start Hook
# Initializes the reviewer session

SHARED="${SPRINTLESS_SHARED:-.sprintless/shared}"

echo "=========================================="
echo "SENTINEL Session Starting"
echo "=========================================="
echo "Shared: ${SHARED}"
echo ""

# Check what needs to be reviewed
if [ -f "${SHARED}/PLAN.md" ]; then
  echo "PLAN.md found - Review the implementation plan"
fi

# Check for segment evaluations
SEGMENT_EVALS=$(ls "${SHARED}"/segment-*-eval.md 2>/dev/null | wc -l)
if [ "$SEGMENT_EVALS" -gt 0 ]; then
  echo "Found ${SEGMENT_EVALS} segment evaluation(s)"
fi

# Check for pending segment
PENDING=$(ls "${SHARED}"/segment-*.md 2>/dev/null | grep -v eval | head -1)
if [ -n "$PENDING" ]; then
  echo ""
  echo "Pending review: ${PENDING}"
fi

exit 0
