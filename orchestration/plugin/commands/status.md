# /status Command

Signal terminal status to the harness. Use when work is complete or blocked.

## Usage

```
/status <status> [reason]
```

## Status Values

- `PR_OPENED` - Work complete, PR created
- `BLOCKED` - Cannot proceed, needs intervention
- `FUEL_EXHAUSTED` - Budget/tokens exhausted, need more allocation

## What it does

1. Writes STATUS.json with current state
2. Lists all files changed
3. Provides reason/explanation
4. Harness reads STATUS.json and takes appropriate action

## STATUS.json Structure

```json
{
  "status": "PR_OPENED | BLOCKED | FUEL_EXHAUSTED",
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
```
/status PR_OPENED
```
Then provide the PR URL when prompted.

### Blocked
```
/status BLOCKED Cannot proceed due to API rate limit
```

### Fuel Exhausted
```
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