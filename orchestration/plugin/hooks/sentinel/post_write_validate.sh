#!/bin/bash
# SENTINEL Post-Write Validation Hook
# Validates that eval files have correct structure
#
# Environment:
#   SPRINTLESS_SHARED - the shared directory
#   CLAUDE_FILE - the file that was just written (injected by Claude Code)

SHARED="${SPRINTLESS_SHARED}"
FILE="${CLAUDE_FILE:-}"

# Only validate eval files
if [[ ! "$FILE" =~ -eval\.md$ ]] && [[ ! "$FILE" =~ final-review\.md$ ]]; then
  exit 0
fi

echo "Validating evaluation file: ${FILE}"
echo ""

# Check required sections
MISSING=""

if ! grep -q "## Summary" "$FILE"; then
  MISSING="${MISSING}Summary, "
fi

if ! grep -q "## Tests Run" "$FILE" && ! grep -q "## Test Results" "$FILE"; then
  MISSING="${MISSING}Tests Run, "
fi

if ! grep -q "## Verdict" "$FILE"; then
  MISSING="${MISSING}Verdict, "
fi

# Check verdict is valid
VERDICT=$(grep -oP '(?<=## Verdict\n)[A-Z_]+' "$FILE" 2>/dev/null || echo "")
if [ -n "$VERDICT" ]; then
  if [ "$VERDICT" != "APPROVED" ] && [ "$VERDICT" != "NEEDS_WORK" ]; then
    echo "ERROR: Invalid verdict '${VERDICT}'. Must be APPROVED or NEEDS_WORK."
    exit 2
  fi
fi

if [ -n "$MISSING" ]; then
  echo "ERROR: Missing required sections: ${MISSING%,*}"
  echo ""
  echo "Required sections for eval files:"
  echo "  - ## Summary"
  echo "  - ## Tests Run (or ## Test Results)"
  echo "  - ## Verdict (APPROVED or NEEDS_WORK)"
  echo ""
  echo "If NEEDS_WORK, also include:"
  echo "  - ## Issues Found"
  echo "  - ## Required Fixes"
  exit 2
fi

echo "Validation passed."
exit 0