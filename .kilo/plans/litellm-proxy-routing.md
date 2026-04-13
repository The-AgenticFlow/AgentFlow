# Plan: Per-Agent LLM Routing via LiteLLM Proxy (Issue #16)

## Summary

Route all Claude Code API calls through a LiteLLM proxy that maps each agent's API key to the appropriate backend model. This allows cost optimization - e.g., LORE (documentation) uses GPT-4o-mini instead of Claude Sonnet.

## Current State Analysis

### How Agents Currently Work

1. **Agent Registry** (`orchestration/agent/registry.json`):
   - Defines team: nexus, forge, sentinel, vessel, lore
   - Each agent has: `id`, `cli`, `active`, `instances`
   - **No model routing info currently**

2. **Agent Spawning** (`crates/pair-harness/src/process.rs`):
   - FORGE and SENTINEL are spawned via Claude CLI
   - Passes LLM provider env vars directly: `ANTHROPIC_API_KEY`, `GEMINI_API_KEY`, etc.
   - **No proxy integration currently**

3. **LLM Client** (`crates/agent-client/src/`):
   - `FallbackClient` tries providers in order
   - Used by NEXUS for orchestration decisions
   - **Independent from Claude CLI spawning**

### Key Insight

The issue targets **Claude CLI-spawned agents** (FORGE, SENTINEL), not the `agent-client` LLM calls. Claude CLI respects:
- `ANTHROPIC_BASE_URL` - redirects API calls to proxy
- `ANTHROPIC_API_KEY` - used by LiteLLM to identify routing rule

---

## Implementation Plan

### Phase 1: Extend Agent Registry

**File**: `orchestration/agent/registry.json`

Add `model_backend` field to each agent entry:

```json
{
  "team": [
    { "id": "nexus",    "cli": "claude", "active": true, "instances": 1, "model_backend": "anthropic/claude-sonnet-4-5" },
    { "id": "forge",    "cli": "claude", "active": true, "instances": 2, "model_backend": "anthropic/claude-sonnet-4-5" },
    { "id": "sentinel", "cli": "claude", "active": true, "instances": 1, "model_backend": "gemini/gemini-2.5-pro" },
    { "id": "vessel",   "cli": "claude", "active": true, "instances": 1, "model_backend": "groq/llama-3.3-70b-versatile" },
    { "id": "lore",     "cli": "claude", "active": true,  "instances": 1, "model_backend": "openai/gpt-4o-mini" }
  ]
}
```

**File**: `crates/config/src/registry.rs`

Update `RegistryEntry` struct:

```rust
pub struct RegistryEntry {
    pub id: String,
    pub cli: String,
    pub active: bool,
    pub instances: u32,
    pub model_backend: Option<String>,  // NEW: e.g., "anthropic/claude-sonnet-4-5"
}
```

---

### Phase 2: Create LiteLLM Config

**File**: `litellm_config.yaml` (repo root)

```yaml
model_list:
  # FORGE - Primary coding agent, needs Claude Sonnet
  - model_name: claude-sonnet-4-5
    litellm_params:
      model: anthropic/claude-sonnet-4-5
      api_key: os.environ/ANTHROPIC_API_KEY
    model_info:
      id: forge-key

  # NEXUS - Orchestrator, needs Claude Sonnet
  - model_name: claude-sonnet-4-5
    litellm_params:
      model: anthropic/claude-sonnet-4-5
      api_key: os.environ/ANTHROPIC_API_KEY
    model_info:
      id: nexus-key

  # SENTINEL - Reviewer, can use Gemini Pro
  - model_name: claude-sonnet-4-5
    litellm_params:
      model: gemini/gemini-2.5-pro
      api_key: os.environ/GEMINI_API_KEY
    model_info:
      id: sentinel-key

  # VESSEL - DevOps/CI, can use Groq (free tier)
  - model_name: claude-sonnet-4-5
    litellm_params:
      model: groq/llama-3.3-70b-versatile
      api_key: os.environ/GROQ_API_KEY
    model_info:
      id: vessel-key

  # LORE - Documentation, can use GPT-4o-mini
  - model_name: claude-sonnet-4-5
    litellm_params:
      model: openai/gpt-4o-mini
      api_key: os.environ/OPENAI_API_KEY
    model_info:
      id: lore-key

router_settings:
  routing_strategy: least-busy
  num_retries: 3
  fallbacks:
    - model: claude-sonnet-4-5
      fallback: anthropic/claude-sonnet-4-5

litellm_settings:
  request_timeout: 600
  drop_params: true
```

