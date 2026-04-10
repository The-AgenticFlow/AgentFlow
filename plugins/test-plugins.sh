#!/bin/bash
# Plugin Test Script
# Tests all hook scripts and validates plugin structure

# Don't use set -e because we test commands that intentionally fail

PLUGIN_DIR="$(cd "$(dirname "$0")" && pwd)"
TEST_DIR="/tmp/agentflow-plugin-test"
SHARED="${TEST_DIR}/shared"
WORKTREE="${TEST_DIR}/worktree"

# Colors
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
NC='\033[0m' # No Color

pass() { echo -e "${GREEN}PASS${NC}: $1"; }
fail() { echo -e "${RED}FAIL${NC}: $1"; }
warn() { echo -e "${YELLOW}WARN${NC}: $1"; }

setup() {
  echo "=== Setting up test environment ==="
  rm -rf "$TEST_DIR"
  mkdir -p "$SHARED" "$WORKTREE"
  echo "Test directory: $TEST_DIR"
}

cleanup() {
  echo "=== Cleaning up ==="
  rm -rf "$TEST_DIR"
}

# ============================================
# STRUCTURE TESTS
# ============================================

test_plugin_structure() {
  echo ""
  echo "=== Testing Plugin Structure ==="
  
  for agent in nexus forge sentinel vessel lore; do
    echo "Checking $agent plugin..."
    
    # Check manifest
    if [ -f "${PLUGIN_DIR}/${agent}/.claude-plugin/plugin.json" ]; then
      pass "$agent: plugin.json exists"
    else
      fail "$agent: plugin.json missing"
    fi
    
    # Check agent definition
    if [ -f "${PLUGIN_DIR}/${agent}/agents/${agent}.md" ]; then
      pass "$agent: agent definition exists"
    else
      fail "$agent: agent definition missing"
    fi
    
    # Check hooks config
    if [ -f "${PLUGIN_DIR}/${agent}/hooks/hooks.json" ]; then
      pass "$agent: hooks.json exists"
    else
      fail "$agent: hooks.json missing"
    fi
    
    # Check MCP config
    if [ -f "${PLUGIN_DIR}/${agent}/.mcp.json" ]; then
      pass "$agent: .mcp.json exists"
    else
      fail "$agent: .mcp.json missing"
    fi
    
    # Check scripts are executable
    for script in "${PLUGIN_DIR}/${agent}/scripts/"*.sh; do
      if [ -x "$script" ]; then
        pass "$agent: $(basename $script) is executable"
      else
        fail "$agent: $(basename $script) not executable"
      fi
    done
  done
}

# ============================================
# FORGE HOOK TESTS
# ============================================

test_forge_session_start() {
  echo ""
  echo "=== Testing FORGE Session Start Hook ==="
  
  # Test new session (no handoff)
  SPRINTLESS_PAIR_ID=forge-1 \
  SPRINTLESS_TICKET_ID=T-42 \
  SPRINTLESS_SHARED="$SHARED" \
  SPRINTLESS_WORKTREE="$WORKTREE" \
  bash "${PLUGIN_DIR}/forge/scripts/session-start.sh" > /dev/null
  
  if [ $? -eq 0 ]; then
    pass "FORGE session-start: new session"
  else
    fail "FORGE session-start: new session"
  fi
  
  # Test resume session (with handoff)
  cat > "${SHARED}/HANDOFF.md" << 'EOF'
# Handoff for T-42

## Exact next step
Continue implementing segment 2.
EOF
  
  SPRINTLESS_PAIR_ID=forge-1 \
  SPRINTLESS_TICKET_ID=T-42 \
  SPRINTLESS_SHARED="$SHARED" \
  SPRINTLESS_WORKTREE="$WORKTREE" \
  bash "${PLUGIN_DIR}/forge/scripts/session-start.sh" > /dev/null
  
  if [ $? -eq 0 ]; then
    pass "FORGE session-start: resume with handoff"
  else
    fail "FORGE session-start: resume with handoff"
  fi
  
  rm -f "${SHARED}/HANDOFF.md"
}

