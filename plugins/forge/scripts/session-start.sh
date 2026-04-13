#!/bin/bash
# FORGE Session Start Hook
# Initializes the worker session and checks for handoff

PAIR_ID="${SPRINTLESS_PAIR_ID:-forge-1}"
TICKET_ID="${SPRINTLESS_TICKET_ID:-unknown}"
SHARED="${SPRINTLESS_SHARED:-.sprintless/shared}"
WORKTREE="${SPRINTLESS_WORKTREE:-.}"

echo "=========================================="
echo "FORGE Session Starting"
echo "=========================================="
echo "Pair ID:    ${PAIR_ID}"
echo "Ticket:     ${TICKET_ID}"
echo "Worktree:   ${WORKTREE}"
echo "Shared:     ${SHARED}"
echo "=========================================="

# Check for handoff (resume mode)
if [ -f "${SHARED}/HANDOFF.md" ]; then
  echo ""
  echo "*** RESUME MODE ***"
  echo "HANDOFF.md found. Read it first:"
  echo "  ${SHARED}/HANDOFF.md"
  echo "Continue from the exact next step described in the handoff."
else
  echo ""
  echo "*** NEW SESSION ***"
  echo "No handoff found. Begin by reading:"
  echo "  - ${SHARED}/TICKET.md"
  echo "  - ${SHARED}/TASK.md"
  echo "  - ${SHARED}/CONTRACT.md (if exists)"
fi

exit 0
