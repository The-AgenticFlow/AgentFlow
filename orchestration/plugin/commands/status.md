---
name: status
description: Signal terminal status to the harness
---

# /status Command

Signal terminal status to the harness. Use when work is complete or blocked.

## Usage

```bash
/status <status> [reason]
```

## Status Values

### Terminal statuses (ends the pair lifecycle)
- `PR_OPENED` - Work complete, PR created
- `COMPLETE` - All work done, PR creation deferred to harness
- `BLOCKED` - Cannot proceed, needs intervention
- `FUEL_EXHAUSTED` - Budget/tokens exhausted, need more allocation

### Non-terminal statuses (continues the event loop)
- `PENDING_REVIEW` - Work paused, waiting for review
- `AWAITING_SENTINEL_REVIEW` - Segment done, waiting for SENTINEL evaluation
- `APPROVED_READY` - Changes requested by SENTINEL have been addressed
- `SEGMENT_N_DONE` - Segment N complete (e.g. `SEGMENT_1_DONE`)

### IMPORTANT
Do NOT use any other status value. Values like `AWAITING_REVIEW`, `DONE`, `FINISHED`, `SUCCESS`, `IMPLEMENTATION_COMPLETE` will be treated as `BLOCKED` and your work will be wasted.

## What it does

1. Writes STATUS.json with current state
2. Lists all files changed
3. Provides reason/explanation
4. Harness reads STATUS.json and takes appropriate action

## STATUS.json Structure

```json
{
  "status": "PR_OPENED | COMPLETE | BLOCKED | FUEL_EXHAUSTED | PENDING_REVIEW | AWAITING_SENTINEL_REVIEW | APPROVED_READY | SEGMENT_N_DONE",
  "pair": "pair-{N}",
  "ticket_id": "T-{id}",
  "branch": "forge-{N}/T-{id}",
  "files_changed": [
    "src/auth.rs",
    "tests/auth_test.rs"
  ],
  "segments_completed": 3,
  "pr_url": "https://github.com/owner/repo/pull/42",
  "reason": "Optional reason for BLOCKED or FUEL_EXHAUSTED",
  "timestamp": "2025-03-24T10:00:00Z"
}
```

## Examples

### Work Complete

```bash
/status PR_OPENED
```

Then provide the PR URL when prompted.

### Blocked

```bash
/status BLOCKED Cannot proceed due to API rate limit
```

### Fuel Exhausted

```bash
/status FUEL_EXHAUSTED Need 50k more tokens to complete
```

## After STATUS.json

The harness will:

- **PR_OPENED**: Notify VESSEL to check CI and merge
- **BLOCKED**: Alert NEXUS for human intervention
- **FUEL_EXHAUSTED**: Request more budget allocation

## Important

- This is a terminal state - you cannot continue after writing STATUS.json
- For temporary pauses, use `/segment-done` instead
- For context reset, use `/handoff` instead
- Always list ALL files changed across all segments
