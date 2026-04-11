# AgentFlow - Autonomous AI Development Team

An autonomous software development team composed of AI agents working in a unified Rust/Tokio flow. The team can take GitHub issues and turn them into working code with pull requests - all autonomously.

## Quick Start

```bash
# 1. Clone and setup
git clone https://github.com/The-AgenticFlow/AgentFlow.git
cd AgentFlow
cp .env.example .env
# Edit .env with your API keys

# 2. Verify setup (optional but recommended)
./scripts/check_setup.sh

# 3. Run the orchestration
cargo run --bin real_test
```

## Getting Started

### 📖 Complete Tutorial
**NEW: [TUTORIAL.md](TUTORIAL.md)** - Detailed walkthrough with:
- ✅ Step-by-step setup from zero
- ✅ Expected logs and outputs at each step
- ✅ File structure and locations explained
- ✅ Troubleshooting common issues
- ✅ How to inspect generated code and PRs

### 🚀 Live Flow Walkthrough
**[docs/demo.md](docs/demo.md)** - Step-by-step walkthrough of a live orchestration run with:
- What each log line means as NEXUS discovers issues and assigns work
- How the FORGE-SENTINEL pair communicates through the shared directory
- Where to find generated plans, evaluations, and code changes on disk
- Troubleshooting table for common failures

## The Team

| Agent | Role | Description |
|-------|------|-------------|
| **NEXUS** | Orchestrator | Scrum Master & Tech Lead. Assigns tickets, approves dangerous commands. |
| **FORGE** | Builder | Senior Engineer. Writes code, tests, opens PRs via Claude Code. |
| **SENTINEL** | Reviewer | Security auditor. Reviews PRs, ensures all logic is tested. |
| **VESSEL** | DevOps | Deployment expert. Manages CI/CD and rollbacks. |
| **LORE** | Writer | Documenter. Writes ADRs, maintains project history. |

## Architecture

```
AgentFlow/
|-- .agent/agents/           # Agent personas (nexus.agent.md, forge.agent.md)
|-- crates/
|   |-- agent-nexus/         # Orchestrator node
|   |-- agent-forge/         # Builder node (spawns Claude Code)
|   |-- agent-client/        # LLM client + MCP integration
|   |-- pair-harness/        # Worktree management, process spawning
|   |-- pocketflow-core/     # Flow engine, shared store, routing
|
|-- binary/src/bin/
    |-- real_test.rs         # Live orchestration entry point
    |-- demo.rs              # Mocked demonstration
```

## How It Works

```
GitHub Issues
     |
     v
  +-------+     +-------+     +-------+
  | NEXUS |---->| FORGE |---->|  PR   |
  +-------+     +-------+     +-------+
     |               |
     |               v
     |          Claude Code
     |               |
     v               v
  Routing        STATUS.json
  Logic
```

1. **NEXUS** discovers open GitHub issues and assigns them to available workers
2. **FORGE** spawns Claude Code to implement the solution in an isolated worktree
3. Claude Code writes code, tests, and creates `STATUS.json` with the result
4. **NEXUS** reviews results and assigns more work or handles blocked workers

## Key Files

| File | Purpose |
|------|---------|
| [`.agent/agents/nexus.agent.md`](.agent/agents/nexus.agent.md) | Orchestrator persona and workflow |
| [`.agent/agents/forge.agent.md`](.agent/agents/forge.agent.md) | Builder persona and instructions |
| [`.agent/registry.json`](.agent/registry.json) | Worker slot definitions |
| [`binary/src/bin/real_test.rs`](binary/src/bin/real_test.rs) | Main entry point |
| [`crates/agent-forge/src/lib.rs`](crates/agent-forge/src/lib.rs) | Forge node implementation |

## Documentation

- **[TUTORIAL.md](TUTORIAL.md)** - Complete tutorial with logs, file structure, and troubleshooting
- **[docs/demo.md](docs/demo.md)** - Live flow walkthrough: logs, file locations, and troubleshooting
- **[docs/setup-claude-cli.md](docs/setup-claude-cli.md)** - Claude CLI setup and troubleshooting
- **[CONTRIBUTING.md](CONTRIBUTING.md)** - Development guidelines
- **[docs/forge-sentinel-arch.md](docs/forge-sentinel-arch.md)** - Architecture details

## Requirements

- Rust 1.70+
- Node.js 18+ (for GitHub MCP server)
- **Claude Code CLI** - [Setup Guide](docs/setup-claude-cli.md)
- API keys: `ANTHROPIC_API_KEY`, plus one orchestrator key for `OPENAI_API_KEY`, `GEMINI_API_KEY`, or `ANTHROPIC_API_KEY`, and `GITHUB_PERSONAL_ACCESS_TOKEN`

## License

MIT