---

### Phase 3: Docker Compose Setup

**File**: `docker-compose.yml` (repo root)

```yaml
services:
  proxy:
    image: ghcr.io/berriai/litellm:main-latest
    ports:
      - "4000:4000"
    volumes:
      - ./litellm_config.yaml:/app/config.yaml:ro
    command: ["--config", "/app/config.yaml", "--port", "4000"]
    env_file:
      - .env
    healthcheck:
      test: ["CMD", "curl", "-f", "http://localhost:4000/health"]
      interval: 10s
      timeout: 5s
      retries: 5
    restart: unless-stopped

  redis:
    image: redis:7-alpine
    ports:
      - "6379:6379"
    volumes:
      - redis_data:/data
    command: redis-server --appendonly yes
    restart: unless-stopped

  agent-team:
    build:
      context: .
      dockerfile: Dockerfile
    ports:
      - "3000:3000"
    volumes:
      - workspace:/workspace
      - ./.agent:/workspace/.agent:ro
      - ./orchestration:/app/orchestration:ro
    environment:
      PROXY_URL: http://proxy:4000
      REDIS_URL: redis://redis:6379
    env_file:
      - .env
    depends_on:
      proxy:
        condition: service_healthy
      redis:
        condition: service_started
    restart: unless-stopped

volumes:
  redis_data:
  workspace:
```

**File**: `Dockerfile` (repo root)

```dockerfile
FROM rust:1.82-bookworm AS builder

WORKDIR /app
COPY Cargo.toml Cargo.lock ./
COPY crates ./crates
COPY binary ./binary
COPY src ./src

RUN cargo build --release -p agent-team

FROM debian:bookworm-slim

RUN apt-get update && apt-get install -y \
    curl \
    git \
    nodejs \
    npm \
    && rm -rf /var/lib/apt/lists/*

# Install Claude CLI
RUN npm install -g @anthropic-ai/claude-code

COPY --from=builder /app/target/release/agent-team /usr/local/bin/

WORKDIR /workspace
CMD ["agent-team"]
```

---

### Phase 4: Update Process Spawning

**File**: `crates/pair-harness/src/process.rs`

Add proxy URL support and per-agent API keys:

```rust
pub struct ProcessManager {
    claude_path: PathBuf,
    github_token: String,
    redis_url: Option<String>,
    proxy_url: Option<String>,  // NEW
}

impl ProcessManager {
    pub fn with_proxy(
        github_token: impl Into<String>,
        redis_url: Option<String>,
        proxy_url: Option<String>,
    ) -> Self {
        // ...
    }

    fn spawn_forge(&self, ...) -> Result<Child> {
        let mut cmd = Command::new(&self.claude_path);
        
        // ... existing args ...

        // Proxy routing
        if let Some(proxy_url) = &self.proxy_url {
            cmd.env("ANTHROPIC_BASE_URL", proxy_url);
            cmd.env("ANTHROPIC_API_KEY", "forge-key");  // LiteLLM routing key
        } else {
            // Direct API access (current behavior)
            cmd.env("ANTHROPIC_API_KEY", std::env::var("ANTHROPIC_API_KEY").unwrap_or_default());
        }
        
        // ... rest of spawn logic
    }

    fn spawn_sentinel(&self, ...) -> Result<Child> {
        // Similar changes, but with "sentinel-key"
        if let Some(proxy_url) = &self.proxy_url {
            cmd.env("ANTHROPIC_BASE_URL", proxy_url);
            cmd.env("ANTHROPIC_API_KEY", "sentinel-key");
        }
        // ...
    }
}
```

