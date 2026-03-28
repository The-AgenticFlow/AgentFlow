# 🤖 Autonomous AI Dev Team
<img width="2975" height="1571" alt="image" src="https://github.com/user-attachments/assets/9b0a4517-16fb-4939-b129-a43cb6a57cd6" />

An autonomous software development team with FORGE-SENTINEL pair programming powered by isolated Claude Code agents.

## 🚀 Quick Start

1. **Clone & Setup**:
   ```bash
   git clone <repo-url>
   cd AgentFlow
   cp .env.example .env
   # Edit .env for pair-harness E2E:
   # - AGENT_TEST_WORKDIR=/absolute/path/to/the checkout you want to watch live
   # - GITHUB_TOKEN or GITHUB_PERSONAL_ACCESS_TOKEN
   # - REDIS_URL=redis://localhost:6379
   # - REPO_PATH=/absolute/path/to/Soft-Dev
   ```

2. **Run Pair Harness E2E Test** (Recommended first step):
   ```bash
   cargo test -p pair-harness --test pair_real_e2e test_pair_harness_real_e2e -- --ignored
   ```

3. **Run the Mock Demo** (Legacy - No real keys required):
   ```bash
   cargo run -p agent-team --bin demo
   ```

## 🏗️ Architecture

### FORGE-SENTINEL Pair Harness
The core execution engine managing isolated agent pairs with autonomous crash recovery:

- **Git Worktree Isolation**: Each pair operates in `/worktrees/pair-N` on dedicated branches
- **Event-Driven Architecture**: Zero-polling file monitoring via inotify/FSEvents  
- **Autonomous Crash Recovery**: Automatic HANDOFF.md synthesis and FORGE respawn
- **Plugin Ecosystem**: Complete Claude Code integration with skills, commands, and hooks
- **In-Memory Locking**: Thread-safe file coordination without external dependencies

### The Agent Team
- **NEXUS** (Orchestrator): Scrum Master & Tech Lead. Assigns tickets and coordinates pairs.
- **FORGE** (Builder): Pragmatic Senior Engineer. Writes code in isolated worktrees, segment by segment.
- **SENTINEL** (Reviewer): Paranoid security auditor. Reviews each segment before approval.
- **VESSEL** (DevOps): Methodical deployment expert. Manages CI/CD and rollbacks.
- **LORE** (Writer): Chronicler & Documenter. Writes ADRs and maintains project history.

## 📂 Project Structure

### Core Harness (NEW ✨)
- **`crates/pair-harness/`**: FORGE-SENTINEL execution engine
  - `src/worktree.rs` - Git worktree isolation
  - `src/watcher.rs` - Event-driven file monitoring  
  - `src/memory_locks.rs` - In-memory file locking
  - `src/pair.rs` - Autonomous pair orchestration
  - `src/mcp_config.rs` - Claude Code plugin installer
  - `tests/pair_real_e2e.rs` - Comprehensive E2E tests

- **`.sprintless/plugin/`**: Claude Code plugin structure
  - `plugin.json` - Plugin manifest
  - `skills/` - Agent behavioral guidelines  
  - `commands/` - Custom slash commands (/plan, /handoff, etc.)
  - `hooks/` - Lifecycle event handlers

### Agent Implementations
- `crates/agent-nexus`: Orchestration logic and ticket assignment
- `crates/agent-forge`: Code generation agent
- `crates/agent-sentinel`: Code review agent
- `crates/agent-client`: Multi-provider LLM client + MCP integration
- `crates/pocketflow-core`: Flow execution primitives

### Documentation
- **`docs/forge-sentinel-arch.md`**: Complete pair harness architecture specification
- **`docs/validation-contract.md`**: Acceptance criteria and validation rules

## 🧪 Testing

### E2E Tests
The ignored real tests load `.env` automatically.
Set `AGENT_TEST_WORKDIR` to a dedicated checkout if you want to watch `worktrees/`, `.sprintless/`, or `forge/workers/` update in real time.

```bash
# Full pair harness lifecycle
cargo test -p pair-harness --test pair_real_e2e test_pair_harness_real_e2e -- --ignored

# Crash recovery simulation
cargo test -p pair-harness --test pair_real_e2e test_crash_recovery_simulation -- --ignored
```

### Unit Tests
```bash
cargo test -p pair-harness
```

## 🔧 Key Features

✅ **Zero External Dependencies**: In-memory locking (no Redis required)  
✅ **Autonomous Recovery**: Handles FORGE crashes with automatic reset  
✅ **Event-Driven**: No polling - pure inotify/FSEvents reactivity  
✅ **Plugin System**: Complete Claude Code customization via manifest  
✅ **Validation**: 13+ architectural criteria passing  

## 📖 Learn More
- [CONTRIBUTING.md](CONTRIBUTING.md): Setup, testing, and contribution workflow
- [docs/forge-sentinel-arch.md](docs/forge-sentinel-arch.md): Pair harness technical specification
- [docs/validation-contract.md](docs/validation-contract.md): Acceptance criteria

<img width="1068" height="378" alt="image" src="https://github.com/user-attachments/assets/d37c29d5-4465-43fe-ac6b-c257fd8413a4" />