test_forge_pre_bash_guard() {
  echo ""
  echo "=== Testing FORGE Pre-Bash Guard Hook ==="
  
  # Test blocked: git push
  CLAUDE_TOOL_INPUT_COMMAND="git push origin main" \
  SPRINTLESS_PAIR_ID=forge-1 \
  bash "${PLUGIN_DIR}/forge/scripts/pre-bash-guard.sh" 2>/dev/null
  
  if [ $? -eq 2 ]; then
    pass "FORGE pre-bash-guard: blocks git push"
  else
    fail "FORGE pre-bash-guard: should block git push"
  fi
  
  # Test blocked: force push
  CLAUDE_TOOL_INPUT_COMMAND="git push --force" \
  SPRINTLESS_PAIR_ID=forge-1 \
  bash "${PLUGIN_DIR}/forge/scripts/pre-bash-guard.sh" 2>/dev/null
  
  if [ $? -eq 2 ]; then
    pass "FORGE pre-bash-guard: blocks force push"
  else
    fail "FORGE pre-bash-guard: should block force push"
  fi
  
  # Test blocked: checkout main
  CLAUDE_TOOL_INPUT_COMMAND="git checkout main" \
  SPRINTLESS_PAIR_ID=forge-1 \
  bash "${PLUGIN_DIR}/forge/scripts/pre-bash-guard.sh" 2>/dev/null
  
  if [ $? -eq 2 ]; then
    pass "FORGE pre-bash-guard: blocks checkout main"
  else
    fail "FORGE pre-bash-guard: should block checkout main"
  fi
  
  # Test blocked: dangerous rm
  CLAUDE_TOOL_INPUT_COMMAND="rm -rf /" \
  SPRINTLESS_PAIR_ID=forge-1 \
  bash "${PLUGIN_DIR}/forge/scripts/pre-bash-guard.sh" 2>/dev/null
  
  if [ $? -eq 2 ]; then
    pass "FORGE pre-bash-guard: blocks dangerous rm"
  else
    fail "FORGE pre-bash-guard: should block dangerous rm"
  fi
  
  # Test allowed: normal command
  CLAUDE_TOOL_INPUT_COMMAND="ls -la" \
  SPRINTLESS_PAIR_ID=forge-1 \
  bash "${PLUGIN_DIR}/forge/scripts/pre-bash-guard.sh" > /dev/null
  
  if [ $? -eq 0 ]; then
    pass "FORGE pre-bash-guard: allows normal command"
  else
    fail "FORGE pre-bash-guard: should allow normal command"
  fi
}

