# Initial Issue Backlog

This backlog is ordered for a fast backend-first MVP.

## Platform Contracts

1. Define shared request context and error envelope.
2. Define provider adapter trait and normalized request/response types.
3. Define tracing, logging, and metrics conventions.
4. Define credit and limit service boundaries.

## Foundation

1. Initialize Rust Cargo workspace.
2. Bring RTK into the workspace as `mizan-rtk`.
3. Preserve RTK CLI proxy behavior as the baseline.
4. Add `mizan-api`, `mizan-core`, `mizan-gateway`, and `mizan-cli` crates.
5. Add `axum` HTTP server with graceful shutdown.
6. Add config loader from environment.
7. Add Docker Compose with API, PostgreSQL, and Redis.
8. Add `/healthz`.
9. Wire shared request context and structured logs.

## Storage

Status: ✅ Implemented in current branch (`fda344c`)

1. Add PostgreSQL migrations.
2. Add migration runner.
3. Create `users`, `api_keys`, `provider_connections`, `model_routes`,
   `wallets`, `credit_ledger`, and `usage_events`.
4. Add indexes for API key hash, usage queries, and wallet ledger lookups.
5. Keep schema ready for multiple providers and multiple model aliases.

## Auth

1. Add password hashing.
2. Add user registration/login.
3. Add admin seed account from env.
4. Add virtual API key creation and revocation.
5. Add Bearer API key middleware.
6. Reuse the shared request context and error envelope.

## Provider Routing

Progress status (current): Milestone 4 done in PR #33 with non-streaming chat proxy follow-up in same PR.

1. Add provider connection CRUD.
2. Add OpenAI-compatible provider adapter.
3. Add local OpenAI-compatible provider mode.
4. Add model route CRUD.
5. Add `/v1/models`.
6. Keep provider-specific logic behind adapter modules.

## Gateway

1. Add non-streaming `/v1/chat/completions`.
2. Add streaming `/v1/chat/completions`.
3. Normalize upstream errors.
4. Add request IDs.
5. Add provider health state.
6. Keep gateway orchestration separate from metering and ledger code.

## Credits And Metering

1. Add integer microcredit wallet.
2. Add ledger-first credit grant.
3. Add usage event recording.
4. Add credit charging from input/output tokens.
5. Add estimated token accounting fallback.
6. Add user usage and credit endpoints.
7. Keep charge calculation reusable across all providers.

## Runtime Limits

1. Add Redis RPM counters.
2. Add Redis TPM counters.
3. Add per-key concurrency leases.
4. Add per-user concurrency leases.
5. Add per-provider concurrency leases.
6. Add insufficient-credit block.
7. Keep Redis key naming and TTL rules in one place.

## Observability

1. Add structured request logs.
2. Add Prometheus metrics endpoint.
3. Add request counters by provider/model/status.
4. Add token and credit counters.
5. Add latency histograms.
6. Propagate request and trace context through all layers.
