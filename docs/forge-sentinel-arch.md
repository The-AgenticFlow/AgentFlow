# FORGE-SENTINEL Pair Architecture

## Design Document v2.0

**Status:** Draft  
**Changes from v1:**
- Multi-pair isolation via Git worktrees
- FORGE works directly from project directory on own branch
- Sprintless plugin: MCP tools, skills, hooks, commands
- External merge gate reviewer removed — pair SENTINEL is sufficient
- VESSEL owns the final CI gate and merge

---

## Table of Contents

1. [Core Principles](#1-core-principles)
2. [Multi-Pair Isolation Model](#2-multi-pair-isolation-model)
3. [Git Worktree Architecture](#3-git-worktree-architecture)
4. [Working Directory Layout](#4-working-directory-layout)
5. [The Sprintless Plugin](#5-the-sprintless-plugin)
6. [MCP Tools](#6-mcp-tools)
7. [Skills](#7-skills)
8. [Hooks](#8-hooks)
9. [Slash Commands](#9-slash-commands)
10. [Artifact Schema](#10-artifact-schema)
11. [The FORGE Workflow](#11-the-forge-workflow)
12. [The SENTINEL Workflow](#12-the-sentinel-workflow)
13. [Sprint Contract Protocol](#13-sprint-contract-protocol)
14. [Context Reset Mechanism](#14-context-reset-mechanism)
15. [Done Contract and PR Flow](#15-done-contract-and-pr-flow)
16. [Why No External Reviewer](#16-why-no-external-reviewer)
17. [Failure Modes and Recovery](#17-failure-modes-and-recovery)
18. [Rust Implementation Structure](#18-rust-implementation-structure)

---

## 1. Core Principles
**Independence** — each pair is a completely isolated process. No pair reads from or writes to another pair's directories under any circumstances.

**Branch ownership** — each pair owns exactly one Git branch. Two pairs never touch the same branch. File locking is handled dynamically at runtime to prevent collisions even when scope expands.

**Event-driven communication** — The Rust harness monitors file system events (inotify/FSEvents) to trigger state transitions instantly. No polling delays.

**Ephemeral Evaluation** — SENTINEL is not a long-running process. It is spawned fresh for every evaluation, ensuring zero context drift and 100% focus on the specific segment.

**Plugin-first capability** — FORGE's autonomous capability comes from the Sprintless plugin: MCP tools for external operations, skills for knowledge, hooks for enforcement, commands for structured procedures.

VESSEL owns the merge — after the pair opens a PR, VESSEL checks CI and merges. This is a mechanical gate, not an agent review.
---

## 2. Multi-Pair Isolation Model

### The solution: three-layer isolation

```
Layer 1: Git worktrees     — each pair on its own branch, own checkout
Layer 2: File ownership    — NEXUS locks files before assignment
Layer 3: Slot directories  — all artifacts scoped to pair-N/
```

### Layer 1 — Git worktrees

Each pair slot has its own Git worktree — a complete checkout of the repo
on its own branch. The worktrees share the same `.git` object store
(efficient) but have completely independent working trees (isolated).

```
/project/
  .git/                        ← shared object store
  main/                        ← main branch checkout (source of truth)
  worktrees/
    pair-1/                    ← forge-1/T-42 branch
    pair-2/                    ← forge-2/T-47 branch
    pair-3/                    ← forge-3/T-51 branch
    pair-N/                    ← idle
```

FORGE works inside `worktrees/pair-N/`. It reads existing code, writes
new code, runs tests — all within its own worktree. It never touches
`worktrees/pair-2/`. Git enforces this at the filesystem level.

### Layer 2 — Dynamic File Ownership

NEXUS locks files identified in the ticket's `touched_files[]` before assignment. However, development often reveals new dependencies.

**The Dynamic Lock Protocol:**
1.  **Initial Lock:** NEXUS locks files explicitly listed in the ticket.
2.  **Discovery:** When FORGE attempts to write to a file *not* in its lock map, the `pre_write_check` hook intercepts.
3.  **Request:** The hook calls the `acquire_lock` MCP tool.
4.  **Resolution:**
    *   **Success:** The lock is granted (file is free). FORGE proceeds.
    *   **Failure:** The file is owned by another pair. The write is blocked. FORGE is notified exactly which pair owns the file and must either find an alternative implementation path or set status to `BLOCKED`.

This ensures safety even when the scope expands beyond the initial ticket estimate.
### Layer 3 — Slot-scoped artifacts

Every artifact is namespaced under `pair-N/`:

```
pair-1/shared/STATUS.json    ← pair-1's terminal output
pair-2/shared/STATUS.json    ← pair-2's terminal output (independent)
pair-3/shared/WORKLOG.md     ← pair-3's progress (independent)
```

NEXUS reads from `pair-N/shared/STATUS.json` by looking up which pair
owns ticket T-{id} from the sprint board in Redis. No ambiguity possible.

---

## 3. Git Worktree Architecture

### Provisioning a worktree

When NEXUS assigns ticket T-{id} to pair-N, the pair harness:

```bash
# 1. Ensure main is current
git -C /project/main fetch origin main
git -C /project/main merge origin/main

# 2. Create worktree for this pair on a new branch
git -C /project/main worktree add \
  /project/worktrees/pair-N \
  -b forge-N/T-{id}

# 3. Confirm branch is clean
git -C /project/worktrees/pair-N status
```

FORGE's working directory is `/project/worktrees/pair-N/`.
It runs all commands from there. It reads the full codebase there.
It writes its changes there.

### FORGE's Git operations inside the worktree

FORGE uses Git directly inside its worktree:

```bash
# FORGE works normally — read, write, test
# All within /project/worktrees/pair-N/

# After each segment, FORGE commits
git -C /project/worktrees/pair-N add -A
git -C /project/worktrees/pair-N commit -m "[T-{id}] segment-N: {description}"

# When done contract is fulfilled
git -C /project/worktrees/pair-N push origin forge-N/T-{id}
# Then opens PR via MCP tool: /mcp create_pr
```

### Cleaning up after merge

When VESSEL merges the PR:

```bash
# Harness removes the worktree
git -C /project/main worktree remove /project/worktrees/pair-N

# Harness recreates a clean idle worktree for the next ticket
git -C /project/main worktree add /project/worktrees/pair-N main
```

The slot is now idle and ready for the next assignment.

### Divergence protection

When a pair's task takes long enough that main has advanced significantly,
the harness checks for divergence before the PR push:

```bash
BEHIND=$(git -C /project/worktrees/pair-N \
  rev-list --count HEAD..origin/main)

if [ "$BEHIND" -gt 10 ]; then
  git -C /project/worktrees/pair-N rebase origin/main
  # Re-run tests after rebase
fi
```

If the rebase produces conflicts NEXUS cannot resolve automatically,
the ticket is flagged as BLOCKED with reason REBASE_CONFLICT.

---

## 4. Working Directory Layout

```
/project/
│
├── .git/                           ← shared Git object store
│
├── main/                           ← main branch (read-only reference)
│   └── src/ tests/ docs/ ...
│
├── worktrees/
│   ├── pair-1/                     ← FORGE-1's working tree (branch: forge-1/T-42)
│   │   └── src/ tests/ ...         ← full project, FORGE writes here
│   ├── pair-2/                     ← FORGE-2's working tree (branch: forge-2/T-47)
│   └── pair-N/                     ← idle (on main branch)
│
└── .sprintless/
    ├── pairs/
    │   ├── pair-1/
    │   │   └── shared/             ← FORGE-1 ↔ SENTINEL-1 communication
    │   │       ├── TICKET.md
    │   │       ├── TASK.md
    │   │       ├── PLAN.md
    │   │       ├── CONTRACT.md
    │   │       ├── WORKLOG.md
    │   │       ├── HANDOFF.md      ← written at context reset
    │   │       ├── segment-1-eval.md
    │   │       ├── segment-N-eval.md
    │   │       ├── final-review.md
    │   │       └── STATUS.json     ← terminal signal
    │   └── pair-2/
    │       └── shared/ ...
    │
    └── plugin/                     ← Sprintless plugin (loaded by all agents)
        ├── mcp/
        ├── skills/
        ├── hooks/
        └── commands/
```

**Key separation:**
- `worktrees/pair-N/` — the codebase FORGE works in (Git-managed)
- `.sprintless/pairs/pair-N/shared/` — the communication channel (not in Git)

The `shared/` directory is explicitly in `.gitignore`. It is runtime
state, not project state. LORE reads from it for documentation but
it is never committed.

---

## 5. The Sprintless Plugin

The Sprintless plugin is a Claude Code plugin that ships with the
Sprintless system. It is loaded into every FORGE and SENTINEL instance
at session start via the `.claude/` directory in the worktree.

### What a Claude Code plugin provides

Claude Code's plugin system allows a directory to define:
- **MCP servers** — external tools callable by the agent
- **Skills** — markdown knowledge documents injected into context
- **Hooks** — shell scripts that run at lifecycle events
- **Commands** — slash commands callable by the agent procedurally

### Plugin directory structure

```
.sprintless/plugin/
│
├── plugin.json                     ← plugin manifest
│
├── mcp/
│  ── mcp.json ← Configuration connecting to existing servers:
│   - github-mcp-server (official)
│   - redis-mcp-server (for locks/state)
│   - filesystem-mcp-server (for scoped writes)
│   - shell-mcp-server (for tests/lint)
│
│
├── skills/
│   ├── forge-coding.md
│   ├── forge-planning.md
│   ├── sentinel-review.md
│   └── sentinel-criteria.md
│
├── hooks/
│   ├── forge/
│   │   ├── session_start.sh
│   │   ├── post_write_lint.sh
│   │   ├── pre_bash_guard.sh
│   │   ├── pre_compact_handoff.sh
│   │   └── stop_require_artifact.sh
│   └── sentinel/
│       ├── session_start.sh
│       ├── post_write_validate.sh
│       ├── pre_bash_readonly_guard.sh
│       └── stop_require_eval.sh
│
└── commands/
    ├── plan.md                     ← /plan command definition
    ├── segment-done.md             ← /segment-done command definition
    ├── handoff.md                  ← /handoff command definition
    └── status.md                   ← /status command definition
```

### plugin.json

```json
{
  "name": "sprintless",
  "version": "1.0.0",
  "description": "Autonomous agent pair tooling for Sprintless",
  "mcp": {
    "server": "mcp/server.toml"
  },
  "skills": {
    "forge": ["skills/forge-coding.md", "skills/forge-planning.md"],
    "sentinel": ["skills/sentinel-review.md", "skills/sentinel-criteria.md"]
  },
  "hooks": {
    "forge": "hooks/forge/",
    "sentinel": "hooks/sentinel/"
  },
  "commands": "commands/"
}
```

### How NEXUS injects the plugin

When NEXUS provisions a pair slot, it symlinks the plugin into the
Claude Code settings for that slot:

```bash
# For FORGE in this worktree
ln -s /project/.sprintless/plugin \
      /project/worktrees/pair-N/.claude/plugins/sprintless

# For SENTINEL (sentinel has its own .claude dir in shared/)
ln -s /project/.sprintless/plugin \
      /project/.sprintless/pairs/pair-N/sentinel/.claude/plugins/sprintless
```

Claude Code loads the plugin on session start. MCP tools, skills,
hooks, and commands are all immediately available.

---

## 6. MCP Tools

We map high-level agent needs to specific existing MCP tools. Enforcement is handled by restricting the agent's permissions (e.g., denying direct Bash execution) so it *must* use the MCP tools.

### Tool Mapping & Enforcement

| Agent Need | MCP Server | Tool Name | Enforcement Rule |
| :--- | :--- | :--- | :--- |
| **Create PR** | `github` | `create_pull_request` | Hook blocks `git push` CLI command. |
| **Lock File** | `redis` | `set` (with NX) | Hook blocks `Write` tool if lock not acquired. |
| **Write Code** | `filesystem` | `write_file` | Agent restricted to filesystem MCP for writes. |
| **Run Tests** | `shell` | `run_command` | Hook blocks direct `npm test`. Must use tool. |

### Specific Usage Rules

**1. Dynamic Locking (via Redis MCP)**
*   **Action:** Before writing to a new file, the agent calls `redis.set` with key `lock:{filepath}` and value `pair-ID`.
*   **Condition:** Use `NX` (Only set if not exists).
*   **Result:** If returns `1` (Success), proceed. If `null` (Exists), the file is locked.

**2. GitHub Operations (via GitHub MCP)**
*   **Action:** Creating branches, pushing commits, opening PRs.
*   **Constraint:** The agent has no `git` credentials in the CLI environment. It *must* use the `github` MCP server which holds the scoped token.

**3. Testing & Linting (via Shell MCP)**
*   **Action:** Running `.agent/tooling/run-tests.sh`.
*   **Constraint:** The `shell` MCP server is configured with an allow-list. The agent can only run commands explicitly defined in the tooling directory, preventing arbitrary code execution.
```

### MCP server implementation

The MCP server is a Rust binary in `crates/plugin-mcp/`. It reads its
configuration from environment variables injected by the pair harness:

```
SPRINTLESS_PAIR_ID=pair-1
SPRINTLESS_TICKET_ID=T-42
SPRINTLESS_WORKTREE=/project/worktrees/pair-1
SPRINTLESS_SHARED=/project/.sprintless/pairs/pair-1/shared
SPRINTLESS_GITHUB_TOKEN=...
SPRINTLESS_REDIS_URL=redis://localhost:6379
```

Every tool call is automatically scoped to the correct pair and worktree.
FORGE cannot accidentally call `commit_segment` and commit to pair-2's
worktree — the MCP server enforces the scope from environment config.

---

## 7. Skills

Skills are markdown files injected into the Claude Code context at
session start. They are not in the conversation history — they are
loaded as reference documents. FORGE and SENTINEL each receive
the skills relevant to their role.

### `skills/forge-coding.md`

```markdown
# FORGE coding skill

## Your role
You are FORGE, the generator in a FORGE-SENTINEL pair.
Your job is to produce correct, complete, well-tested implementations.
You work in segments. After each segment you submit to SENTINEL.
Quality comes from the pair loop, not from you alone.

## Before writing any code
1. Read TICKET.md — understand what you are building
2. Read CONTRACT.md — this is your definition of done
3. Search the codebase — find existing patterns before inventing new ones
   Tool: search_codebase

## Coding standards
- All standards are in .agent/standards/CODING.md
- All architecture patterns are in .agent/arch/patterns.md
- API contracts are in .agent/arch/api-contracts.md
- READ these before implementing. They are not optional.

## Testing discipline
- Every new function needs a test
- Every changed file needs updated tests
- Run tests after every segment: Tool: run_tests
- Do not submit a segment with failing tests

## Error handling
- Never throw raw Error — use AppError from src/errors/
- Every async function must have explicit error handling
- Network calls must have timeout and retry

## Submitting a segment
When you believe a segment is complete:
1. Run tests — all must pass
2. Run linter — zero warnings
3. Use /segment-done command
```

### `skills/forge-planning.md`

```markdown
# FORGE planning skill

## Writing PLAN.md
Before any implementation, write a plan.
Use the /plan command to structure it correctly.

## What a good plan contains
- Your understanding of the ticket in your own words
- Technical approach that follows .agent/arch/patterns.md
- Explicit segment breakdown — each segment is independently testable
- Definition of done per segment — specific and verifiable
- List of files you will create or modify
- Risk areas — things you are uncertain about
- Questions for SENTINEL — clarifications needed before starting

## Segment sizing
A good segment:
- Touches 1-3 files
- Has a single clear purpose
- Can be tested in isolation
- Takes roughly 20-40 minutes to implement

A segment that is too large:
- Touches more than 5 files
- Has multiple unrelated concerns
- Cannot be independently verified
Split it.

## Contract negotiation
SENTINEL will review your plan.
If SENTINEL objects, read the objection carefully.
Update PLAN.md addressing each specific objection.
Do not argue — either accept the feedback or ask a clarifying question.
```

### `skills/sentinel-review.md`

```markdown
# SENTINEL review skill

## Your role
You are SENTINEL. You are spawned for a single purpose: evaluate one segment.You have no history. You have no future. You only have this segment.

## Your disposition
Be skeptical. Be specific. Be constructive.
FORGE is your partner, not your adversary.
Your feedback must be actionable — FORGE must know exactly what to fix.

## Reviewing a plan (PLAN.md)
Check:
1. Does the plan address all acceptance criteria in TICKET.md?
2. Does the technical approach follow .agent/arch/patterns.md?
3. Are all relevant files identified?
4. Is the definition of done specific and testable?
5. Is there an explicit out-of-scope list?

## Reviewing a segment
Check:
1. Run tests — Tool: run_tests — they must all pass
2. Run linter — Tool: run_linter — zero warnings
3. Read every changed file against the CONTRACT criteria
4. Check error handling — every error path covered?
5. Check test coverage — is every new function tested?
6. Check standards compliance — CODING.md and patterns.md respected?

## Writing feedback
When writing segment-N-eval.md with CHANGES_REQUESTED:
- Every item must have: file, line number, problem, required fix
- Do not write vague feedback like "improve error handling"
- Write: "src/auth/session.ts line 47: throws raw Error. 
  Required: throw new AppError('SESSION_EXPIRED', 401) per CODING.md rule 3"

## Final review
When all segments are approved, run the complete verification:
1. Full test suite via run_tests
2. Full linter via run_linter
3. Check every CONTRACT criterion is satisfied
4. Write final-review.md with APPROVED verdict and PR description
5. Your PR description becomes the actual PR body — make it informative
```

### `skills/sentinel-criteria.md`

```markdown
# SENTINEL evaluation criteria

## The five criteria — all must pass for any approval

### 1. Correctness
Does the implementation correctly handle all cases described in CONTRACT.md?
Does it handle the error paths, edge cases, and boundary conditions?
FAIL: any CONTRACT criterion not met, any obvious logic error

### 2. Test coverage
Are all changed files covered by tests?
Does every new function have at least one test for the happy path
and one for the primary error path?
FAIL: any changed file with no tests, any new function with no test

### 3. Standards compliance
Does the implementation follow .agent/standards/CODING.md?
Does it use the patterns in .agent/arch/patterns.md?
Does it respect the API contracts in .agent/arch/api-contracts.md?
FAIL: any violation of the team's written standards

### 4. Code quality
Is the code readable? Are names clear? Is complexity justified?
Is there duplication that should be extracted?
NOTE: This criterion is advisory — it cannot block alone.
It informs feedback but a single quality concern is not a blocker.

### 5. No regressions
Do all existing tests still pass?
Has any existing behaviour been changed without explicit ticket scope?
FAIL: any previously passing test now failing
```

---

## 8. Hooks

Hooks are shell scripts that run at Claude Code lifecycle events.
They enforce invariants that cannot be left to the agent's judgment.

### FORGE hooks

**`hooks/forge/session_start.sh`**

```bash
#!/bin/bash
# Runs when FORGE starts a new session

PAIR_ID="${SPRINTLESS_PAIR_ID}"
TICKET_ID="${SPRINTLESS_TICKET_ID}"
SHARED="${SPRINTLESS_SHARED}"

# Log session start to event stream
redis-cli -u "${SPRINTLESS_REDIS_URL}" XADD sprintless:events '*' \
  agent "forge-${PAIR_ID}" \
  type "session_start" \
  ticket "${TICKET_ID}" \
  ts "$(date -u +%Y-%m-%dT%H:%M:%SZ)"

# Check if this is a resume (HANDOFF.md exists)
if [ -f "${SHARED}/HANDOFF.md" ]; then
  echo "RESUME MODE: HANDOFF.md found."
  echo "Read ${SHARED}/HANDOFF.md before doing anything else."
  echo "Continue from the exact next step described in the handoff."
else
  echo "NEW SESSION: No handoff found."
  echo "Read ${SHARED}/TICKET.md and ${SHARED}/TASK.md to begin."
fi
```

**`hooks/forge/post_write_lint.sh`**

```bash
#!/bin/bash
# Runs after every Write, Edit, MultiEdit tool call
# $CLAUDE_TOOL_INPUT_FILE_PATH set by hook runtime

FILE="${CLAUDE_TOOL_INPUT_FILE_PATH}"
WORKTREE="${SPRINTLESS_WORKTREE}"

# Only lint source files
case "$FILE" in
  *.ts|*.tsx)
    OUTPUT=$(cd "$WORKTREE" && npx eslint "$FILE" 2>&1)
    if [ $? -ne 0 ]; then
      echo "Lint failed on ${FILE}:"
      echo "$OUTPUT"
      echo ""
      echo "Fix these lint errors before continuing."
      exit 2
    fi
    ;;
  *.rs)
    OUTPUT=$(cd "$WORKTREE" && cargo clippy --quiet 2>&1)
    if [ $? -ne 0 ]; then
      echo "Clippy failed:"
      echo "$OUTPUT"
      echo ""
      echo "Fix these warnings before continuing."
      exit 2
    fi
    ;;
  *.py)
    OUTPUT=$(cd "$WORKTREE" && ruff check "$FILE" 2>&1)
    if [ $? -ne 0 ]; then
      echo "Ruff failed on ${FILE}:"
      echo "$OUTPUT"
      exit 2
    fi
    ;;
esac

exit 0
```

**`hooks/forge/pre_bash_guard.sh`**

```bash
#!/bin/bash
# Runs before every Bash tool call
# $CLAUDE_TOOL_INPUT_COMMAND set by hook runtime

CMD="${CLAUDE_TOOL_INPUT_COMMAND}"

# Block direct git push — must use create_pr MCP tool
if echo "$CMD" | grep -qE "^git push|git push "; then
  echo "BLOCKED: Direct git push not allowed."
  echo "When ready to push, use the /status command which"
  echo "will verify the done contract and push via MCP tool."
  exit 2
fi

# Block writes to other pairs' worktrees
if echo "$CMD" | grep -qE "worktrees/pair-[0-9]+/" ; then
  REFERENCED=$(echo "$CMD" | grep -oE "pair-[0-9]+" | head -1)
  if [ "$REFERENCED" != "${SPRINTLESS_PAIR_ID}" ]; then
    echo "BLOCKED: Cannot access ${REFERENCED}'s worktree."
    echo "You are pair ${SPRINTLESS_PAIR_ID}."
    exit 2
  fi
fi

# Block writes to main branch
if echo "$CMD" | grep -qE "checkout main|checkout origin/main"; then
  echo "BLOCKED: Cannot checkout main. Work on your branch only."
  exit 2
fi

exit 0
```

**`hooks/forge/pre_compact_handoff.sh`**

```bash
#!/bin/bash
# Runs on PreCompact — converts compaction to clean context reset

SHARED="${SPRINTLESS_SHARED}"

echo "CONTEXT RESET REQUIRED

Your context window is approaching its limit.
Before this session ends, you must write a handoff.

Use the /handoff command now. It will:
1. Collect everything needed for the handoff
2. Write ${SHARED}/HANDOFF.md
3. Update WORKLOG.md with current state
4. Exit cleanly

A fresh FORGE session will read your handoff and continue.
Do not attempt to continue working — write the handoff now."

exit 2
```

**`hooks/forge/stop_require_artifact.sh`**

```bash
#!/bin/bash
# Runs on Stop — FORGE cannot exit without a terminal artifact

SHARED="${SPRINTLESS_SHARED}"

# Accept: STATUS.json written (done or blocked)
if [ -f "${SHARED}/STATUS.json" ]; then
  # Validate it has required fields
  VALID=$(python3 -c "
import json, sys
try:
    s = json.load(open('${SHARED}/STATUS.json'))
    required = ['status','pair','ticket_id','files_changed']
    missing = [k for k in required if k not in s]
    valid_statuses = ['PR_OPENED','BLOCKED','FUEL_EXHAUSTED']
    if missing:
        print(f'missing: {missing}')
        sys.exit(1)
    if s['status'] not in valid_statuses:
        print(f'invalid status: {s[\"status\"]}')
        sys.exit(1)
except Exception as e:
    print(str(e))
    sys.exit(1)
  " 2>&1)
  if [ $? -ne 0 ]; then
    echo "STATUS.json exists but is invalid: ${VALID}"
    echo "Fix STATUS.json before exiting."
    exit 2
  fi
  exit 0
fi

# Accept: HANDOFF.md written (context reset in progress)
if [ -f "${SHARED}/HANDOFF.md" ]; then
  # Verify HANDOFF.md has required sections
  if grep -q "## Exact next step" "${SHARED}/HANDOFF.md"; then
    exit 0
  else
    echo "HANDOFF.md is incomplete. It must contain '## Exact next step'."
    exit 2
  fi
fi

# Neither exists — block exit
echo "BLOCKED: Cannot exit without writing either:
  - ${SHARED}/STATUS.json  (if done or blocked)
  - ${SHARED}/HANDOFF.md   (if context reset)

Use /status command if done.
Use /handoff command if you need a context reset."
exit 2
```

### SENTINEL hooks

**`hooks/sentinel/post_write_validate.sh`**

```bash
#!/bin/bash
# Validates eval files on write

FILE="${CLAUDE_TOOL_INPUT_FILE_PATH}"

# Only validate evaluation artifacts
case "$FILE" in
  *segment-*-eval.md)
    # Must have Verdict section
    if ! grep -q "^## Verdict" "$FILE"; then
      echo "INVALID: segment eval must have '## Verdict' section."
      exit 2
    fi
    # CHANGES_REQUESTED must have specific feedback
    if grep -q "CHANGES_REQUESTED" "$FILE"; then
      if ! grep -q "^## Specific feedback" "$FILE"; then
        echo "INVALID: CHANGES_REQUESTED requires '## Specific feedback'"
        echo "with file:line:problem:fix for each item."
        exit 2
      fi
    fi
    ;;
  *final-review.md)
    if ! grep -q "^## Verdict" "$FILE"; then
      echo "INVALID: final-review must have '## Verdict' section."
      exit 2
    fi
    if grep -q "APPROVED" "$FILE"; then
      if ! grep -q "^## PR description" "$FILE"; then
        echo "INVALID: APPROVED final-review must include PR description."
        exit 2
      fi
    fi
    ;;
esac

exit 0
```

**`hooks/sentinel/pre_bash_readonly_guard.sh`**

```bash
#!/bin/bash
CMD="${CLAUDE_TOOL_INPUT_COMMAND}"
WORKTREE="${SPRINTLESS_WORKTREE}"

# SENTINEL can read and run tests but cannot modify source files
WRITE_PATTERNS="sed -i|awk.*>|tee |> [^/]|echo.*>.*src|echo.*>.*tests"
if echo "$CMD" | grep -qE "$WRITE_PATTERNS"; then
  echo "BLOCKED: SENTINEL cannot modify source files."
  echo "Write your evaluation to ${SPRINTLESS_SHARED}/segment-*-eval.md"
  exit 2
fi

exit 0
```

**`hooks/sentinel/stop_require_eval.sh`**

```bash
#!/bin/bash
SHARED="${SPRINTLESS_SHARED}"

EVALS=$(ls "${SHARED}"/segment-*-eval.md 2>/dev/null | wc -l)
FINAL=$(ls "${SHARED}/final-review.md" 2>/dev/null | wc -l)

if [ "$EVALS" -eq 0 ] && [ "$FINAL" -eq 0 ]; then
  echo "BLOCKED: SENTINEL must write an evaluation before exiting.
Either segment-N-eval.md or final-review.md must exist."
  exit 2
fi

exit 0
```
crates/
pocketflow-core/ Node, Flow, SharedStore, BatchNode, Action

pair-harness/ Owns the pair lifecycle
src/
pair.rs ForgeSentinelPair struct
worktree.rs Git worktree create/remove
process.rs Spawns FORGE process (SENTINEL is now spawned by Harness directly)
watcher.rs inotify logic
mcp_config.rs Generates mcp.json env vars for the plugin
agent-forge/ ForgeNode implementation
agent-sentinel/ SentinelNode implementation
agent-nexus/ NexusNode implementation
---

## 9. Slash Commands

Slash commands are structured procedures callable by the agent.
They act as high-level operations that coordinate multiple steps.

### `/plan`

```markdown
# /plan command

Generates a structured PLAN.md for the current ticket.

## When to use
After reading TICKET.md and TASK.md, before writing any code.

## Steps this command performs
1. Read TICKET.md acceptance criteria
2. Read TASK.md constitution references
3. Search codebase for relevant existing code (search_codebase tool)
4. Draft PLAN.md with full segment breakdown
5. Write PLAN.md to shared/ via write_to_shared tool
6. Notify: "Plan written. Waiting for SENTINEL review."

## Output
Creates shared/PLAN.md
```

### `/segment-done`

```markdown
# /segment-done command

Submits the current segment to SENTINEL for evaluation.

## When to use
When you believe a segment is complete and ready for review.

## Steps this command performs
1. Run full test suite (run_tests tool) — must pass before continuing
2. Run linter on changed files (run_linter tool) — must be clean
3. Commit the segment (commit_segment tool)
4. Append to WORKLOG.md (write_to_shared tool)
5. Emit event: "Segment N submitted for evaluation" (emit_event tool)
6. Wait state: "Submitted. Waiting for SENTINEL segment-N-eval.md"

## Blocked if
- Any tests are failing
- Any lint warnings exist
- No files have been changed since last commit

## Output
Updates WORKLOG.md, commits segment, notifies SENTINEL
```

### `/handoff`

```markdown
# /handoff command

Writes a complete handoff and exits for context reset.

## When to use
When the PreCompact hook fires, or when you choose to reset
context at a natural segment boundary.

## Steps this command performs
1. Read WORKLOG.md to summarise completed segments
2. Scan worktree for in-progress files
3. Collect all decisions made (from WORKLOG.md decisions sections)
4. Write complete HANDOFF.md to shared/ (write_to_shared tool)
5. Emit event: "Context reset — handoff written" (emit_event tool)
6. Exit cleanly

## Output
Creates shared/HANDOFF.md
Agent exits — harness spawns fresh session
```

### `/status`

```markdown
# /status command

Writes the terminal STATUS.json after done contract is fulfilled.

## When to use
ONLY after:
- final-review.md exists with APPROVED verdict
- All segments are complete and approved

## Steps this command performs
1. Verify final-review.md exists and verdict is APPROVED
2. Verify all tests pass (run_tests tool)
3. Push branch to GitHub origin (git push via bash)
4. Open PR using SENTINEL's PR description (create_pr tool)
5. Write STATUS.json to shared/ (write_to_shared tool)
6. Emit event: "PR opened — T-{id} complete" (emit_event tool)
7. Exit cleanly

## Blocked if
- final-review.md does not exist
- final-review.md verdict is not APPROVED
- Tests are failing

## Output
PR opened on GitHub
Creates shared/STATUS.json with status: PR_OPENED
Agent exits — NEXUS reads STATUS.json
```

---

## 10. Artifact Schema

### STATUS.json — terminal output

```json
{
  "status": "PR_OPENED | BLOCKED | FUEL_EXHAUSTED",
  "pair": "pair-1",
  "ticket_id": "T-42",
  "pr_url": "https://github.com/org/repo/pull/47",
  "pr_number": 47,
  "branch": "forge-1/T-42",
  "files_changed": ["src/auth/refresh.ts", "tests/auth.test.ts"],
  "test_results": { "passed": 89, "failed": 0, "skipped": 0 },
  "segments_completed": 3,
  "context_resets": 1,
  "sentinel_approved": true,
  "blockers": [],
  "elapsed_ms": 5420000,
  "timestamp": "2025-03-24T14:22:00Z"
}
```

If BLOCKED:

```json
{
  "status": "BLOCKED",
  "pair": "pair-1",
  "ticket_id": "T-42",
  "pr_url": null,
  "files_changed": [],
  "sentinel_approved": false,
  "blockers": [
    {
      "type": "AMBIGUOUS_REQUIREMENT",
      "description": "Ticket says 'support OAuth' but architecture docs have no OAuth pattern. Need decision: implement from scratch or wait for T-38?",
      "nexus_action": "Clarify OAuth approach or resolve dependency on T-38"
    }
  ],
  "timestamp": "2025-03-24T14:22:00Z"
}
```

### WORKLOG.md — append-only progress log

```markdown
# Worklog — T-42 — pair-1

## Segment 1 — JWT refresh endpoint — 2025-03-24T10:14:00Z
Status: COMPLETE — SENTINEL APPROVED
Commit: a3f9b12
Files:
  - src/auth/refresh.ts (new)
  - tests/auth/refresh.test.ts (new)
Tests: 12 passed
Decisions:
  - Used httpOnly cookie for refresh token (security standard)
  - 7-day refresh window (per ticket spec)

## Segment 2 — Session middleware — 2025-03-24T11:02:00Z
Status: COMPLETE — SENTINEL APPROVED (after 1 revision)
...
```

---

## 11. The FORGE Workflow

```
NEXUS writes TICKET.md + TASK.md to pair-N/shared/
Git worktree pair-N created on branch forge-N/T-{id}
FORGE session spawned — plugin loaded — hooks active
                │
                ▼
    Hook: session_start
    Check: HANDOFF.md exists?
         ┌────┴────┐
        Yes        No
         │          │
    RESUME       NEW SESSION
    Read         Read TICKET.md
    HANDOFF.md   + TASK.md
         │          │
         └────┬─────┘
              │
              ▼
    Run /plan command
    search_codebase → read standards → write PLAN.md
              │
    Wait: CONTRACT.md from SENTINEL
         ┌────┴────┐
      Issues    AGREED
         │          │
    Update        Begin
    PLAN.md     segment 1
         │
    (max 3 rounds,
     then BLOCKED)
              │
   ┌──────────▼──────────────────────────┐
   │         SEGMENT LOOP                │
   │                                     │
   │  Read CONTRACT.md + WORKLOG.md      │
   │  Implement segment in worktree      │
   │  Hook: post_write_lint (per file)   │
   │                                     │
   │  Run /segment-done                  │
   │  → run_tests (must pass)            │
   │  → run_linter (must be clean)       │
   │  → commit_segment                   │
   │  → append WORKLOG.md                │
   │                                     │
   │  Wait: segment-N-eval.md            │
   │     ┌───────┴──────┐                │
   │  APPROVED    CHANGES_REQUESTED      │
   │     │              │                │
   │  Next seg    Fix specific           │
   │              items                  │
   │              (targeted, not         │
   │               rebuild)             │
   │                                     │
   │  PreCompact hook fires?             │
   │  → Run /handoff → exit              │
   │    Fresh session reads HANDOFF.md   │
   └─────────────────────────────────────┘
              │
    All segments approved
              │
    Wait: final-review.md
         ┌────┴────┐
      REJECTED   APPROVED
         │           │
    Fix items    Run /status
                 → verify final-review
                 → git push branch
                 → create_pr tool
                 → write STATUS.json
                 Hook: stop_require_artifact
                 Exit
```

---

## 12. The SENTINEL Workflow

```
FORGE writes PLAN.md
        │
        ▼
SENTINEL session spawned — plugin loaded — hooks active
Hook: session_start
        │
        ▼
Read PLAN.md + TICKET.md + .agent/standards/
        │
   Plan review
   ┌────┴────┐
 Issues    Sound
   │           │
Write      Write CONTRACT.md
objections (status: AGREED)
to PLAN.md
   │
FORGE updates plan
   │
(max 3 rounds)
        │
┌───────▼───────────────────────────────┐
│          EVALUATION LOOP              │
│                                       │
│  Watch shared/ for WORKLOG update     │
│  (new segment = ready to evaluate)    │
│                                       │
│  Read WORKLOG.md latest entry         │
│  Read changed files in worktree       │
│  Run run_tests tool                   │
│  Run run_linter tool                  │
│  Grade against 5 criteria             │
│  Hook: post_write_validate            │
│  Write segment-N-eval.md             │
│                                       │
│  FORGE responds:                      │
│  ┌──────────┐  ┌──────────────────┐   │
│  │Next seg  │  │Fixes submitted   │   │
│  └────┬─────┘  └────────┬─────────┘   │
│       │                 │             │
│  Next eval         Re-evaluate        │
│                    same segment       │
│  (if >5 iterations: BLOCKED signal)  │
└───────────────────────────────────────┘
        │
All segments approved
        │
   Final review
   run_tests (full suite)
   run_linter (full project)
   Check every CONTRACT criterion
   Write final-review.md
        │
   ┌────┴────┐
 Issues   APPROVED
   │          │
Write      Include PR description
REJECTED   Authorise FORGE to push
verdict    Exit
   │
FORGE fixes
then resubmit
```

---

## 13. Sprint Contract Protocol

```
FORGE writes PLAN.md
      │
      ▼ (SENTINEL polls shared/ every 10s)
SENTINEL reads PLAN.md
      │
  ┌───┴───┐
Issues  Sound
  │         │
Write   Write CONTRACT.md
PLAN.md     status: AGREED
addendum    signed: FORGE+SENTINEL
  │
FORGE reads addendum
Addresses each objection
Rewrites PLAN.md
  │
SENTINEL re-reviews
  │
(repeat max 3 rounds)
  │
Round 4 reached:
Write STATUS.json
status: BLOCKED
reason: CONTRACT_DISAGREEMENT
```

**What SENTINEL checks in a plan:**

| Check | Pass condition |
|---|---|
| Scope alignment | Plan addresses all acceptance criteria, nothing more |
| Technical approach | Follows `.agent/arch/patterns.md` |
| File coverage | All files that need changing are listed |
| Test strategy | Verification is specific and runnable |
| Out of scope | Explicit list of what is NOT being built |
| Risk flags | Uncertain areas acknowledged |

---

## 14. Context Reset Mechanism

```
Context threshold approaching (70% of model limit)
          │
          ▼
PreCompact hook fires
Returns message to FORGE:
"Write /handoff now"
          │
FORGE runs /handoff command:
  1. Reads WORKLOG.md for completed state
  2. Scans worktree for in-progress files
  3. Collects all decisions made
  4. Writes HANDOFF.md
  5. Emits reset event to shared store
  6. Exits cleanly (exit 0)
          │
Rust harness detects:
  HANDOFF.md written + FORGE process exited
          │
Harness spawns fresh FORGE process
Same pair slot, same worktree, same plugin
          │
Fresh FORGE:
  Hook: session_start detects HANDOFF.md
  "RESUME MODE: Read HANDOFF.md"
          │
FORGE reads HANDOFF.md:
  - Which segments are complete
  - Which segment is in progress
  - Decisions already made (do not contradict)
  - Files already written (do not rewrite)
  - Exact next step
          │
Continues from exact next step
SENTINEL is unaffected — keeps evaluating normally
```

**Harness reset tracking:**

```rust
pub struct PairHarness {
    pub pair_id:        String,
    pub ticket_id:      String,
    pub reset_count:    u32,
    pub max_resets:     u32,    // default 10, configurable per ticket
    pub start_time:     Instant,
}

impl PairHarness {
    async fn on_forge_exit(&mut self) -> Result<ResetAction> {
        let shared = self.shared_dir();
        
        if shared.join("STATUS.json").exists() {
            return Ok(ResetAction::Done);
        }
        
        if shared.join("HANDOFF.md").exists() {
            if self.reset_count >= self.max_resets {
                return Ok(ResetAction::ExceededLimit);
            }
            self.reset_count += 1;
            return Ok(ResetAction::SpawnFresh);
        }
        
        // Unclean exit — synthesise handoff from WORKLOG
        self.synthesise_handoff().await?;
        self.reset_count += 1;
        Ok(ResetAction::SpawnFresh)
    }
}
```

---

## 15. Done Contract and PR Flow

### Done conditions — all must be true

```
✓  CONTRACT.md status: AGREED
✓  All PLAN.md segments have APPROVED segment-N-eval.md
✓  final-review.md verdict: APPROVED
✓  run_tests: 0 failed, 0 skipped
✓  run_linter: 0 warnings, 0 errors
✓  All CONTRACT acceptance criteria checked off
✓  reset_count < max_resets
```

### PR flow (executed by /status command)

```
1. FORGE verifies final-review.md — APPROVED
2. FORGE runs run_tests — confirms still passing
3. FORGE: git push origin forge-N/T-{id}
4. FORGE calls create_pr MCP tool:
   - title: "[T-{id}] {ticket title}"
   - body: SENTINEL's PR description from final-review.md
   - head: forge-N/T-{id}
   - base: main
5. FORGE writes STATUS.json with PR URL
6. Stop hook validates STATUS.json
7. FORGE exits
8. Harness reads STATUS.json
9. Harness emits PR_OPENED event to Redis
10. NEXUS reads event, updates sprint board
11. NEXUS routes to VESSEL
12. VESSEL checks CI, merges on green
```

---

## 16. Why No External Reviewer

The external merge gate reviewer was designed for a world where a
single SENTINEL instance reviewed only the final PR — seeing the
diff cold, without knowing how the work was built.

In the pair model, SENTINEL has:

- Reviewed the plan before any code was written
- Evaluated every segment with full test and lint verification
- Approved every segment before FORGE could proceed
- Run the complete final verification including security scan
- Drafted the PR description with full context

What does a second reviewer add at this point? Mechanically:
nothing that SENTINEL has not already done. An external reviewer
reading the PR diff cold has less context than SENTINEL which
lived the entire implementation.

The only remaining gate is mechanical: did CI pass, does the branch
have conflicts with main, is the PR format correct. These are not
judgment calls — they are checks a script can run.

**VESSEL owns the merge gate:**

```
PR opened by FORGE
      │
      ▼
GitHub webhook: pull_request.opened
      │
      ▼
VESSEL checks:
  1. CI status — all checks green?
  2. Branch conflicts — mergeable?
  3. PR format — title, body, linked ticket?
      │
  ┌───┴───┐
 Issues  Clean
  │         │
Notify    Merge PR
NEXUS     Delete branch
(CI fail, Emit MERGED event
 conflict) NEXUS → LORE → VESSEL deploy
```

This is deterministic and cheap. No LLM call. No review queue.
The quality guarantee was provided by the pair — VESSEL just
confirms the mechanical conditions are met and executes the merge.

---

## 17. Failure Modes and Recovery

### FORGE stalls — no WORKLOG update for N minutes

```
NEXUS watchdog fires
↓
NEXUS reads WORKLOG.md — last update > 20 minutes ago
↓
NEXUS sends Notification event to FORGE:
"No WORKLOG update in 20 minutes.
 If working: append progress note to WORKLOG.md
 If blocked: run /status with status=BLOCKED"
↓
If no response in 5 minutes:
  NEXUS marks pair as stalled
  Attempts reassignment to different pair slot
  Preserves WORKLOG.md and HANDOFF.md for continuity
```

### FORGE and SENTINEL loop on same segment (> 5 iterations)

```
Harness counts segment-N-eval.md files for segment N
If count > 5 with no APPROVED:
↓
Harness writes STATUS.json:
  status: BLOCKED
  reason: SEGMENT_LOOP
  details: "Segment N has 5+ evaluation cycles with no approval"
↓
NEXUS escalates to human via Slack:
  "pair-1 T-42 segment 3 cannot be agreed after 5 cycles.
   See .sprintless/pairs/pair-1/shared/segment-3-eval.md history."
```

### Contract disagreement (> 3 rounds)

SENTINEL writes STATUS.json with `BLOCKED / CONTRACT_DISAGREEMENT`.
NEXUS re-assigns with clarified ticket after human input.

### Rebase conflict after main advances

```
Harness detects divergence > 10 commits
↓
Harness runs: git rebase origin/main
↓
If conflicts:
  Write STATUS.json: BLOCKED / REBASE_CONFLICT
  NEXUS escalates to human
If clean:
  Re-run tests to verify
  Continue normally
```

### File lock conflict (NEXUS assignment error)

If by some race condition two pairs attempt to write the same file
(should be impossible with proper lock checking):

```
post_write_lint hook detects file lock violation via MCP:
get_file_owners returns different pair as owner
↓
FORGE stops immediately
Writes STATUS.json: BLOCKED / FILE_LOCK_CONFLICT
NEXUS resolves by completing the conflicting pair first
```

---

## 18. Rust Implementation Structure

```
crates/
  pocketflow-core/      Node, Flow, SharedStore, BatchNode, Action
  
  plugin-mcp/           Sprintless MCP server binary
    src/
      main.rs
      tools/
        github.rs       create_pr, get_issue
        test_runner.rs  run_tests
        linter.rs       run_linter
        search.rs       search_codebase
        shared_store.rs write_to_shared, read_from_shared, emit_event
        git.rs          commit_segment, get_file_owners
  
  pair-harness/         Owns the pair lifecycle
    src/
      pair.rs           ForgeSentinelPair struct + run()
      worktree.rs       Git worktree create/remove/rebase
      isolation.rs      File lock registry reads/writes
      process.rs        Spawn FORGE + SENTINEL processes
      reset.rs          Context reset detection + handoff synthesis
      signals.rs        STATUS.json watcher
      watchdog.rs       Stall detection + NEXUS notification
  
  agent-forge/
    src/
      node.rs           ForgeNode implementing PocketFlow Node
      pool.rs           ForgeWorkerPool implementing BatchNode
  
  agent-sentinel/
    src/
      node.rs           SentinelNode implementing PocketFlow Node
  
  agent-nexus/
    src/
      node.rs           NexusNode
      assignment.rs     Ticket assignment + file lock checking
      watchdog.rs       Timer monitoring + stall detection
  
  agent-vessel/
    src/
      node.rs           VesselNode — CI check + merge + deploy
  
  agent-lore/
    src/
      node.rs           LoreNode — ADR + changelog
  
  watcher/              Axum SSE watcher UI
  
  config/               registry.json + .agent.md parser
  
  github/               octocrab wrapper (used by NEXUS + VESSEL)
  
  binary/
    src/
      main.rs           Assembles Flow, starts runtime
```

### ForgeSentinelPair — core struct

```rust
pub struct ForgeSentinelPair {
    pub pair_id:       String,
    pub ticket_id:     String,
    pub worktree:      PathBuf,
    pub shared:        PathBuf,
    pub harness:       PairHarness,
    pub store:         SharedStore,
}

impl ForgeSentinelPair {
    pub async fn run(&self, ticket: &Ticket) -> Result<PairOutcome> {
        // 1. Provision worktree
        self.create_worktree(ticket).await?;
        
        // 2. Write TICKET.md and TASK.md to shared/
        self.write_task_context(ticket).await?;
        
        // 3. Spawn FORGE and SENTINEL processes
        let forge    = self.spawn_forge().await?;
        let sentinel = self.spawn_sentinel().await?;
        
        // 4. Watch until terminal signal
        loop {
            // Check STATUS.json
            if let Some(status) = self.read_status().await? {
                self.cleanup_processes(&forge, &sentinel).await;
                return Ok(PairOutcome::from(status));
            }
            
            // Check for context reset
            if self.harness.reset_needed(&forge).await? {
                self.harness.execute_reset(&mut forge).await?;
            }
            
            // Check for stall
            self.harness.check_watchdog(&self.store).await?;
            
            // Check for segment loop
            self.check_segment_loop().await?;
            
            tokio::time::sleep(Duration::from_secs(10)).await;
        }
    }
}
```

---

## Summary

The FORGE-SENTINEL pair with these components:

| Component | What it provides |
|---|---|
| Git worktrees | Full isolation between pairs — no shared filesystem |
| File ownership map | No two pairs touch same file — no merge conflicts |
| Slot-scoped artifacts | STATUS.json, WORKLOG.md etc. never confused across pairs |
| Sprintless plugin | MCP tools, skills, hooks, commands — autonomous capability |
| MCP tools | Structured operations: tests, lint, PR creation, codebase search |
| Skills | Role knowledge injected at session start — not prompt bloat |
| Hooks | Infrastructure-level enforcement — invariants cannot be bypassed |
| Slash commands | Multi-step procedures (/plan, /segment-done, /handoff, /status) |
| Sprint contract | Agreed definition of done before any code is written |
| Context resets | Long tasks stay coherent — PreCompact → HANDOFF.md → fresh start |
| No external reviewer | SENTINEL already did the full review — VESSEL does the CI gate |