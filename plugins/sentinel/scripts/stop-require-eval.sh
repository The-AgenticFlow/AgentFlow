#!/bin/bash
# SENTINEL Stop Hook
# Requires an evaluation before exit

SHARED="${SPRINTLESS_SHARED:-.sprintless/shared}"

# Check for segment evaluations
EVALS=$(ls "${SHARED}"/segment-*-eval.md 2>/dev/null | wc -l)

# Check for final review
FINAL=$(ls "${SHARED}/final-review.md" 2>/dev/null | wc -l)

if [ "$EVALS" -eq 0 ] && [ "$FINAL" -eq 0 ]; then
  echo ""
  echo "=== BLOCKED: No Evaluation Written ==="
  echo "SENTINEL must write an evaluation before exiting."
  echo "Either segment-N-eval.md or final-review.md must exist."
  exit 2
fi

exit 0