test_forge_stop_require_artifact() {
  echo ""
  echo "=== Testing FORGE Stop Hook ==="
  
  # Test blocked: no artifact
  SPRINTLESS_SHARED="$SHARED" \
  bash "${PLUGIN_DIR}/forge/scripts/stop-require-artifact.sh" 2>/dev/null
  
  if [ $? -eq 2 ]; then
    pass "FORGE stop: blocks without artifact"
  else
    fail "FORGE stop: should block without artifact"
  fi
  
  # Test pass: valid STATUS.json
  cat > "${SHARED}/STATUS.json" << 'EOF'
{
  "status": "PR_OPENED",
  "pair": "forge-1",
  "ticket_id": "T-42",
  "files_changed": ["src/file.rs"]
}
EOF
  
  SPRINTLESS_SHARED="$SHARED" \
  bash "${PLUGIN_DIR}/forge/scripts/stop-require-artifact.sh" > /dev/null
  
  if [ $? -eq 0 ]; then
    pass "FORGE stop: allows with valid STATUS.json"
  else
    fail "FORGE stop: should allow with valid STATUS.json"
  fi
  
  rm -f "${SHARED}/STATUS.json"
  
  # Test pass: valid HANDOFF.md
  cat > "${SHARED}/HANDOFF.md" << 'EOF'
# Handoff

## Exact next step
Continue with segment 2.
EOF
  
  SPRINTLESS_SHARED="$SHARED" \
  bash "${PLUGIN_DIR}/forge/scripts/stop-require-artifact.sh" > /dev/null
  
  if [ $? -eq 0 ]; then
    pass "FORGE stop: allows with valid HANDOFF.md"
  else
    fail "FORGE stop: should allow with valid HANDOFF.md"
  fi
  
  rm -f "${SHARED}/HANDOFF.md"
  
  # Test blocked: invalid STATUS.json
  cat > "${SHARED}/STATUS.json" << 'EOF'
{
  "status": "INVALID_STATUS"
}
EOF
  
  SPRINTLESS_SHARED="$SHARED" \
  bash "${PLUGIN_DIR}/forge/scripts/stop-require-artifact.sh" 2>/dev/null
  
  if [ $? -eq 2 ]; then
    pass "FORGE stop: blocks with invalid STATUS.json"
  else
    fail "FORGE stop: should block with invalid STATUS.json"
  fi
  
  rm -f "${SHARED}/STATUS.json"
}

test_forge_pre_compact() {
  echo ""
  echo "=== Testing FORGE Pre-Compact Hook ==="
  
  SPRINTLESS_SHARED="$SHARED" \
  bash "${PLUGIN_DIR}/forge/scripts/pre-compact-handoff.sh" > /dev/null 2>&1
  
  # Pre-compact always blocks (exit 2) to force handoff
  if [ $? -eq 2 ]; then
    pass "FORGE pre-compact: triggers handoff"
  else
    fail "FORGE pre-compact: should trigger handoff"
  fi
}

# ============================================
# SENTINEL HOOK TESTS
# ============================================

test_sentinel_session_start() {
  echo ""
  echo "=== Testing SENTINEL Session Start Hook ==="
  
  SPRINTLESS_SHARED="$SHARED" \
  bash "${PLUGIN_DIR}/sentinel/scripts/session-start.sh" > /dev/null
  
  if [ $? -eq 0 ]; then
    pass "SENTINEL session-start: initializes"
  else
    fail "SENTINEL session-start: should initialize"
  fi
}

