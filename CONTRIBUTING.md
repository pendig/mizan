# Contributing

Thanks for your interest in Mizan.

Mizan is currently in bootstrap stage. The first contribution priority is the
Rust backend foundation described in `docs/BACKEND_IMPLEMENTATION_PLAN.md`.

## Scope

Good first contributions:

- Rust workspace setup.
- `axum` server skeleton.
- PostgreSQL migrations.
- Redis limit primitives.
- OpenAI-compatible provider adapter.
- Virtual API key auth.
- Usage metering.
- Credit ledger implementation.
- Documentation improvements.

## Development Principles

- Prefer ledger-first credit mutations.
- Do not log prompts or provider secrets by default.
- Use Rust formatting and linting once the workspace exists.
- Add focused tests for auth, ledger, limits, and gateway behavior.

## Local Checks

Run these before opening a pull request:

```sh
cargo fmt --all
cargo check --workspace
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
```

For local services:

```sh
docker compose up --build
```

## Local-Only Context

`CONTEXT.md` is intentionally ignored by git. It may contain local AI-agent
coding guidance and private integration notes. Do not commit it.
