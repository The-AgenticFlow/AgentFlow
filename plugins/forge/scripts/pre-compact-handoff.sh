#!/bin/bash
# FORGE Pre-Compact Handoff Hook
# Triggers context reset with handoff when context window approaches limit

SHARED="${SPRINTLESS_SHARED:-.sprintless/shared}"

echo ""
echo "=========================================="
echo "CONTEXT RESET REQUIRED"
echo "=========================================="
echo ""
echo "Your context window is approaching its limit."
echo "Before this session ends, you must write a handoff."
echo ""
echo "Use the /handoff command now. It will:"
echo "  1. Collect everything needed for the handoff"
echo "  2. Write ${SHARED}/HANDOFF.md"
echo "  3. Update WORKLOG.md with current state"
echo "  4. Exit cleanly"
echo ""
echo "A fresh FORGE session will read your handoff and continue."
echo "Do not attempt to continue working - write the handoff now."
echo ""

exit 2