test_sentinel_post_write_validate() {
  echo ""
  echo "=== Testing SENTINEL Post-Write Validate Hook ==="
  
  # Test valid APPROVED segment eval
  cat > "${SHARED}/segment-1-eval.md" << 'EOF'
# Segment 1 Evaluation

## Verdict
APPROVED

## Summary
Good implementation.
EOF
  
  CLAUDE_TOOL_INPUT_FILE_PATH="${SHARED}/segment-1-eval.md" \
  bash "${PLUGIN_DIR}/sentinel/scripts/post-write-validate.sh" > /dev/null
  
  if [ $? -eq 0 ]; then
    pass "SENTINEL post-write: valid APPROVED eval"
  else
    fail "SENTINEL post-write: should allow valid APPROVED eval"
  fi
  
  # Test invalid: missing verdict
  cat > "${SHARED}/segment-2-eval.md" << 'EOF'
# Segment 2 Evaluation

## Summary
Missing verdict section.
EOF
  
  CLAUDE_TOOL_INPUT_FILE_PATH="${SHARED}/segment-2-eval.md" \
  bash "${PLUGIN_DIR}/sentinel/scripts/post-write-validate.sh" 2>/dev/null
  
  if [ $? -eq 2 ]; then
    pass "SENTINEL post-write: blocks missing verdict"
  else
    fail "SENTINEL post-write: should block missing verdict"
  fi
  
  # Test invalid: CHANGES_REQUESTED without feedback
  cat > "${SHARED}/segment-3-eval.md" << 'EOF'
# Segment 3 Evaluation

## Verdict
CHANGES_REQUESTED

## Summary
Some issues found.
EOF
  
  CLAUDE_TOOL_INPUT_FILE_PATH="${SHARED}/segment-3-eval.md" \
  bash "${PLUGIN_DIR}/sentinel/scripts/post-write-validate.sh" 2>/dev/null
  
  if [ $? -eq 2 ]; then
    pass "SENTINEL post-write: blocks CHANGES_REQUESTED without feedback"
  else
    fail "SENTINEL post-write: should block CHANGES_REQUESTED without feedback"
  fi
  
  # Test valid: CHANGES_REQUESTED with feedback
  cat > "${SHARED}/segment-3-eval.md" << 'EOF'
# Segment 3 Evaluation

## Verdict
CHANGES_REQUESTED

## Summary
Some issues found.

## Specific feedback
### Issue 1
- File: src/auth.rs
- Line: 42
- Problem: Missing error handling
- Required Fix: Add AppError wrapper
EOF
  
  CLAUDE_TOOL_INPUT_FILE_PATH="${SHARED}/segment-3-eval.md" \
  bash "${PLUGIN_DIR}/sentinel/scripts/post-write-validate.sh" > /dev/null
  
  if [ $? -eq 0 ]; then
    pass "SENTINEL post-write: allows CHANGES_REQUESTED with feedback"
  else
    fail "SENTINEL post-write: should allow CHANGES_REQUESTED with feedback"
  fi
  
  # Test valid final-review
  cat > "${SHARED}/final-review.md" << 'EOF'
# Final Review

## Verdict
APPROVED

## Summary
All segments complete.

## PR description
This PR implements feature X.
EOF
  
  CLAUDE_TOOL_INPUT_FILE_PATH="${SHARED}/final-review.md" \
  bash "${PLUGIN_DIR}/sentinel/scripts/post-write-validate.sh" > /dev/null
  
  if [ $? -eq 0 ]; then
    pass "SENTINEL post-write: valid final-review"
  else
    fail "SENTINEL post-write: should allow valid final-review"
  fi
  
  rm -f "${SHARED}"/segment-*.md "${SHARED}/final-review.md"
}

test_sentinel_pre_bash_readonly() {
  echo ""
  echo "=== Testing SENTINEL Pre-Bash Readonly Hook ==="
  
  # Test blocked: write operation
  CLAUDE_TOOL_INPUT_COMMAND="echo 'test' > src/file.rs" \
  SPRINTLESS_WORKTREE="$WORKTREE" \
  SPRINTLESS_SHARED="$SHARED" \
  bash "${PLUGIN_DIR}/sentinel/scripts/pre-bash-readonly.sh" 2>/dev/null
  
  if [ $? -eq 2 ]; then
    pass "SENTINEL pre-bash: blocks write operation"
  else
    fail "SENTINEL pre-bash: should block write operation"
  fi
  
  # Test allowed: read operation
  CLAUDE_TOOL_INPUT_COMMAND="cat src/file.rs" \
  SPRINTLESS_WORKTREE="$WORKTREE" \
  SPRINTLESS_SHARED="$SHARED" \
  bash "${PLUGIN_DIR}/sentinel/scripts/pre-bash-readonly.sh" > /dev/null
  
  if [ $? -eq 0 ]; then
    pass "SENTINEL pre-bash: allows read operation"
  else
    fail "SENTINEL pre-bash: should allow read operation"
  fi
}

