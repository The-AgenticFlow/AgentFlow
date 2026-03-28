# Contributing to Autonomous AI Dev Team

This guide explains how to set up your environment, test the FORGE-SENTINEL pair harness, and contribute effectively to the project.

## 🛠️ Prerequisites

1. **Rust**: [Install Rust](https://rustup.rs/) (latest stable - 1.75+).
2. **Git**: Version 2.25+ (for worktree isolation support).
3. **Claude Code CLI** (Required for FORGE agent):
   ```bash
   npm install -g @anthropic-ai/claude-code
   claude auth login
   ```
4. **GitHub Personal Access Token**: Required for MCP integration.
   - Create at: https://github.com/settings/tokens
   - Scopes needed: `repo`, `workflow`

## ⚙️ Environment Setup

1. **Copy Template**:
   ```bash
   cp .env.example .env
   ```

2. **Configure Variables**:
   - `AGENT_TEST_WORKDIR`: Optional checkout/workdir for real/E2E tests. Point it at the clone you want to inspect live while agents run.
   - `GITHUB_TOKEN` or `GITHUB_PERSONAL_ACCESS_TOKEN`: Required for pair-harness E2E tests
   - `REDIS_URL`: Redis connection string for pair-harness locking (defaults to `redis://localhost:6379`)
   - `REPO_PATH`: Pair-harness-specific alias for the git repository path
   - `OPENAI_API_KEY`: Optional (for legacy Nexus orchestration)
   - `LLM_PROVIDER`: Set to `openai` or `anthropic`
   - `GITHUB_REPOSITORY`: Target repo (e.g., `owner/repo`)

## 🚀 Running the Project

### **RECOMMENDED: Pair Harness E2E Test** ✨
The pair-harness is the core execution engine managing FORGE-SENTINEL pairs. Test it first:

```bash
# Full lifecycle test (worktree isolation, event-driven execution, autonomous recovery)
cargo test -p pair-harness --test pair_real_e2e test_pair_harness_real_e2e -- --ignored

# Crash recovery simulation
cargo test -p pair-harness --test pair_real_e2e test_crash_recovery_simulation -- --ignored
```

**What the test validates:**
- Git worktree isolation on dedicated branches (`forge-N/T-{id}`)
- Event-driven file monitoring (zero polling)
- Autonomous FORGE crash recovery with auto-respawn
- Plugin ecosystem installation (manifest, skills, commands, hooks)
- In-memory file locking (no Redis required)
- SENTINEL evaluation and STATUS.json emission

If you want to watch a live run, set `AGENT_TEST_WORKDIR` to a dedicated checkout first. The tests print the exact worktree and artifact paths they are using.

### Legacy: Mock Demo (Safe, No API Keys Needed)
Uses local mock servers for the LLM and MCP.

1. **Start Mock Infrastructure**:
   ```bash
   # Terminal 1: Mock LLM (OpenAI-compatible)
   python3 scripts/mock_llm.py
   ```

2. **Run Demo**:
   ```bash
   cargo run -p agent-team --bin demo
   ```

### Legacy: Real-World Orchestration
Connects to live GitHub and LLM providers.

```bash
cargo run -p agent-team --bin real_test
```

## 🧪 Testing

### Pair Harness (Core Module)
```bash
# Unit tests
cargo test -p pair-harness

# E2E tests (load .env automatically)
cargo test -p pair-harness --test pair_real_e2e -- --ignored
```

### Legacy Agent Tests
```bash
# All workspace tests
cargo test --workspace

# Specific agent tests
cargo test -p agent-nexus
cargo test -p agent-forge --test forge_claude_e2e
```

## 📂 Architecture Overview

### Pair Harness (Core Engine)
The FORGE-SENTINEL execution engine manages isolated agent pairs:

1. **Git Worktree Isolation**: Each pair operates in `/worktrees/pair-N` on branch `forge-N/T-{id}`
2. **Event-Driven Monitoring**: Uses inotify/FSEvents to watch `WORKLOG.md` and `STATUS.json` (zero polling)
3. **Autonomous Crash Recovery**: Auto-synthesizes HANDOFF.md from WORKLOG.md, increments reset counter, respawns FORGE
4. **Plugin Ecosystem**: Installs complete Claude Code customization:
   - `plugin.json` - Manifest with MCP servers, skills, commands, hooks
   - `skills/*.md` - Agent behavioral guidelines (coding discipline, review protocol)
   - `commands/*.md` - Slash commands (`/plan`, `/handoff`, `/segment-done`, `/status`)
   - `hooks/*.sh` - Lifecycle handlers (session start, pre-tool guard, artifact validation)
5. **In-Memory Locking**: Thread-safe file coordination using `Arc<Mutex<HashMap>>` (no Redis required)

**Key Design Constraints (Non-Negotiable):**
- SENTINEL is ephemeral (spawned on-demand, exits after evaluation)
- No polling loops (event-driven via `notify` crate)
- Dynamic locking (file ownership checked before every write)
- MCP abstraction (no raw API clients in agents)

### Legacy Orchestration (PocketFlow)
- **SharedStore**: Key-value state exchange between agents
- **Graph Nodes**: Each agent is a `BatchNode` in the execution graph
- **PocketFlow**: Flow engine managing state transitions

## 📜 Development Workflow

### For New Contributors

1. **Read the Architecture**:
   - **CRITICAL**: [`docs/forge-sentinel-arch.md`](docs/forge-sentinel-arch.md) - Pair harness specification
   - Optional: [`docs/validation-contract.md`](docs/validation-contract.md) - Acceptance criteria

2. **Verify Your Environment**:
   ```bash
   # Check Rust toolchain
   rustc --version  # Should be 1.75+
   
   # Install Claude Code CLI
   npm install -g @anthropic-ai/claude-code
   claude auth login
   
   # Populate .env with pair-harness settings
   # AGENT_TEST_WORKDIR=/absolute/path/to/the checkout you want to inspect live
   # GITHUB_TOKEN or GITHUB_PERSONAL_ACCESS_TOKEN
   # REDIS_URL=redis://localhost:6379
   # REPO_PATH=/absolute/path/to/Soft-Dev
   
   # Run E2E test to validate setup
   cargo test -p pair-harness --test pair_real_e2e test_pair_harness_real_e2e -- --ignored
   ```

3. **Understand the Code Structure**:
   ```
   crates/pair-harness/
   ├── src/
   │   ├── worktree.rs      # Git worktree isolation
   │   ├── watcher.rs       # Event-driven monitoring
   │   ├── memory_locks.rs  # In-memory file locking
   │   ├── pair.rs          # Main orchestration + crash recovery
   │   ├── mcp_config.rs    # Plugin installer
   │   └── process.rs       # FORGE/SENTINEL spawning
   ├── tests/
   │   └── pair_real_e2e.rs # E2E validation suite
   └── Cargo.toml
   
   .sprintless/plugin/
   ├── plugin.json          # Plugin manifest
   ├── skills/              # Agent behavioral guidelines
   ├── commands/            # Custom slash commands
   └── hooks/               # Lifecycle event handlers
   ```

4. **Making Changes**:
   - Create a feature branch: `git checkout -b feature/your-feature`
   - Follow validation criteria in `docs/validation-contract.md`
   - Add `// Validation: C{category}-{id}` comments for rule compliance
   - Run tests before committing:
     ```bash
     cargo test -p pair-harness
     cargo clippy -p pair-harness
     ```

5. **Contribution Types**:
   - **Core Harness**: Improvements to worktree isolation, event handling, crash recovery
   - **Plugin System**: New skills, commands, or lifecycle hooks
   - **Testing**: Additional E2E scenarios, integration tests
   - **Documentation**: Architecture updates, API docs, contribution examples

6. **Submit PR**:
   - Reference the issue number
   - Include test results (E2E test output if applicable)
   - Explain validation criteria satisfied

### Common Development Tasks

**Adding a new skill:**
```bash
# Create skill file
cat > .sprintless/plugin/skills/my-skill.md << 'EOF'
# My Skill
## When to Use
...
EOF

# Update plugin.json to reference it
# Run test to validate plugin installation
cargo test -p pair-harness test_plugin_installation
```

**Testing crash recovery:**
```bash
cargo test -p pair-harness test_crash_recovery_simulation -- --ignored --nocapture
```

**Debugging event monitoring:**
```bash
# Enable trace logs
RUST_LOG=pair_harness::watcher=trace \
cargo test -p pair-harness test_watcher -- --nocapture
```

---
For detailed architecture, see [`docs/forge-sentinel-arch.md`](docs/forge-sentinel-arch.md).
