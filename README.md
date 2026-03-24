# Autonomous AI Dev Team

An autonomous software development team composed of five AI agents working in a unified Rust/Tokio flow.

## 🤖 The Team
- **NEXUS** (Orchestrator): Scrum Master & Tech Lead. Assigns tickets and approves dangerous commands.
- **FORGE** (Builder): Pragmatic Senior Engineer. Writes code, tests, and pushes PRs via GitHub MCP.
- **SENTINEL** (Reviewer): Paranoid security auditor. Reviews PRs and ensures all logic is tested.
- **VESSEL** (DevOps): Methodical deployment expert. Manages CI/CD and rollbacks.
- **LORE** (Writer): Chronicler & Documenter. Writes ADRs and maintains the project history.

## 🏗️ Architecture
Built on **PocketFlow (Rust)**, the team operates as a Graph + Shared Store state machine:
1. **Flow**: Defined in `main.rs`, handles all state transitions.
2. **SharedStore**: Redis-backed concurrent context for agent communication.
3. **Claude Code**: The primary execution engine for all agents, routed via a **LiteLLM** proxy.

## ⚙️ Configuration
The team's "constitution" is located in the `.agent/` directory:
- `registry.json`: Live registry of active agents and instances.
- `agents/`: Personality and capability definitions for each agent.
- `standards/`: Coding, Security, and Review rules.
- `templates/`: Structured Markdown templates for tasks and status reports.

## 🚀 Getting Started
Check the `FINAL_DESIGN.md` in the development brain for the full technical specification.