---

### Phase 5: Update .env.example

```env
# LiteLLM Proxy (optional - if not set, uses direct API access)
PROXY_URL=http://localhost:4000

# Anthropic - FORGE and NEXUS
ANTHROPIC_API_KEY=sk-ant-...

# Google Gemini - SENTINEL
GEMINI_API_KEY=AIza...

# OpenAI - LORE
OPENAI_API_KEY=sk-...

# Groq - VESSEL (free tier available)
GROQ_API_KEY=gsk_...

# Existing LLM config (for NEXUS orchestration via agent-client)
LLM_PROVIDER=fallback
LLM_FALLBACK=anthropic,gemini
```

---

### Phase 6: Integration Tests

**File**: `tests/proxy_routing.rs`

```rust
#[cfg(test]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_forge_routes_to_claude_sonnet() {
        // Start mock LiteLLM
        // Spawn FORGE with proxy env vars
        // Verify request hits correct backend
    }

    #[tokio::test]
    async fn test_sentinel_routes_to_gemini() {
        // Similar test for SENTINEL
    }

    #[tokio::test]
    async fn test_fallback_on_provider_failure() {
        // Simulate Gemini failure
        // Verify fallback to Claude Sonnet
    }
}
```

---

## Files Changed Summary

| File | Change |
|------|--------|
| `orchestration/agent/registry.json` | Add `model_backend` field |
| `crates/config/src/registry.rs` | Update struct |
| `litellm_config.yaml` | **NEW** - LiteLLM routing config |
| `docker-compose.yml` | **NEW** - Proxy, Redis, agent-team services |
| `Dockerfile` | **NEW** - Build container |
| `crates/pair-harness/src/process.rs` | Add proxy URL support |
| `crates/pair-harness/src/types.rs` | Add proxy_url to PairConfig |
| `.env.example` | Add PROXY_URL and all provider keys |
| `tests/proxy_routing.rs` | **NEW** - Integration tests |

---

## Implementation Order

1. **Phase 1**: Extend registry (low risk, backward compatible)
2. **Phase 2**: Create litellm_config.yaml
3. **Phase 4**: Update process.rs (can work without Docker)
4. **Phase 5**: Update .env.example
5. **Phase 3**: Add Docker files (optional deployment path)
6. **Phase 6**: Add tests

---

## Rollout Strategy

1. **Dev mode**: Run without proxy (current behavior) - `PROXY_URL` not set
2. **Staging**: Enable proxy with Docker Compose
3. **Production**: Deploy proxy as separate service

---

## Risks & Mitigations

| Risk | Mitigation |
|------|------------|
| Proxy becomes bottleneck | LiteLLM supports horizontal scaling |
| Provider API changes | Fallback config handles failures |
| Claude CLI incompatibility | Use official `ANTHROPIC_BASE_URL` env var |
| Cost monitoring | LiteLLM provides usage logs per routing key |

---

## Acceptance Criteria (from issue)

- [ ] LiteLLM proxy service running in Docker Compose on port 4000
- [ ] `litellm_config.yaml` defines routing rules for all five agents
- [ ] Each agent spawned with `ANTHROPIC_BASE_URL=http://proxy:4000`
- [ ] Each agent spawned with its own scoped `ANTHROPIC_API_KEY` that maps to correct model
- [ ] Each agent routes to model defined in registry
- [ ] Fallback configured — any provider failure falls back to `anthropic/claude-sonnet-4-5`
- [ ] `pair-harness/src/process.rs` injects correct env vars per agent
- [ ] `.env.example` updated with all required provider key placeholders
- [ ] `docker-compose.yml` updated with proxy service and health check
- [ ] Integration test: spawn one agent of each type, verify each reaches correct backend
- [ ] README section added explaining how to configure provider keys
