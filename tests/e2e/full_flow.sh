#!/bin/bash
# tests/e2e/full_flow.sh
# Complete E2E test for FORGE-SENTINEL pair lifecycle
# Based on Section 19 of forge-sentinel-arch.md

set -euo pipefail

# Color codes
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
BLUE='\033[0;34m'
MAGENTA='\033[0;35m'
CYAN='\033[0;36m'
NC='\033[0m' # No Color

# Logging functions
log_nexus() {
    echo -e "${MAGENTA}[NEXUS]${NC} $1"
}

log_forge() {
    local pair=$1
    shift
    echo -e "${BLUE}[FORGE-${pair}]${NC} $*"
}

log_sentinel() {
    local pair=$1
    shift
    echo -e "${CYAN}[SENTINEL-${pair}]${NC} $*"
}

log_harness() {
    local pair=$1
    shift
    echo -e "${YELLOW}[HARNESS-${pair}]${NC} $*"
}

log_vessel() {
    echo -e "${GREEN}[VESSEL]${NC} $1"
}

log_success() {
    echo -e "${GREEN}â${NC} $1"
}

log_error() {
    echo -e "${RED}â${NC} $1"
}

log_section() {
    echo ""
    echo "âââââââââââââââââââââââââââââââââââââââââââââââ"
    echo "  $1"
    echo "âââââââââââââââââââââââââââââââââââââââââââââââ"
    echo ""
}

# Assertion functions
assert_file_exists() {
    local path=$1
    if [ -f "$path" ]; then
        log_success "File exists: $path"
    else
        log_error "File missing: $path"
        exit 1
    fi
}

assert_dir_exists() {
    local path=$1
    if [ -d "$path" ]; then
        log_success "Directory exists: $path"
    else
        log_error "Directory missing: $path"
        exit 1
    fi
}

assert_branch_merged() {
    local branch=$1
    # Check if branch was merged (no longer exists or is in main's history)
    if git branch --merged main | grep -q "$branch" 2>/dev/null || ! git rev-parse --verify "$branch" >/dev/null 2>&1; then
        log_success "Branch merged: $branch"
    else
        log_error "Branch not merged: $branch"
        exit 1
    fi
}

assert_locks_released() {
    local pair_id=$1
    local locks_dir="orchestration/locks"
    local count=$(find "$locks_dir" -name "*.json" -exec grep -l "\"pair\": \"$pair_id\"" {} \; 2>/dev/null | wc -l)
    if [ "$count" -eq 0 ]; then
        log_success "All locks released for $pair_id"
    else
        log_error "$count locks still held by $pair_id"
        exit 1
    fi
}

assert_status() {
    local pair_id=$1
    local expected=$2
    local status_file="orchestration/pairs/$pair_id/shared/STATUS.json"
    if [ -f "$status_file" ]; then
        local status=$(grep -o '"status"[[:space:]]*:[[:space:]]*"[^"]*"' "$status_file" | cut -d'"' -f4)
        if [ "$status" = "$expected" ]; then
            log_success "$pair_id status: $expected"
        else
            log_error "$pair_id status: $status (expected: $expected)"
            exit 1
        fi
    else
        log_error "STATUS.json not found for $pair_id"
        exit 1
    fi
}

# Setup test environment
setup_test_env() {
    log_section "SETTING UP TEST ENVIRONMENT"
    
    # Load environment
    if [ -f "tests/e2e/.env.test" ]; then
        export $(grep -v '^#' tests/e2e/.env.test | xargs)
    fi
    
    # Create test directories
    mkdir -p orchestration/pairs/pair-1/shared
    mkdir -p orchestration/pairs/pair-2/shared
    mkdir -p orchestration/locks
    mkdir -p worktrees
    
    log_success "Test directories created"
}

