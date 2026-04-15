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

2. **Choose Your Mode**:

   ### Mode 1: Proxy (Recommended)
   Routes all LLM calls through a LiteLLM proxy. **Individual API keys are optional.**

   ```env
   PROXY_URL=http://localhost:8080/v1      # Required - enables proxy mode
   PROXY_API_KEY=your-key                   # Optional - for authenticated proxies
   MODEL_PROVIDER_MAP=glm=openai,...        # Maps model names to client format
   ```

   ### Mode 2: Direct
   Calls LLM providers directly. **Requires individual API keys.**

   ```env
   # PROXY_URL not set (or commented out)
   LLM_FALLBACK=anthropic,gemini,openai     # Provider order
   ANTHROPIC_API_KEY=your-key               # Required for anthropic
   GEMINI_API_KEY=your-key                  # Required for gemini
   OPENAI_API_KEY=your-key                  # Required for openai
   ```

3. **Required Variables** (both modes):
   - `GITHUB_PERSONAL_ACCESS_TOKEN`: For GitHub API (issues, PRs, CI polling)
   - `GITHUB_REPOSITORY`: Target repository (e.g., `owner/repo`)
   - `CLAUDE_PATH`: Path to Claude CLI binary (for Forge workers)

## 🔑 Environment Variables Reference

| Variable | Proxy Mode | Direct Mode | Description |
|----------|------------|-------------|-------------|
| `PROXY_URL` | **Required** | Not set | Enables proxy mode |
| `PROXY_API_KEY` | Optional | N/A | Auth key for proxy |
| `ANTHROPIC_API_KEY` | Optional | Required* | Anthropic/Claude API key |
| `OPENAI_API_KEY` | Optional | Required* | OpenAI API key |
| `GEMINI_API_KEY` | Optional | Required* | Google Gemini API key |
| `LLM_FALLBACK` | N/A | Optional | Provider fallback order |
| `MODEL_PROVIDER_MAP` | Optional | Optional | Model→provider mapping |

*Required only if listed in `LLM_FALLBACK`

**Key insight**: When `PROXY_URL` is set, the system uses proxy mode and individual API keys become optional. The proxy handles all authentication and model routing.

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

**If your gateway supports Anthropic protocol** (LiteLLM, native Anthropic API):
```bash
# Just run — no proxy needed
cargo run -p agent-team --bin real_test
```

**If your gateway only supports OpenAI protocol** (common for third-party gateways):
```bash
# Terminal 1: Start the local Anthropic-to-OpenAI proxy
./scripts/start_proxy.sh

# Terminal 2: Run the orchestration
cargo run -p agent-team --bin real_test
```

The proxy reads `GATEWAY_URL` and `GATEWAY_API_KEY` from `.env` automatically, translates Claude CLI's Anthropic-format requests into OpenAI format, and forwards them to your gateway. See [Local Anthropic Proxy](#local-anthropic-proxy-openai-only-gateways) below for details.

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

### <a id="per-agent-llm-routing-litellm-proxy"></a>Per-Agent LLM Routing (LiteLLM Proxy)

AgentFlow supports routing each agent to a different LLM backend through a LiteLLM proxy. This allows cheaper models for simpler tasks.

**Registry configuration** (`orchestration/agent/registry.json`):

```json
{ "id": "forge",    "model_backend": "anthropic/claude-sonnet-4-5",     "routing_key": "forge-key" },
{ "id": "sentinel", "model_backend": "gemini/gemini-2.5-pro",          "routing_key": "sentinel-key" },
{ "id": "vessel",   "model_backend": "groq/llama-3.3-70b-versatile",   "routing_key": "vessel-key" },
{ "id": "lore",     "model_backend": "openai/gpt-4o-mini",             "routing_key": "lore-key" }
```

**How it works**:

1. `model_backend` is sent to the LLM client as the model name
2. `MODEL_PROVIDER_MAP` determines which client format to use (e.g., `glm=openai` sends OpenAI-format requests)
3. `routing_key` maps to backend models in your proxy's `litellm_config.yaml`

**Quick setup** (self-hosted LiteLLM):

```bash
# .env
PROXY_URL=http://localhost:4000/v1

# litellm_config.yaml
model_list:
  - model_name: forge-key
    litellm_params:
      model: anthropic/claude-sonnet-4-5
      api_key: os.environ/ANTHROPIC_API_KEY
```

### <a id="local-anthropic-proxy-openai-only-gateways"></a>Local Anthropic Proxy (OpenAI-Only Gateways)

Claude CLI speaks the Anthropic Messages API (`/v1/messages`). If your LLM gateway only supports the OpenAI Chat Completions format (`/v1/chat/completions`), Claude CLI will get a `403`/`404` and exit immediately.

AgentFlow includes a local proxy that translates between the two protocols:

```
Claude CLI ──Anthropic format──> localhost:8080 ──OpenAI format──> Gateway
```

**Setup** — add these to `.env`:

```env
# Points Claude CLI and Nexus at the LOCAL proxy
PROXY_URL=http://localhost:8080/v1
PROXY_API_KEY=your-gateway-api-key

# Tells the LOCAL proxy where to FORWARD (the remote gateway)
GATEWAY_URL=https://api.ai.camer.digital/v1/
GATEWAY_API_KEY=your-gateway-api-key
```

**Run** — two terminals:

```bash
# Terminal 1: Start proxy (reads .env automatically)
./scripts/start_proxy.sh

# Terminal 2: Run orchestration
cargo run --bin real_test
```

**When your provider adds native Anthropic support**, just change `PROXY_URL` to the gateway directly and remove `GATEWAY_*`:

```env
PROXY_URL=https://api.ai.camer.digital/v1/
PROXY_API_KEY=your-gateway-api-key
# Remove GATEWAY_URL and GATEWAY_API_KEY — no longer needed
```

**Also see**: `MODEL_PROVIDER_MAP` in `.env.example` for routing non-Anthropic models (like `glm-5`) through `OpenAiClient` instead of `AnthropicClient` within the Nexus agent.

---

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
For more specific rules, see `orchestration/agent/standards/`.
