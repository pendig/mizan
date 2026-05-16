# Mizan

[![CI](https://github.com/pendig/mizan/actions/workflows/ci.yml/badge.svg)](https://github.com/pendig/mizan/actions/workflows/ci.yml)
[![License: Apache-2.0](https://img.shields.io/badge/license-Apache--2.0-blue.svg)](LICENSE)

Open-source AI gateway for controlled access, usage metering, and internal
credit accounting.

Mizan lets an admin expose AI provider connections behind virtual API keys,
model routes, rate limits, concurrency limits, and ledger-backed credits. The
first milestone is backend-first: prove the gateway, metering, wallet, and
runtime limit engine before building a large dashboard.

## Status

Mizan is in active bootstrap-to-MVP delivery. Milestone 3 (auth/API keys) and
Milestone 4 (provider/model management + `GET /v1/models`) are implemented.
Milestone 2 (SQLite-first database foundation) is also implemented with
idempotent migrations.
Milestone 5 has a `POST /v1/chat/completions` route with routing and upstream
error shaping in place, and OpenAI-compatible streaming upstream chunks now flow
through SSE as `chat.completion.chunk`.

## MVP Scope

- OpenAI-compatible gateway for `/v1/chat/completions` and `/v1/models`
- Admin-managed upstream connections for API providers and local models
- User registration, virtual API keys, model access rules, and usage history
- Credit accounting based on input/output token prices per 1M tokens
- Redis-backed rate limits, concurrency limits, and short-lived usage counters
- Durable request, usage, and credit ledger storage

## Core Scope

Mizan core manages internal credits, admin grants, user min/max credit policies,
usage charges, provider routing, and access limits. Credits are an internal
accounting unit used by the gateway to meter and control usage.

## Current Docs

- [Product Requirements](docs/PRD.md)
- [Architecture](docs/ARCHITECTURE.md)
- [Engineering Principles](docs/ENGINEERING_PRINCIPLES.md)
- [MVP Roadmap](docs/MVP_ROADMAP.md)
- [Backend Implementation Plan](docs/BACKEND_IMPLEMENTATION_PLAN.md)
- [RTK Base Strategy](docs/RTK_BASE_STRATEGY.md)
- [Research Notes](docs/RESEARCH.md)
- [Name Options](docs/NAME_OPTIONS.md)
- [Initial Issue Backlog](docs/ISSUE_BACKLOG.md)

## Recommended MVP Stack

Use Rust for the first backend implementation, with RTK as the starting base for
the CLI proxy and token-saving layer. Do not rebuild command rewriting,
command-output filtering, or CLI token-saving from scratch. Mizan should wrap,
adapt, or vendor the RTK layer, then build gateway, metering, wallet, and admin
APIs around it. Use Redis for fast runtime controls and PostgreSQL for
source-of-truth records. We run SQLite by default for phase-0/1 and keep
PostgreSQL in the migration model for future production deployment.

Recommended Rust stack:

- `tokio` for async runtime
- `axum` for HTTP APIs and streaming gateway routes
- `sqlx` for SQLite and PostgreSQL
- `redis` or `deadpool-redis` for runtime counters and leases
- `tower` middleware for auth, tracing, timeouts, and limits
- RTK-derived `mizan-rtk` module for CLI proxying and command-output filtering

## Target Architecture

```mermaid
flowchart LR
    Client["User app or AI CLI"] --> Gateway["Mizan gateway"]
    Gateway --> Auth["Virtual API key auth"]
    Auth --> Limits["Redis limits"]
    Limits --> Router["Model router"]
    Router --> Provider["Provider or local model"]
    Provider --> Meter["Usage meter"]
    Meter --> Wallet["Credit ledger"]
    Wallet --> DB["SQLite (default) / PostgreSQL"]
```

## Core Provides

The open-source core should provide:

- Provider connection registry
- Model routing and pricing rules
- Virtual API keys
- Usage metering
- Credit ledger primitives
- Admin-managed min/max credit policy
- Manual/admin credit grants and adjustments
- Admin and user APIs
- Local/self-hosted deployment

## Development

The Rust workspace follows the crate boundaries described in
[Engineering Principles](docs/ENGINEERING_PRINCIPLES.md) and
[Backend Implementation Plan](docs/BACKEND_IMPLEMENTATION_PLAN.md).

Install Rust using the pinned toolchain in `rust-toolchain.toml`, then run:

```sh
cargo fmt --all
cargo check --workspace
cargo test --workspace
```

Environment variables:

- `MIZAN_PROVIDER_SECRET_KEY` (required before creating provider connections, used to encrypt provider API keys at rest)
- `MIZAN_HTTP_ADDR` (default `0.0.0.0:18180`)
- `DATABASE_URL`, `MIZAN_DB_MAX_CONNECTIONS`, `MIZAN_RUN_MIGRATIONS` for storage
- `REDIS_URL`, `MIZAN_LIMIT_RPM`, `MIZAN_LIMIT_TPM`, `MIZAN_LIMIT_CONCURRENCY`,
  `MIZAN_LIMIT_WINDOW_SECONDS`, and `MIZAN_LIMIT_LEASE_SECONDS` for runtime limits
- `MIZAN_ADMIN_EMAIL`, `MIZAN_ADMIN_PASSWORD`, `MIZAN_ADMIN_ROLE` for optional bootstrap

Run the API locally:

```sh
cargo run -p mizan-api
```

Run API, SQLite-backed storage, and Redis with Docker Compose:

```sh
docker compose up --build
```

## License

Apache-2.0. See [LICENSE](LICENSE).

## Security

Do not commit provider credentials, user secrets, or local agent context. See
[SECURITY.md](SECURITY.md).

## Contributing

Mizan is in bootstrap, so small focused changes are welcome. See
[CONTRIBUTING.md](CONTRIBUTING.md) and the issue templates before opening a PR.
