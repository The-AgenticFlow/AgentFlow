# Coding Standards (CODING.md)

## 1. General Principles
- **Simplicity first**: Avoid over-engineering. Favor readable code over clever optimizations.
- **Fail fast**: Use `anyhow` for errors in application code; `thiserror` for library crates (`pocketflow-core`).
- **Async/Await**: Use `tokio` for all async operations. Avoid blocking calls in async contexts.

## 2. Rust Conventions
- Follow `clippy` and `rustfmt` defaults.
- Use `Arc<RwLock<T>>` for shared state but minimize locking duration.
- Document all public traits and structs with doc comments.

## 3. Testing Requirements
- Every new feature MUST have corresponding unit tests.
- Bug fixes MUST include a regression test.
- Use `mockall` for mocking external dependencies where feasible.
- Run `orchestration/agent/tooling/run-tests.sh` before submitting any `STATUS.json`.

## 4. Commits & PRs
- One ticket = One PR. No mega-PRs.
- Commit messages must be descriptive (e.g., `feat(forge): implement task timeout watchdog`).
- If a change is architectural, you MUST propose an ADR via LORE.
