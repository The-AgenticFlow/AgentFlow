# Contributing to Autonomous AI Dev Team

This guide explains how to set up your environment, run the project in different modes, and contribute effectively.

## 🛠️ Prerequisites

1. **Rust**: [Install Rust](https://rustup.rs/) (latest stable).
2. **Node.js**: Required for Claude Code CLI and MCP servers (v18+).
3. **Python 3**: Required for running mock servers.
4. **Claude Code CLI** (Required for Forge workers):
   The FORGE agent spawns Claude Code processes to implement code. Without this binary,
   Forge workers will fail with `Failed to spawn FORGE process`.

   ```bash
   # Install Claude Code CLI globally
   npm install -g @anthropic-ai/claude-code

   # Authenticate (required on first run)
   claude auth login

   # Verify installation
   claude --version
   ```

   Then set `CLAUDE_PATH` in your `.env` to the absolute path:
   ```bash
   # Find the path
   which claude

   # Set it in .env (example output)
   CLAUDE_PATH=/home/user/.nvm/versions/node/v24.14.1/bin/claude
   ```

   **Troubleshooting**: If you see `Failed to spawn FORGE process` in logs, the most
   common cause is that the `claude` binary cannot be found. Verify:
   - `claude --version` works from the same terminal you run `cargo` from
   - `CLAUDE_PATH` in `.env` points to an existing, executable binary
   - The binary has execute permissions (`chmod +x <path>` on Linux/macOS)

## ⚙️ Environment Setup

1. **Copy Template**:
   ```bash
   cp .env.example .env
   ```

2. **Configure Variables**:
   - `OPENAI_API_KEY`: Required for Nexus if using OpenAI.
   - `GEMINI_API_KEY`: Required for Nexus if using Gemini.
   - `LLM_PROVIDER`: Set to `openai`, `gemini`, or `anthropic`.
   - `GITHUB_PERSONAL_ACCESS_TOKEN`: Required for real-world PR creation.
   - `GITHUB_REPOSITORY`: The target repository (e.g., `owner/repo`).

## 🚀 Running the Project

**New contributors**: Read the **[live flow walkthrough](docs/demo.md)** first — it explains what you will see in the logs at each stage and where files end up on disk.

### Option A: Local Mock Demo (Safe, No API Keys Needed)
This uses local mock servers for the LLM and MCP, and a mock Claude script for Forge.

1. **Start Mock Infrastructure**:
   ```bash
   # Terminal 1: Mock LLM (OpenAI-compatible)
   python3 scripts/mock_llm.py
   
   # Terminal 2: Mock GitHub MCP
   # (The demo binary starts this automatically via GITHUB_MCP_CMD)
   ```

2. **Run Demo**:
   ```bash
   cargo run -p agent-team --bin demo
   ```

### Option B: Real-World Orchestration
This connects to live GitHub and live LLM providers.

1. **Run Real Test**:
   ```bash
   cargo run -p agent-team --bin real_test
   ```

## 🧪 Testing

### Unit Tests
```bash
cargo test --workspace
```

### End-to-End Tests
We have specific E2E tests for core logic:
```bash
# Test Nexus decision making
cargo test -p agent-nexus

# Test Forge suspension logic (mocked)
cargo test -p agent-forge --test forge_claude_e2e
```

## 📂 Architecture Overview
- **SharedStore**: A key-value store where agents exchange state (e.g., `worker_slots`, `tickets`).
- **Graph Nodes**: Each agent is a `BatchNode` that reads from the store and writes back "actions" (e.g., `work_assigned`).
- **PocketFlow**: The engine that executes the graph and manages state transitions.

## 📜 Development Workflow

If you want to contribute, please follow these steps:

1. **Understand the Architecture**: Read the [design.pdf](file:///home/christian/sandbox/Soft-Dev/docs/design.pdf) (provided in the repository) to get a deep understanding of the PocketFlow engine and agent roles.
2. **Verify the Environment**: Run all tests (unit and E2E) to ensure the current flow is running fine on your side:
   ```bash
   cargo test --workspace
   cargo run -p agent-team --bin demo
   ```
3. **Get Assigned**: Create a new issue or comment on an existing one to express your interest. I will then add you to the repository as a contributor.
4. **Implement**: Follow the standard agentic coding workflow (Plan -> Implement -> Verify -> Walkthrough).

---
For more specific rules, see `sprintless/agent/standards/`.