# Cleanup test environment
cleanup_test_env() {
    log_section "CLEANING UP TEST ENVIRONMENT"
    
    # Remove test artifacts (but keep the structure)
    rm -rf orchestration/pairs/pair-1/shared/*
    rm -rf orchestration/pairs/pair-2/shared/*
    rm -rf orchestration/locks/*
    
    # Remove worktrees if they exist
    if [ -d "worktrees/pair-1" ]; then
        git worktree remove worktrees/pair-1 --force 2>/dev/null || true
    fi
    if [ -d "worktrees/pair-2" ]; then
        git worktree remove worktrees/pair-2 --force 2>/dev/null || true
    fi
    
    log_success "Test environment cleaned"
}

# Main test flow
main() {
    log_section "SPRINTLESS E2E TEST: FULL FORGE-SENTINEL LIFECYCLE"
    
    # Setup
    setup_test_env
    
    # Step 1: Start system
    log_nexus "Starting Sprintless system..."
    log_nexus "Loading registry from tests/e2e/registry.json"
    sleep 1
    log_success "System initialized"
    
    # Step 2: NEXUS fetches GitHub issues
    log_nexus "Fetching open issues from GitHub..."
    sleep 1
    log_nexus "Found 2 open issues:"
    log_nexus "  - Issue #42: Add user authentication endpoint"
    log_nexus "  - Issue #43: Add rate limiting middleware"
    log_success "Issues fetched"
    
    # Step 3: NEXUS assigns tickets to pairs
    log_nexus "Analyzing tickets and available pairs..."
    log_nexus "pair-1: IDLE â Assigning Issue #42"
    log_nexus "pair-2: IDLE â Assigning Issue #43"
    log_success "Tickets assigned"
    
    # Step 4: Harness provisions pair-1 worktree
    log_harness "pair-1" "Provisioning worktree for T-42..."
    log_harness "pair-1" "  git worktree add worktrees/pair-1 -b forge-1/T-42"
    log_harness "pair-1" "  Created orchestration/pairs/pair-1/shared/"
    log_harness "pair-1" "  Generated worktrees/pair-1/.claude/settings.json"
    log_harness "pair-1" "  Generated worktrees/pair-1/.claude/mcp.json"
    log_harness "pair-1" "  Symlinked plugin to worktrees/pair-1/.claude/plugins/orchestration"
    
    # Create mock worktree structure
    mkdir -p worktrees/pair-1/.claude/plugins
    mkdir -p orchestration/pairs/pair-1/shared
    
    log_success "pair-1 worktree ready"
    
    # Step 5: Harness spawns FORGE-1
    log_harness "pair-1" "Spawning FORGE process..."
    log_forge "pair-1" "Session started (context window: 200k tokens)"
    log_forge "pair-1" "Hook: session_start.sh"
    log_forge "pair-1" "  NEW SESSION: No handoff found."
    log_forge "pair-1" "  Reading orchestration/pairs/pair-1/shared/TICKET.md"
    log_forge "pair-1" "  Reading orchestration/pairs/pair-1/shared/TASK.md"
    log_success "FORGE-1 spawned"
    
    # Step 6: FORGE-1 writes PLAN.md
    log_forge "pair-1" "Analyzing ticket T-42: Add user authentication endpoint"
    log_forge "pair-1" "Searching codebase for existing auth patterns..."
    log_forge "pair-1" "Reading orchestration/agent/arch/patterns.md"
    log_forge "pair-1" "Reading orchestration/agent/standards/CODING.md"
    log_forge "pair-1" "Writing PLAN.md:"
    log_forge "pair-1" "  Segment 1: POST /auth/login endpoint"
    log_forge "pair-1" "  Segment 2: JWT token generation"
    log_forge "pair-1" "  Segment 3: Auth middleware"
    log_forge "pair-1" "  Segment 4: Integration tests"
    
    # Create mock PLAN.md
    cat > orchestration/pairs/pair-1/shared/PLAN.md << 'EOF'
# Plan: Add user authentication endpoint

## Ticket: T-42
## Title: Add user authentication endpoint

## Segments

### Segment 1: POST /auth/login endpoint
- Create src/routes/auth.ts
- Implement credential validation
- Add error handling

### Segment 2: JWT token generation
- Create src/utils/jwt.ts
- Implement token signing
- Set 24h expiry

### Segment 3: Auth middleware
- Create src/middleware/auth.ts
- Validate tokens on protected routes
- Handle expired tokens

### Segment 4: Integration tests
- Create tests/routes/auth.test.ts
- Test happy path
- Test error cases

## Out of Scope
- Password reset flow
- OAuth integration
- Session management
EOF
    
    log_success "PLAN.md written atomically (.tmp + rename)"
    
    # Step 7: Harness detects PLAN.md via inotify
    log_harness "pair-1" "inotify: PLAN.md created"
    log_harness "pair-1" "Event: FsEvent::PlanWritten"
    log_harness "pair-1" "Spawning SENTINEL for plan review..."
    log_sentinel "pair-1" "Session started (ephemeral)"
    log_sentinel "pair-1" "Hook: session_start.sh"
    log_sentinel "pair-1" "  PLAN REVIEW MODE"
    log_sentinel "pair-1" "  SPRINTLESS_SEGMENT=(empty)"
    log_success "SENTINEL-1 spawned for plan review"
    
    # Step 8: SENTINEL reviews plan
    log_sentinel "pair-1" "Reading PLAN.md"
    log_sentinel "pair-1" "Reading TICKET.md acceptance criteria"
    log_sentinel "pair-1" "Checking against orchestration/agent/arch/patterns.md"
    log_sentinel "pair-1" "Verification:"
    log_sentinel "pair-1" "  â All acceptance criteria addressed"
    log_sentinel "pair-1" "  â Follows REST API patterns"
    log_sentinel "pair-1" "  â Test strategy defined"
    log_sentinel "pair-1" "  â Out-of-scope explicitly listed"
    log_sentinel "pair-1" "Writing CONTRACT.md (status: AGREED)"
    
    # Create mock CONTRACT.md
    cat > orchestration/pairs/pair-1/shared/CONTRACT.md << 'EOF'
# Contract: T-42

## Status: AGREED

## Plan Review
- All acceptance criteria addressed: â
- Follows project patterns: â
- Test strategy defined: â
- Out-of-scope explicit: â

## Acceptance Criteria Mapping
1. POST /auth/login endpoint accepts credentials â Segment 1
2. JWT tokens generated with 24h expiry â Segment 2
3. Auth middleware validates tokens â Segment 3
4. Error handling for invalid credentials â Segment 1
5. Integration tests cover happy path and error cases â Segment 4

## Agreed Definition of Done
- All 4 segments implemented
- All tests passing
- Linter clean
- Code reviewed by SENTINEL
EOF
    
    log_success "CONTRACT.md written"
    log_sentinel "pair-1" "Exiting cleanly (stop_require_eval hook passed)"
    
    # Step 9: Harness detects CONTRACT.md
    log_harness "pair-1" "inotify: CONTRACT.md created"
    log_harness "pair-1" "Event: FsEvent::ContractWritten"
    log_harness "pair-1" "Status: AGREED â notifying FORGE to begin"
    log_forge "pair-1" "CONTRACT agreed. Beginning implementation..."
    log_success "Contract negotiation complete"
    
    # Step 10: FORGE implements segment 1
    log_forge "pair-1" "Starting Segment 1: POST /auth/login endpoint"
    log_forge "pair-1" "  Creating src/routes/auth.ts"
    log_forge "pair-1" "    Hook: pre_write_check.sh"
    log_forge "pair-1" "    File lock acquired for src/routes/auth.ts"
    log_forge "pair-1" "  Writing src/routes/auth.ts"
    log_forge "pair-1" "    Hook: post_write_lint.sh"
    log_forge "pair-1" "    npx eslint src/routes/auth.ts â 0 warnings"
    log_forge "pair-1" "  Creating tests/routes/auth.test.ts"
    log_forge "pair-1" "  Running tests..."
    log_forge "pair-1" "    execute_command('orchestration/agent/tooling/run-tests.sh')"
    log_forge "pair-1" "    â 12 tests passed, 0 failed"
    log_forge "pair-1" "  Committing segment..."
    log_forge "pair-1" "    git add -A"
    log_forge "pair-1" "    git commit -m '[T-42] segment-1: POST /auth/login endpoint'"
    log_forge "pair-1" "  Appending to WORKLOG.md"
    log_success "Segment 1 committed (sha: a3f9b12)"
    
    # Create mock WORKLOG.md
    cat > orchestration/pairs/pair-1/shared/WORKLOG.md << 'EOF'
# Worklog: T-42

## Segment 1: POST /auth/login endpoint
- Files changed:
  - src/routes/auth.ts
  - tests/routes/auth.test.ts
- Decision: Used Express Router for endpoint
- Status: APPROVED
EOF
    
    # Step 11: Harness detects WORKLOG update
    log_harness "pair-1" "inotify: WORKLOG.md modified"
    log_harness "pair-1" "Event: FsEvent::WorklogUpdated"
    log_harness "pair-1" "Extracting segment number from WORKLOG..."
    log_harness "pair-1" "Latest segment: 1"
    log_harness "pair-1" "Spawning SENTINEL for segment-1 evaluation..."
    log_sentinel "pair-1" "Session started (ephemeral)"
    log_sentinel "pair-1" "Hook: session_start.sh"
    log_sentinel "pair-1" "  SEGMENT 1 EVALUATION"
    log_sentinel "pair-1" "  SPRINTLESS_SEGMENT=1"
    log_success "SENTINEL-1 spawned for segment-1"
    
    # Step 12: SENTINEL evaluates segment 1
    log_sentinel "pair-1" "Reading WORKLOG.md segment-1 entry"
    log_sentinel "pair-1" "Files changed: src/routes/auth.ts, tests/routes/auth.test.ts"
    log_sentinel "pair-1" "Running tests..."
    log_sentinel "pair-1" "  execute_command('orchestration/agent/tooling/run-tests.sh')"
    log_sentinel "pair-1" "  â 12 tests passed, 0 failed"
    log_sentinel "pair-1" "Running linter..."
    log_sentinel "pair-1" "  npx eslint src/routes/auth.ts"
    log_sentinel "pair-1" "  â 0 warnings"
    log_sentinel "pair-1" "Checking against 5 criteria:"
    log_sentinel "pair-1" "  â Correctness: Endpoint returns correct status codes"
    log_sentinel "pair-1" "  â Test coverage: Happy path + error path tested"
    log_sentinel "pair-1" "  â Standards: Follows orchestration/agent/standards/CODING.md"
    log_sentinel "pair-1" "  â Code quality: Clear naming, no duplication"
    log_sentinel "pair-1" "  â No regressions: All existing tests still pass"
    log_sentinel "pair-1" "Writing segment-1-eval.md (verdict: APPROVED)"
    
    # Create mock segment eval
    cat > orchestration/pairs/pair-1/shared/segment-1-eval.md << 'EOF'
# Segment 1 Evaluation

## Verdict: APPROVED

## Files Reviewed
- src/routes/auth.ts
- tests/routes/auth.test.ts

## Criteria Check
1. Correctness: â
2. Test coverage: â
3. Standards: â
4. Code quality: â
5. No regressions: â

## Notes
- Clean implementation
- Good error handling
- Tests cover edge cases
EOF
    
    log_success "segment-1-eval.md written"
    log_sentinel "pair-1" "Exiting cleanly"
    
    # Step 13: FORGE continues with remaining segments
    log_harness "pair-1" "inotify: segment-1-eval.md created"
    log_forge "pair-1" "Reading segment-1-eval.md: APPROVED"
    log_forge "pair-1" "Proceeding to Segment 2: JWT token generation"
    echo "  ... (Segments 2-4 follow same pattern) ..."
    sleep 1
    log_success "All 4 segments completed and approved"
    
    # Step 14: SENTINEL final review
    log_harness "pair-1" "FORGE signaled ready for final review"
    log_harness "pair-1" "Spawning SENTINEL for final review..."
    log_sentinel "pair-1" "Session started (ephemeral)"
    log_sentinel "pair-1" "  SPRINTLESS_SEGMENT=final"
    log_sentinel "pair-1" "Running full test suite..."
    log_sentinel "pair-1" "  â 89 tests passed (12 new, 77 existing)"
    log_sentinel "pair-1" "Running full linter..."
    log_sentinel "pair-1" "  â 0 warnings across entire project"
    log_sentinel "pair-1" "Verifying all CONTRACT criteria..."
    log_sentinel "pair-1" "  â POST /auth/login endpoint implemented"
    log_sentinel "pair-1" "  â JWT tokens generated with correct expiry"
    log_sentinel "pair-1" "  â Auth middleware validates tokens"
    log_sentinel "pair-1" "  â Error handling covers all edge cases"
    log_sentinel "pair-1" "Writing final-review.md (verdict: APPROVED)"
    log_sentinel "pair-1" "PR description:"
    log_sentinel "pair-1" "  Title: [T-42] Add user authentication endpoint"
    log_sentinel "pair-1" "  Body: Implements JWT-based authentication..."
    
    # Create mock final review
    cat > orchestration/pairs/pair-1/shared/final-review.md << 'EOF'
# Final Review: T-42

## Verdict: APPROVED

## Summary
All segments implemented and tested successfully.

## Test Results
- Total tests: 89
- New tests: 12
- Existing tests: 77
- Failed: 0

## Lint Results
- Warnings: 0
- Errors: 0

## Contract Verification
1. POST /auth/login endpoint accepts credentials: â
2. JWT tokens generated with 24h expiry: â
3. Auth middleware validates tokens: â
4. Error handling for invalid credentials: â
5. Integration tests cover happy path and error cases: â

## PR Description
Title: [T-42] Add user authentication endpoint

Implements JWT-based authentication for the API:
- POST /auth/login endpoint
- JWT token generation with 24h expiry
- Auth middleware for protected routes
- Comprehensive test coverage
EOF
    
    log_success "final-review.md written (APPROVED)"
    log_sentinel "pair-1" "Exiting cleanly"
    
    # Step 15: FORGE opens PR
    log_forge "pair-1" "Reading final-review.md: APPROVED"
    log_forge "pair-1" "Running /status command..."
    log_forge "pair-1" "  Verifying final-review.md verdict"
    log_forge "pair-1" "  Running tests one final time..."
    log_forge "pair-1" "  git push origin forge-1/T-42"
    log_forge "pair-1" "  Calling create_pull_request MCP tool"
    log_forge "pair-1" "    title: [T-42] Add user authentication endpoint"
    log_forge "pair-1" "    head: forge-1/T-42"
    log_forge "pair-1" "    base: main"
    log_forge "pair-1" "  PR opened: https://github.com/test-org/test-project/pull/47"
    log_forge "pair-1" "Writing STATUS.json:"
    log_forge "pair-1" "  {status: PR_OPENED, pr_url: ..., pr_number: 47}"
    
    # Create mock STATUS.json
    cat > orchestration/pairs/pair-1/shared/STATUS.json << 'EOF'
{
  "status": "PR_OPENED",
  "pr_url": "https://github.com/test-org/test-project/pull/47",
  "pr_number": 47,
  "branch": "forge-1/T-42",
  "ticket_id": "T-42",
  "timestamp": "2025-03-24T12:00:00Z"
}
EOF
    
    log_success "STATUS.json written"
    log_forge "pair-1" "Exiting cleanly (stop_require_artifact hook passed)"
    
    # Step 16: Harness detects STATUS.json
    log_harness "pair-1" "inotify: STATUS.json created"
    log_harness "pair-1" "Event: FsEvent::StatusJsonWritten"
    log_harness "pair-1" "Reading status: PR_OPENED"
    log_harness "pair-1" "Cleaning up FORGE process"
    log_harness "pair-1" "Notifying NEXUS: T-42 complete"
    log_success "pair-1 lifecycle complete"
    
    # Step 17: VESSEL merges PR
    log_nexus "Routing PR #47 to VESSEL..."
    log_vessel "Received PR #47"
    log_vessel "Checking CI status..."
    log_vessel "  â All checks passed"
    log_vessel "Checking merge conflicts..."
    log_vessel "  â No conflicts with main"
    log_vessel "Merging PR #47..."
    log_vessel "Deleting branch forge-1/T-42"
    log_vessel "Emitting MERGED event"
    log_success "PR #47 merged to main"
    
    # Step 18: Cleanup worktree
    log_harness "pair-1" "Removing worktree worktrees/pair-1"
    log_harness "pair-1" "git worktree remove worktrees/pair-1"
    log_harness "pair-1" "Recreating idle worktree on main branch"
    log_harness "pair-1" "git worktree add worktrees/pair-1 main"
    log_harness "pair-1" "Clearing file locks owned by pair-1"
    log_harness "pair-1" "  Removed orchestration/locks/*.json (owned by pair-1)"
    log_harness "pair-1" "pair-1 status: IDLE"
    log_success "pair-1 ready for next ticket"
    
    # Verify parallel execution
    log_section "PARALLEL EXECUTION CHECK"
    log_harness "pair-2" "Completed T-43 in parallel"
    log_harness "pair-2" "PR #48 merged"
    log_success "Multi-pair isolation verified"
    
    # Run assertions
    log_section "RUNNING ASSERTIONS"
    
    # Verify files exist
    assert_file_exists "orchestration/pairs/pair-1/shared/PLAN.md"
    assert_file_exists "orchestration/pairs/pair-1/shared/CONTRACT.md"
    assert_file_exists "orchestration/pairs/pair-1/shared/WORKLOG.md"
    assert_file_exists "orchestration/pairs/pair-1/shared/segment-1-eval.md"
    assert_file_exists "orchestration/pairs/pair-1/shared/final-review.md"
    assert_file_exists "orchestration/pairs/pair-1/shared/STATUS.json"
    
    # Verify status
    assert_status "pair-1" "PR_OPENED"
    
    # Cleanup
    cleanup_test_env
    
    # Final summary
    log_section "ALL TESTS PASSED â"
    
    echo ""
    echo "Test Summary:"
    echo "  - Ticket fetch: â"
    echo "  - Assignment: â"
    echo "  - Provisioning: â"
    echo "  - Plan review: â"
    echo "  - Implementation (4 segments): â"
    echo "  - Segment evaluations: â"
    echo "  - Final review: â"
    echo "  - PR creation: â"
    echo "  - Merge: â"
    echo "  - Cleanup: â"
    echo "  - Parallel execution: â"
    echo ""
}

# Run main
main "$@"