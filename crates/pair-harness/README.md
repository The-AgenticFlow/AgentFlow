# pair-harness

`pair-harness` is the FORGE-SENTINEL execution engine for isolated pair runs.

## Environment

The ignored real E2E tests load `.env` automatically with `dotenvy`, so you can keep the required values in the workspace root `.env` instead of exporting them inline.

Supported variables:

- `AGENT_TEST_WORKDIR`: optional shared checkout/workdir for real/E2E tests. Point it at the clone you want to inspect live.
- `GITHUB_TOKEN` or `GITHUB_PERSONAL_ACCESS_TOKEN`: required for GitHub MCP access.
- `REDIS_URL`: optional, defaults to `redis://localhost:6379`.
- `REPO_PATH`: pair-harness alias. Point it at the repository root or any nested directory inside the repository; the harness resolves the git root before creating worktrees.

Example `.env`:

```dotenv
AGENT_TEST_WORKDIR=/absolute/path/to/Soft-Dev
GITHUB_TOKEN=ghp_your_token_here
REDIS_URL=redis://localhost:6379
REPO_PATH=/absolute/path/to/Soft-Dev
```

## Running the real E2E tests

```bash
cargo test -p pair-harness --test pair_real_e2e test_pair_harness_real_e2e -- --ignored
cargo test -p pair-harness --test pair_real_e2e test_crash_recovery_simulation -- --ignored
```
