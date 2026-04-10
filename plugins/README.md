# AgentFlow Plugins

Claude Code plugins for each agent in the AgentFlow system.

## Plugin Structure

```
plugins/
|-- nexus/           # Orchestrator - assigns work, approves commands
|-- forge/           # Builder - implements tickets, opens PRs
|-- sentinel/        # Reviewer - evaluates segments, approves work
|-- vessel/          # Deployer - checks CI, merges PRs
|-- lore/            # Documenter - maintains docs and changelogs
```

## Each Plugin Contains

| Component | Description |
|-----------|-------------|
| `.claude-plugin/plugin.json` | Plugin manifest |
| `agents/*.md` | Agent definition with system prompt |
| `skills/*/SKILL.md` | Knowledge documents injected at session start |
| `hooks/hooks.json` | Lifecycle event hooks |
| `scripts/*.sh` | Hook implementation scripts |
| `commands/*.md` | Slash command definitions |
| `.mcp.json` | MCP server configuration |
| `bin/TOOLS.md` | MCP tool documentation |

## Loading a Plugin

```bash
# Single plugin
claude --plugin-dir ./plugins/forge

# Multiple plugins
claude --plugin-dir ./plugins/nexus --plugin-dir ./plugins/forge
```

## Plugin Details

### NEXUS (Orchestrator)

**Skills:**
- `orchestration` - Worker assignment and command gate protocols
- `ticket-triage` - Analyzing and prioritizing tickets

**Commands:**
- `/assign` - Assign ticket to worker
- `/gate-approve` - Approve/reject dangerous command
- `/status-check` - Check system status

**MCP Tools:**
- `get_worker_slots`, `assign_worker`
- `get_command_gate`, `approve_command`, `reject_command`
- `emit_event`

---

### FORGE (Builder)

**Skills:**
- `coding` - Coding standards and testing discipline
- `planning` - Creating implementation plans
- `git-workflow` - Branch management and commits

**Commands:**
- `/plan` - Create implementation plan
- `/segment-done` - Submit segment for review
- `/handoff` - Write handoff for context reset
- `/status` - Write STATUS.json and open PR

**Hooks:**
- `SessionStart` - Check for handoff, initialize session
- `PostToolUse(Write|Edit)` - Run linter after writes
- `PreToolUse(Bash)` - Block dangerous commands
- `PreCompact` - Trigger context reset handoff
- `Stop` - Require STATUS.json or HANDOFF.md

**MCP Tools:**
- `create_pr`, `get_issue`
- `run_tests`, `run_linter`, `search_codebase`
- `commit_segment`, `write_to_shared`, `read_from_shared`
- `emit_event`

---

### SENTINEL (Reviewer)

**Skills:**
- `review` - Code review protocol and feedback guidelines
- `criteria` - The five evaluation criteria

**Commands:**
- `/approve-segment` - Evaluate segment and write verdict
- `/final-review` - Write final review after all segments approved

**Hooks:**
- `SessionStart` - Initialize reviewer session
- `PostToolUse(Write|Edit)` - Validate evaluation files
- `PreToolUse(Bash)` - Enforce readonly mode
- `Stop` - Require evaluation written

**MCP Tools:**
- `run_tests`, `run_linter`
- `read_from_shared`, `write_eval`, `write_final_review`
- `get_changed_files`

---

### VESSEL (Deployer)

**Skills:**
- `ci-gate` - Checking CI status and merge readiness
- `merge-protocol` - Safe PR merging protocol

**Commands:**
- `/check-ci` - Check CI status for PR
- `/merge` - Merge approved PR

**MCP Tools:**
- `get_open_prs`, `check_ci_status`, `merge_pr`
- `check_final_review`, `emit_event`

---

### LORE (Documenter)

**Skills:**
- `documentation` - Maintaining project documentation
- `changelog` - CHANGELOG.md maintenance

**Commands:**
- `/document-pr` - Update documentation for merged PR
- `/update-changelog` - Add entry to CHANGELOG.md

**MCP Tools:**
- `get_merged_prs`, `get_pr_details`
- `update_file`, `read_changelog`, `update_changelog`

## Environment Variables

| Variable | Used By | Description |
|----------|---------|-------------|
| `SPRINTLESS_PAIR_ID` | Forge, Sentinel | Pair identifier (e.g., "forge-1") |
| `SPRINTLESS_TICKET_ID` | Forge | Ticket ID (e.g., "T-42") |
| `SPRINTLESS_WORKTREE` | Forge, Sentinel | Path to worktree |
| `SPRINTLESS_SHARED` | Forge, Sentinel | Path to shared artifacts |
| `GITHUB_TOKEN` | All | GitHub API token |
| `REDIS_URL` | Nexus, Forge, Vessel | Redis connection URL |
| `AGENTFLOW_STORE` | Nexus | Path to shared store |

## Testing

### Run Automated Tests

```bash
# Run all hook and structure tests
cd plugins
./test-plugins.sh
```

This tests:
- Plugin structure (all required files exist)
- JSON validity (all JSON files parse correctly)
- Hook scripts (FORGE, SENTINEL, NEXUS hooks work correctly)

### Test with Claude Code

```bash
# Test a single plugin
claude --plugin-dir ./plugins/forge

# In the session:
# /help              - Show available commands
# /agents            - List available agents
# /forge:coding      - Invoke the coding skill
# /plan              - Run the plan command
```

### Test Mock MCP Server

```bash
# Test the mock FORGE MCP server
echo '{"jsonrpc":"2.0","id":1,"method":"initialize"}' | ./plugins/forge/bin/forge-mcp

# Expected output:
# {"jsonrpc":"2.0","id":1,"result":{"protocolVersion":"2024-11-05",...}}
```

## Next Steps

1. **Implement Real MCP Servers** - Replace mock Python servers with Rust binaries in `crates/plugin-mcp/`
2. **Integration Testing** - Test with real Claude Code sessions
3. **Integrate with Harness** - Symlink plugins into worktrees at runtime
