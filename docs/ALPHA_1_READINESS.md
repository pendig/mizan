# Alpha 1 Readiness

This document captures the release boundary for `v0.1.0-alpha.1`.

## Release Boundary

`v0.1.0-alpha.1` is a backend/API pre-release. It is intended to prove the
gateway path, metering, credits, limits, and observability before adding a
larger dashboard or broader CLI proxy surface.

Included:

- SQLite-first storage with PostgreSQL-ready SQL preparation.
- Admin seed login and user auth.
- Virtual API keys.
- Provider connections and model routes.
- `GET /v1/models`.
- OpenAI-compatible `POST /v1/chat/completions`.
- Non-streaming and streaming responses.
- Usage events, credit ledger, wallet balance, and credit grants.
- Redis RPM counters and concurrency leases.
- Prometheus gateway metrics.
- Docker Compose local Redis path.

Not included yet:

- Stable/full release guarantee.
- RTK-backed CLI proxy baseline.
- Durable request log and admin audit log foundations.
- Production deployment hardening beyond local smoke validation.

## Required Validation

Run the default workspace checks:

```bash
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
```

Run Redis-backed limit checks:

```bash
MIZAN_REDIS_URL=redis://127.0.0.1:6379 scripts/limit-smoke.sh
```

Run the end-to-end alpha flow:

```bash
REDIS_URL=redis://127.0.0.1:6379/ scripts/alpha-smoke.sh
```

## Latest Local Proof

Validated on 2026-05-17 with Redis from Docker Compose:

- `cargo fmt --all -- --check` passed.
- `cargo clippy --workspace --all-targets -- -D warnings` passed.
- `cargo test --workspace` passed.
- `MIZAN_REDIS_URL=redis://127.0.0.1:6379 scripts/limit-smoke.sh` passed.
- `REDIS_URL=redis://127.0.0.1:6379/ MIZAN_ALPHA_API_PORT=18184 MIZAN_ALPHA_MOCK_PORT=18186 scripts/alpha-smoke.sh` passed.

The alpha smoke covers seeded admin login, virtual API key creation, provider
connection creation, model route creation, credit grant, model listing,
non-streaming chat, streaming chat, usage reads, credit reads, and Prometheus
metrics scraping.

## Remaining MVP Work

These issues do not block the backend/API alpha pre-release, but they should
block a broader MVP or stable release:

- Issue #11: integrate the RTK baseline into `mizan-rtk`.
- Issue #7: add request log and admin audit storage foundations.
