# OpenFlows — Quick Start

Get OpenFlows running on a fresh machine in 10 steps. For what OpenFlows is and how it works, see the [README](README.md).

> **Working directory:** all commands run from the **project root** (the directory containing `docker-compose.yml`). No need to `cd` into subdirectories.

## Contents

- [Prerequisites](#prerequisites)
- [Step 1 — Create a GitHub App](#step-1--create-a-github-app)
- [Step 2 — Set up .env](#step-2--set-up-env)
- [Step 3 — Start Docker](#step-3--start-docker)
- [Step 4 — Sign in with GitHub](#step-4--sign-in-with-github)
- [Step 5 — Get your Coder session token](#step-5--get-your-coder-session-token)
- [Step 6 — Configure an LLM model](#step-6--configure-an-llm-model)
- [Step 7 — Test the AI setup](#step-7--test-the-ai-setup)
- [Step 8 — Bootstrap](#step-8--bootstrap)
- [Step 9 — Add a tenant](#step-9--add-a-tenant)
- [Step 10 — Run the controller](#step-10--run-the-controller)
- [Verify it's working](#verify-its-working)
- [Configuration](#configuration)
- [Troubleshooting](#troubleshooting)
- [More](#more)

---

## Prerequisites

- Docker 24+
- Rust 1.70+ (builds the `openflows` binary during bootstrap)
- The `coder` CLI on your `PATH` — bootstrap shells out to `coder templates push`:
  ```bash
  curl -fsSL https://coder.com/install.sh | sh
  ```
- A GitHub account.

---

## Step 1 — Create a GitHub App

OpenFlows agents authenticate to your GitHub repositories (including private repos) through a **GitHub App** using GitHub OIDC / Coder external auth. Creating the app is free.

1. On GitHub, go to **Settings → Developer settings → GitHub Apps → New GitHub App**.
2. Fill in:
   - **GitHub App name** — this becomes your app's URL slug (e.g. `my-openflows-app`).
   - **Homepage URL** — any URL you own.
   - **Callback URL** — exactly:
     ```
     http://localhost:7080/external-auth/primary-github/callback
     ```
   - **Webhook** — can be left **Active: false** (blank).
   - **Permissions → Repository permissions → Contents** — set to **Read and write** (needed to clone/push private repos).
   - **Where can this GitHub App be installed?** — choose **Any account** (or limit to specific orgs).
3. Click **Create GitHub App**.
4. On the app's page, copy the **Client ID** (top of the page).
5. In the **Client secrets** section, click **Generate a new client secret** and copy it now — it is shown only once.
6. Your **Install URL** is:
   ```
   https://github.com/apps/<your-app-slug>/installations/new
   ```
   where `<your-app-slug>` is the slug from step 2. This URL is how you (and your agents) install the app on your org/repos.

Keep the Client ID, Client Secret, and Install URL — you'll need all three in the next step.

---

## Step 2 — Set up `.env`

Create your `.env` from the template:

```bash
cp .env.example .env
```

Fill in the required values:

| Variable | What to put |
|----------|-------------|
| `GITHUB_TOKEN` | GitHub PAT with `repo` scope. |
| `GITHUB_REPOSITORY` | The repo the controller watches, as `owner/repo`. |
| `CODER_SESSION_TOKEN` | Leave empty for now — you'll fill it in [Step 5](#step-5--get-your-coder-session-token). |

Then uncomment the GitHub external auth block in `.env` and set the three values from [Step 1](#step-1--create-a-github-app):

```bash
CODER_EXTERNAL_AUTH_0_ID=primary-github
CODER_EXTERNAL_AUTH_0_TYPE=github
CODER_EXTERNAL_AUTH_0_CLIENT_ID=<your-github-app-client-id>
CODER_EXTERNAL_AUTH_0_CLIENT_SECRET=<your-github-app-client-secret>
CODER_EXTERNAL_AUTH_0_SCOPES=repo
CODER_EXTERNAL_AUTH_0_APP_INSTALL_URL=https://github.com/apps/<your-app-slug>/installations/new
```

> **Why now?** The Coder container reads these vars from `.env` when it starts (see `docker-compose.yml`). Setting them here **before** starting Docker means Coder comes up with GitHub App auth already wired — no UI editing, and no risk of Coder failing to start with empty credentials.

---

## Step 3 — Start Docker

First, make sure ports 6379 (Redis) and 7080 (Coder) are free so there's no conflict. If another, unrelated container is already holding one of those ports, find and remove only that one by name:

```bash
docker ps --filter "publish=6379" --filter "publish=7080"
docker rm -f <conflicting-container-name>
```

Then bring up Redis, the Coder database, and the Coder server:

```bash
docker compose up -d
```

Wait until all three report healthy:

```bash
docker compose ps
```

> **Next:** visit your app's **Install URL** and install the GitHub App on your org/repos so the app can access them.

---

## Step 4 — Sign in with GitHub

Open **http://localhost:7080** and sign in with your GitHub account (Coder's device flow).

---

## Step 5 — Get your Coder session token

1. Open **http://localhost:7080/settings/tokens**
2. Click **Create Token**, copy it.
3. Paste it into `.env`:
   ```bash
   CODER_SESSION_TOKEN=your_token_here
   ```

---

## Step 6 — Configure an LLM model

OpenFlows agents need at least one model.

1. Go to **http://localhost:7080/ai/settings/providers**
2. **Add a provider** (e.g. OpenAI, Anthropic).
3. Go to **http://localhost:7080/ai/settings/models** and **add a model** to that provider (e.g. `deepseek-v4-flash-0731`).

---

## Step 7 — Test the AI setup

Open **http://localhost:7080/agents** and confirm agents/models show up. Say "hello" in the chat to verify the model responds.

---

## Step 8 — Bootstrap

Run the one-time setup to initialize Coder with the OpenFlows templates and config:

```bash
./scripts/prod.sh bootstrap
```

This builds the `openflows` binary into `.dev-binaries/`, creates the admin user, pushes the workspace templates, and verifies GitHub/LLM auth.

Confirm the templates were pushed at **http://localhost:7080/templates**.

---

## Step 9 — Add a tenant

Bind a GitHub repo to the controller:

```bash
./scripts/prod.sh tenant <owner/repo> --name <my-team>
```

You'll see the tenant under **http://localhost:7080/workspaces**.

---

## Step 10 — Run the controller

Open a **separate terminal** (the controller runs in the foreground and streams logs) and run:

```bash
./scripts/prod.sh run
```

Create a GitHub issue in the bound repo → OpenFlows automatically assigns it, provisions a workspace, and starts working.

---

## Verify it's working

In a separate terminal:

```bash
./scripts/prod.sh doctor
```

---

## Configuration

These are optional — the defaults work out of the box. Only touch them if you need to.

| Variable | Default | Notes |
|----------|---------|-------|
| `CODER_ADMIN_USERNAME` | `admin` | Admin account created by bootstrap. |
| `CODER_ADMIN_EMAIL` | `admin@openflows.dev` | |
| `CODER_ADMIN_PASSWORD` | `Op3nFl0ws!` | Must be ≥8 chars with upper, lower, digit, and special char — otherwise bootstrap silently falls back to the default. |
| `REDIS_URL` | `redis://localhost:6379` | Set only if you host Redis elsewhere. |
| `CODER_URL` | `http://localhost:7080` | Set only if you host Coder elsewhere. |
| `OPENFLOWS_TENANT` | `default` | Namespace for Redis keys. |
| `SLACK_WEBHOOK_URL` / `DISCORD_WEBHOOK_URL` | unset | Escalation notifications. |

### Granting a non-admin (OAuth) user the needed permissions

When a team member signs in with GitHub OAuth, Coder creates them as a **regular member**, who can't create workspaces or push templates. If you want OpenFlows to run as that user, grant them these roles (or bootstrap fails with `403 Unauthorized to create workspace`):

| Role | Why |
|------|-----|
| `organization-admin` | Create the control-plane workspace + template management. |
| `organization-template-admin` | Push/update the `openflows-*` templates. |
| `organization-workspace-access` | Required for org workspaces. Keep it — `edit-roles` replaces the whole role set. |

> **Trap:** `organization-workspace-creation-ban` carries a *negative* `workspace:create` permission that **overrides** `organization-admin`. If you see `403 Unauthorized to create workspace`, make sure this role is **not** assigned.

Via CLI:

```bash
export CODER_URL=http://localhost:7080
export CODER_SESSION_TOKEN=<your-token>

# List orgs, then grant roles (include ALL existing roles or they'll be removed)
coder organizations list
coder organizations members edit-roles -O=<org> <username> \
  organization-admin \
  organization-template-admin \
  organization-workspace-access
```

Or via the dashboard: **Admin settings → Organizations → `<your org>` → Members → Edit roles** and select the roles above.

---

## Troubleshooting

### `Failed to run coder templates push` (during bootstrap)

`coder` is missing or not on your `PATH`. Install it and re-run bootstrap:

```bash
curl -fsSL https://coder.com/install.sh | sh
coder version
```

### "No LLM models configured in Coder"

Open **http://localhost:7080/ai/settings/providers** and add a provider/model, then re-run bootstrap.

### `cp: cannot create regular file '.dev-binaries/openflows': Permission denied`

The `.dev-binaries/` directory is root-owned:

```bash
sudo chown -R "$USER":"$USER" .dev-binaries/
```

### Port 6379 already in use

Another process/container holds port 6379. Stop or remove the conflicting container, or change the Redis port mapping in `docker-compose.yml`.

### Coder fails to start / external auth not showing in the UI

Confirm the external auth vars are set in `.env` **before** running `docker compose up -d`, then restart Coder:

```bash
docker compose restart coder
```

Then verify the provider at **http://localhost:7080/external-auth** (or the admin external-auth page).

### Controller not picking up issues

1. Confirm a tenant is bound (`./scripts/prod.sh tenant <owner/repo> --name <my-team>`).
2. Watch the controller's foreground terminal for errors.
3. Verify Coder is reachable: `curl http://localhost:7080/api/v2/buildinfo`.

---

## More

- **Full docs:** [README.md](README.md)
- **Testing & debugging:** [testing_quick_start.md](testing_quick_start.md)
- **Token acquisition:** [token_guide.md](token_guide.md)
