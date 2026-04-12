#!/bin/bash
# SENTINEL Session Start Hook
# Determines segment mode and reads the correct input artifacts
#
# Environment:
#   SPRINTLESS_SHARED - the shared directory
#   SPRINTLESS_SEGMENT - segment number (set by harness)

SHARED="${SPRINTLESS_SHARED}"
SEGMENT="${SPRINTLESS_SEGMENT:-1}"

echo "=============================================="
echo "  SENTINEL SESSION STARTED"
echo "=============================================="
echo ""
echo "Segment: ${SEGMENT}"
echo ""

# Determine mode based on what files exist
if [ "${SEGMENT}" = "final" ] || [ -f "${SHARED}/DONE.md" ]; then
  echo "MODE: FINAL_REVIEW"
  echo ""
  echo "Reading DONE.md to verify completion..."
  if [ -f "${SHARED}/DONE.md" ]; then
    echo "--- DONE.md ---"
    head -50 "${SHARED}/DONE.md"
    echo "..."
  else
    echo "ERROR: DONE.md not found. Cannot perform final review."
    exit 1
  fi
elif [ -f "${SHARED}/PLAN.md" ]; then
  echo "MODE: SEGMENT_REVIEW"
  echo ""
  echo "Reading PLAN.md and segment inputs..."
  echo ""
  echo "--- PLAN.md (first 30 lines) ---"
  head -30 "${SHARED}/PLAN.md"
  echo "..."
  echo ""
  if [ -f "${SHARED}/WORKLOG.md" ]; then
    echo "--- WORKLOG.md (last 20 lines) ---"
    tail -20 "${SHARED}/WORKLOG.md"
    echo ""
  fi
else
  echo "MODE: UNKNOWN - No PLAN.md found"
  echo "This may be an initial setup. Check TICKET.md and TASK.md."
  if [ -f "${SHARED}/TICKET.md" ]; then
    echo ""
    echo "--- TICKET.md ---"
    cat "${SHARED}/TICKET.md"
  fi
fi

echo ""
echo "=============================================="
echo "  YOUR MISSION"
echo "=============================================="
echo ""
echo "1. Read the segment changes from WORKLOG.md"
echo "2. Run tests and linters to verify quality"
echo "3. Write your evaluation to segment-${SEGMENT}-eval.md"
echo ""
echo "Evaluation must include:"
echo "  - ## Summary"
echo "  - ## Tests Run"
echo "  - ## Issues Found (if any)"
echo "  - ## Verdict (APPROVED / NEEDS_WORK)"
echo ""
echo "If NEEDS_WORK, list specific issues that must be fixed."
echo ""

exit 0