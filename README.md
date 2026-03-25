# 🤖 Autonomous AI Dev Team
<img width="2975" height="1571" alt="image" src="https://github.com/user-attachments/assets/9b0a4517-16fb-4939-b129-a43cb6a57cd6" />

An autonomous software development team composed of five AI agents working in a unified Rust/Tokio flow.

## 🚀 Quick Start

1. **Clone & Setup**:
   ```bash
   git clone <repo-url>
   cd Soft-Dev
   cp .env.example .env
   # Edit .env with your keys (OPENAI_API_KEY, GITHUB_PERSONAL_ACCESS_TOKEN)
   ```

2. **Run the Mock Demo** (No real keys required if using mock servers):
   ```bash
   # Start mock servers in separate terminals if needed
   # python3 scripts/mock_llm.py
   # python3 scripts/mock_mcp.py
   cargo run -p agent-team --bin demo
   ```

3. **Run Real-World Orchestration**:
   ```bash
   cargo run -p agent-team --bin real_test
   ```

## 🏗️ The Team
- **NEXUS** (Orchestrator): Scrum Master & Tech Lead. Assigns tickets and approves dangerous commands.
- **FORGE** (Builder): Pragmatic Senior Engineer. Writes code, tests, and pushes PRs via GitHub MCP.
- **SENTINEL** (Reviewer): Paranoid security auditor. Reviews PRs and ensures all logic is tested.
- **VESSEL** (DevOps): Methodical deployment expert. Manages CI/CD and rollbacks.
- **LORE** (Writer): Chronicler & Documenter. Writes ADRs and maintains the project history.

## 📂 Project Structure
- `crates/agent-nexus`: Orchestration logic and ticket assignment.
- `crates/agent-forge`: Code execution via Claude Code CLI.
- `crates/agent-client`: Multi-provider LLM client (OpenAI/Anthropic) + MCP integration.
- `binary/src/bin/real_test.rs`: Live orchestration entry point.
- `binary/src/bin/demo.rs`: Mocked E2E demonstration.

## 📖 Learn More
- [CONTRIBUTING.md](file:///home/christian/sandbox/Soft-Dev/CONTRIBUTING.md): Detailed setup, testing, and contribution workflow.
- [FINAL_DESIGN.md](file:///home/christian/sandbox/Soft-Dev/docs/FINAL_DESIGN.md): Technical specification of the PocketFlow architecture.

<img width="1068" height="378" alt="image" src="https://github.com/user-attachments/assets/d37c29d5-4465-43fe-ac6b-c257fd8413a4" />
