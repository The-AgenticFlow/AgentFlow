#!/bin/bash
# Runs when FORGE starts a new session
# This hook runs at the beginning of every FORGE session

PAIR_ID="${SPRINTLESS_PAIR_ID}"
TICKET_ID="${SPRINTLESS_TICKET_ID}"
SHARED="${SPRINTLESS_SHARED}"
WORKTREE="${SPRINTLESS_WORKTREE}"

# Log session start to shared event log
echo "[$(date -u +%Y-%m-%dT%H:%M:%SZ)] forge-${PAIR_ID} session_start ticket=${TICKET_ID}" \
  >> "${SHARED}/../events.log"

# Check if this is a resume (HANDOFF.md exists)
if [ -f "${SHARED}/HANDOFF.md" ]; then
  echo "=========================================="
  echo "  RESUME MODE: HANDOFF.md found"
  echo "=========================================="
  echo ""
  echo "Read ${SHARED}/HANDOFF.md before doing anything else."
  echo "Continue from the exact next step described in the handoff."
  echo ""
  echo "Key things to check:"
  echo "  1. Which segments are already complete"
  echo "  2. Which segment is in progress"
  echo "  3. Decisions already made (do not contradict)"
  echo "  4. Files already written (do not rewrite)"
  echo "  5. Exact next step to take"
  echo ""
else
  echo "=========================================="
  echo "  NEW SESSION: No handoff found"
  echo "=========================================="
  echo ""
  echo "Starting fresh on ticket ${TICKET_ID}"
  echo ""
  echo "IMPORTANT - Directory Structure:"
  echo "  CURRENT DIR (worktree): ${WORKTREE}"
  echo "    -> Write ALL source code, tests, package.json here"
  echo "  SHARED DIR: ${SHARED}"
  echo "    -> Write PLAN.md, WORKLOG.md, STATUS.json here"
  echo ""
  echo "First steps:"
  echo "  1. Read ${SHARED}/TICKET.md to understand the task"
  echo "  2. Read ${SHARED}/TASK.md for specific instructions"
  echo "  3. Use /plan command to create ${SHARED}/PLAN.md"
  echo "  4. Wait for CONTRACT.md from SENTINEL"
  echo ""
fi

# Show current state
echo "Environment:"
echo "  PAIR_ID:    ${PAIR_ID}"
echo "  TICKET_ID:  ${TICKET_ID}"
echo "  WORKTREE:   ${WORKTREE}"
echo "  SHARED:     ${SHARED}"
echo ""

exit 0