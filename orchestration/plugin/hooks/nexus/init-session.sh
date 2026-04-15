#!/bin/bash
# NEXUS Session Start Hook
# Initializes the orchestrator session

echo "NEXUS session starting..."
echo "Reading registry from: ${AGENTFLOW_REGISTRY:-.agent/registry.json}"
echo "Store path: ${AGENTFLOW_STORE:-.agent/store.json}"

# Check if jq is installed
if ! command -v jq &> /dev/null; then
  echo "ERROR: jq is not installed. Please install it to use NEXUS."
  exit 1
fi

# Check for pending command gate items
if [ -n "${AGENTFLOW_STORE}" ] && [ -f "${AGENTFLOW_STORE}" ]; then
  GATE_PENDING=$(jq -r '.command_gate | length // 0' "${AGENTFLOW_STORE}" 2>/dev/null || echo "0")
  if [ "$GATE_PENDING" -gt 0 ]; then
    echo "WARNING: ${GATE_PENDING} pending command(s) awaiting approval"
  fi
fi

exit 0
