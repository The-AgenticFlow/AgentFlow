#!/bin/bash
# Runs on PreCompact - converts compaction to clean context reset
# This hook fires when the context window is approaching its limit
#
# Environment:
#   SPRINTLESS_SHARED - the shared directory

SHARED="${SPRINTLESS_SHARED}"

echo "=============================================="
echo "  CONTEXT RESET REQUIRED"
echo "=============================================="
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
echo ""
echo "DO NOT attempt to continue working - write the handoff now."
echo ""

exit 2