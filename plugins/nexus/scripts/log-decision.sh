#!/bin/bash
# NEXUS Stop Hook
# Logs the decision made this session

echo "NEXUS session ending. Decision logged."

# The actual decision is captured in the shared store
# This hook ensures we have a log entry

exit 0
