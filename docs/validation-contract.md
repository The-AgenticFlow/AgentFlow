1. The Validation Contract
File: docs/validation-contract.md

AgentFlow Implementation Validation Contract v1.0
This document defines the strict acceptance criteria for the AgentFlow Pair Harness implementation. All checks must pass for the code generation to be considered successful.

C1: Isolation & Workspace Integrity
ID	Check	Expected Implementation	Pass Criteria
C1-01	Git Worktree Isolation	worktree.rs	Creates distinct worktrees in /worktrees/pair-N. No overlap with main or other pairs.
C1-02	Branch Ownership	worktree.rs	Strictly enforces forge-N/T-{id} branch naming. Prevents checkout to main.
C1-03	Dynamic File Locking	isolation.rs + Redis	acquire_lock uses atomic Redis SET NX. Returns LockError if file is owned by another pair.
C1-04	Artifact Scoping	File paths	All artifacts written to .sprintless/pairs/pair-N/shared/. No absolute paths outside project root.
C2: Lifecycle & Context Management
ID	Check	Expected Implementation	Pass Criteria
C2-01	SENTINEL is Ephemeral	process.rs	SENTINEL process is spawned only on event. Process terminates immediately after eval write. No long-running loop.
C2-02	FORGE Context Reset	reset.rs	PreCompact hook trigger leads to HANDOFF.md creation. Harness detects file and restarts FORGE process.
C2-03	Resume Logic	reset.rs	Fresh FORGE instance reads HANDOFF.md and skips completed segments.
C2-04	Crash Recovery	reset.rs	If FORGE dies without HANDOFF.md, harness synthesizes handoff from WORKLOG.md state.
C3: Event-Driven Architecture
ID	Check	Expected Implementation	Pass Criteria
C3-01	No Polling	watcher.rs	ZERO usage of tokio::time::sleep or loop polling in the main watcher logic.
C3-02	inotify Integration	watcher.rs	Uses notify crate to watch .sprintless/pairs/pair-N/shared/.
C3-03	Reaction Latency	watcher.rs	WORKLOG.md modification triggers spawn_sentinel within the same async tick.
C4: Tooling & Safety (MCP/Hooks)
ID	Check	Expected Implementation	Pass Criteria
C4-01	No Raw Git in Code	process.rs	FORGE is not spawned with git credentials. It must use MCP tools.
C4-02	Hook Enforcement	pre_tool_use_guard.sh	Script blocks git push commands and validates file locks before Write tool execution.
C4-03	MCP Configuration	mcp_config.rs	Generates valid mcp.json connecting to github, redis, filesystem, and shell servers.
C4-04	Shell Allow-list	MCP Config	shell MCP is configured to allow only commands in .agent/tooling/.
C5: Data Integrity
ID	Check	Expected Implementation	Pass Criteria
C5-01	Schema Compliance	artifacts.rs	STATUS.json struct requires status, pair, ticket_id. Serialization fails if missing.
C5-02	Idempotency	worktree.rs	Running create_worktree on an existing slot cleans up old files before creating new ones.