test_sentinel_stop_require_eval() {
  echo ""
  echo "=== Testing SENTINEL Stop Hook ==="
  
  # Test blocked: no evaluation
  SPRINTLESS_SHARED="$SHARED" \
  bash "${PLUGIN_DIR}/sentinel/scripts/stop-require-eval.sh" 2>/dev/null
  
  if [ $? -eq 2 ]; then
    pass "SENTINEL stop: blocks without evaluation"
  else
    fail "SENTINEL stop: should block without evaluation"
  fi
  
  # Test pass: with segment eval
  touch "${SHARED}/segment-1-eval.md"
  
  SPRINTLESS_SHARED="$SHARED" \
  bash "${PLUGIN_DIR}/sentinel/scripts/stop-require-eval.sh" > /dev/null
  
  if [ $? -eq 0 ]; then
    pass "SENTINEL stop: allows with segment eval"
  else
    fail "SENTINEL stop: should allow with segment eval"
  fi
  
  rm -f "${SHARED}/segment-1-eval.md"
  
  # Test pass: with final review
  touch "${SHARED}/final-review.md"
  
  SPRINTLESS_SHARED="$SHARED" \
  bash "${PLUGIN_DIR}/sentinel/scripts/stop-require-eval.sh" > /dev/null
  
  if [ $? -eq 0 ]; then
    pass "SENTINEL stop: allows with final review"
  else
    fail "SENTINEL stop: should allow with final review"
  fi
  
  rm -f "${SHARED}/final-review.md"
}

# ============================================
# NEXUS HOOK TESTS
# ============================================

test_nexus_session_start() {
  echo ""
  echo "=== Testing NEXUS Session Start Hook ==="
  
  AGENTFLOW_STORE="$SHARED" \
  AGENTFLOW_REGISTRY="${TEST_DIR}/registry.json" \
  bash "${PLUGIN_DIR}/nexus/scripts/init-session.sh" > /dev/null
  
  if [ $? -eq 0 ]; then
    pass "NEXUS session-start: initializes"
  else
    fail "NEXUS session-start: should initialize"
  fi
}

test_nexus_stop() {
  echo ""
  echo "=== Testing NEXUS Stop Hook ==="
  
  bash "${PLUGIN_DIR}/nexus/scripts/log-decision.sh" > /dev/null
  
  if [ $? -eq 0 ]; then
    pass "NEXUS stop: logs decision"
  else
    fail "NEXUS stop: should log decision"
  fi
}

# ============================================
# JSON VALIDATION TESTS
# ============================================

test_json_validity() {
  echo ""
  echo "=== Testing JSON Validity ==="
  
  for agent in nexus forge sentinel vessel lore; do
    # Test plugin.json
    if python3 -c "import json; json.load(open('${PLUGIN_DIR}/${agent}/.claude-plugin/plugin.json'))" 2>/dev/null; then
      pass "$agent: plugin.json is valid JSON"
    else
      fail "$agent: plugin.json is invalid JSON"
    fi
    
    # Test hooks.json
    if python3 -c "import json; json.load(open('${PLUGIN_DIR}/${agent}/hooks/hooks.json'))" 2>/dev/null; then
      pass "$agent: hooks.json is valid JSON"
    else
      fail "$agent: hooks.json is invalid JSON"
    fi
    
    # Test .mcp.json
    if python3 -c "import json; json.load(open('${PLUGIN_DIR}/${agent}/.mcp.json'))" 2>/dev/null; then
      pass "$agent: .mcp.json is valid JSON"
    else
      fail "$agent: .mcp.json is invalid JSON"
    fi
  done
}

# ============================================
# RUN ALL TESTS
# ============================================

main() {
  echo "=========================================="
  echo "AgentFlow Plugin Test Suite"
  echo "=========================================="
  
  setup
  
  test_plugin_structure
  test_json_validity
  test_forge_session_start
  test_forge_pre_bash_guard
  test_forge_stop_require_artifact
  test_forge_pre_compact
  test_sentinel_session_start
  test_sentinel_post_write_validate
  test_sentinel_pre_bash_readonly
  test_sentinel_stop_require_eval
  test_nexus_session_start
  test_nexus_stop
  
  cleanup
  
  echo ""
  echo "=========================================="
  echo "All tests completed!"
  echo "=========================================="
}

main "$@"
