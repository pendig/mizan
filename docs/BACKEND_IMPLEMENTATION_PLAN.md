# Backend Implementation Plan

This is the first coding plan for the MVP. It assumes Rust, SQLite (phase 0-1),
PostgreSQL-ready design, Redis,
and RTK as the starting base for the CLI proxy/token-saving layer.

## Implementation Rules

Apply these rules to every milestone:

- Keep transport, orchestration, domain logic, and infrastructure separate.
- Keep provider-specific logic behind adapter traits.
- Keep limit checks and wallet writes in their own modules.
- Keep logging, tracing, and request context consistent across crates.
- Keep handlers thin and service-driven.
- Extract shared types early when more than one crate needs them.

## Milestone 1 - Service Skeleton

Crates:

- `crates/mizan-api`
- `crates/mizan-core`
- `crates/mizan-gateway`
- `crates/mizan-rtk`
- `crates/mizan-cli`

Tasks:

- Create Cargo workspace.
- Define crate ownership for `mizan-api`, `mizan-core`, `mizan-gateway`,
  `mizan-rtk`, and `mizan-cli`.
- Add shared request context and error types in `mizan-core`.
- Bring in RTK as the `mizan-rtk` base module instead of rebuilding the CLI
  proxy from scratch.
- Add config from env with startup validation.
- Add centralized structured logging and trace span initialization.
- Add `axum` HTTP server with graceful shutdown.
- Add `/healthz`.
- Add SQLX pool through `sqlx` (SQLite default, PostgreSQL ready).
- Add Redis client.
- Add first migration runner.
- Add Docker Compose.

Acceptance:

- `docker compose up` starts all default services.
- `curl localhost:18180/healthz` returns healthy status.
- `mizan-rtk` exposes the RTK-backed command filtering/proxy functions for
  later gateway and CLI integration.
- Shared request context and error types are available to all crates.
- Logging and tracing are wired once, not duplicated in each handler.

## Milestone 2 - Database Foundation

First migration tables:

- `users`
- `sessions`
- `api_keys`
- `provider_connections`
- `model_routes`
- `wallets`
- `credit_ledger`
- `usage_events`
- `request_logs`
- `admin_audit_logs`

Indexes:

- `api_keys.key_hash`
- `usage_events.user_id, created_at`
- `usage_events.api_key_id, created_at`
- `usage_events.model, created_at`
- `credit_ledger.wallet_id, created_at`
- `provider_connections.enabled`
- `model_routes.public_model`

Acceptance:

- Fresh database migrates from zero.
- Re-running migrations is safe.
- Schema supports many providers and many models without requiring transport
  changes.

## Milestone 3 - Auth and API Keys

Crates:

- `crates/mizan-core`
- `crates/mizan-api`

Tasks:

- Password hashing.
- User registration.
- Login endpoint.
- Admin seed user from env.
- API key creation.
- API key hashing.
- API key revocation.
- Middleware for Bearer keys.
- Shared auth and request context types remain reusable by the gateway and
  admin API.

Acceptance:

- User can register and login.
- User can create an API key.
- Revoked API key fails auth.

## Milestone 4 - Provider and Route Management

Crates:

- `crates/mizan-providers`
- `crates/mizan-gateway`
- `crates/mizan-api`

Tasks:

- Admin provider CRUD.
- Encrypt provider secrets before storage.
- Admin model route CRUD.
- Public model route resolver.
- User-visible `/v1/models`.
- Provider adapters remain isolated from route handlers.
- Model registry lookups stay separate from provider transport details.

Acceptance:

- Admin can add an OpenAI-compatible provider.
- Admin can map `mizan/smart` to an upstream model.
- User can list available models with a virtual key.

## Milestone 5 - Chat Completions Gateway

Crates:

- `crates/mizan-gateway`
- `crates/mizan-providers`

Tasks:

- Implement `POST /v1/chat/completions`.
- Implement non-streaming proxy.
- Implement streaming proxy.
- Normalize upstream errors.
- Attach request id to logs and responses.
- Store request log without raw body by default.
- Keep provider-specific request transforms in `mizan-providers`.
- Keep gateway orchestration separate from metering and wallet writes.

Acceptance:

- OpenAI SDK can call the gateway by changing base URL.
- Streaming and non-streaming calls work.
- Upstream failure returns a useful OpenAI-compatible error shape.

## Milestone 6 - Usage and Credits

Crates:

- `crates/mizan-metering`
- `crates/mizan-wallet`
- `crates/mizan-core`

Tasks:

- Parse upstream usage metadata.
- Estimate tokens when upstream usage is missing.
- Compute credit charge using integer microcredits.
- Insert immutable usage event.
- Insert immutable credit ledger entry.
- Update wallet balance in a transaction.
- Add user usage endpoint.
- Add user credit endpoint.
- Add admin manual credit grant endpoint.
- Put charge calculation behind a reusable service so provider count does not
  affect ledger logic.

Acceptance:

- Every successful request has a usage event.
- Every charged request has a ledger entry.
- Concurrent requests cannot push balance below zero.

## Milestone 7 - Redis Limits

Crates:

- `crates/mizan-limits`

Tasks:

- Implement per-key RPM.
- Implement per-key TPM.
- Implement per-key concurrency.
- Implement per-user concurrency.
- Implement per-provider concurrency.
- Release leases after request completion.
- Use TTLs so crashed requests do not permanently block traffic.
- Keep Redis key naming and lease behavior centralized in `mizan-limits`.

Acceptance:

- Load test shows limits are enforced.
- Limit errors are clear.
- Concurrency lease cleanup is reliable.

## Milestone 8 - Observability

Crates:

- `crates/mizan-api`
- `crates/mizan-core`

Tasks:

- Add Prometheus metrics endpoint.
- Add request counters by provider/model/status.
- Add token counters by provider/model.
- Add latency histograms.
- Add credit spend counters.
- Add structured logs with request id.
- Instrumentation should use the same request context used by gateway, wallet,
  and limit services.

Acceptance:

- Local Prometheus scrape can see gateway metrics.
- Usage data is visible both in database and metrics.

## First Issue Breakdown

Suggested issue order:

1. Bootstrap Rust workspace and bring in RTK as `mizan-rtk`.
2. Add Docker Compose and health endpoint.
3. Add migration runner and initial migrations for the phase-1 schema.
4. Add Redis client and config.
5. Implement user auth and admin seed.
6. Implement virtual API keys.
7. Implement provider connection CRUD.
8. Implement model route CRUD.
9. Implement `/v1/models`.
10. Implement non-streaming `/v1/chat/completions`.
11. Implement streaming `/v1/chat/completions`.
12. Implement usage event recording.
13. Implement credit wallet and ledger.
14. Implement Redis RPM/concurrency limits.
15. Add metrics endpoint.
16. Add smoke test with a local OpenAI-compatible upstream.
