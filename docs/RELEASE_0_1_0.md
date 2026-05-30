# v0.1.0 Release Readiness

This document captures the stable backend/API release boundary for `v0.1.0`.

## Release Boundary

`v0.1.0` is a backend/API-first stable release. It is stable for the current
OpenAI-compatible API provider surface and local development workflow, not a
promise that every future provider family is runnable yet.

Included:

- SQLite-first storage with PostgreSQL-ready SQL preparation.
- Admin seed login, user auth, sessions, and virtual API keys.
- Provider connections with `auth_mode` metadata.
- Model routes and `GET /v1/models`.
- OpenAI-compatible `POST /v1/chat/completions`.
- OpenAI-compatible non-streaming `POST /v1/responses`.
- Non-streaming and streaming chat responses.
- Usage events, credit ledger, wallet balance, and credit grants.
- Redis RPM counters and concurrency leases.
- Request log and admin audit log foundations.
- Centralized gateway completion logging.
- RTK baseline crate and CLI proxy/filter tooling.
- Prometheus gateway metrics.
- Docker Compose local Redis path and local Redis fallback for smoke validation.

Not included yet:

- Production deployment hardening beyond local smoke validation.
- Runnable non-API provider adapters for subscription CLI or browser-session
  auth modes. Registration metadata and validation are present, but runtime
  adapters remain a follow-up.
- Advanced dashboard, enterprise RBAC, prompt compression, or billing-provider
  integration.

## Release Gate

Run these from `main` before tagging:

```bash
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
MIZAN_REDIS_URL=redis://127.0.0.1:6379 scripts/limit-smoke.sh
REDIS_URL=redis://127.0.0.1:6379/ MIZAN_ALPHA_API_PORT=18194 MIZAN_ALPHA_MOCK_PORT=18196 scripts/alpha-smoke.sh
```

## Latest Proof

Validated on 2026-05-31 from the final `main` release candidate:

- `cargo fmt --all -- --check` passed.
- `cargo clippy --workspace --all-targets -- -D warnings` passed.
- `cargo test --workspace` passed.
- `MIZAN_REDIS_URL=redis://127.0.0.1:6379 scripts/limit-smoke.sh` passed
  against local `redis-server`.
- `REDIS_URL=redis://127.0.0.1:6379/ MIZAN_ALPHA_API_PORT=18194 MIZAN_ALPHA_MOCK_PORT=18196 scripts/alpha-smoke.sh`
  passed against local `redis-server` and a mock OpenAI-compatible upstream.

The alpha smoke covered seeded admin login, admin virtual API key creation,
provider connection creation, model route creation, credit grant, model listing,
model sync, non-streaming chat, non-streaming responses, streaming chat, usage
reads, credit reads, and Prometheus metrics scraping.
