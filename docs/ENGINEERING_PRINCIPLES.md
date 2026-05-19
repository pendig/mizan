# Engineering Principles

This document defines the implementation shape for Mizan. The goal is to keep
the system modular, maintainable, and easy to extend as the number of providers,
models, routes, limits, and accounting rules grows.

## Design Goals

- Keep domain logic separate from transport code.
- Keep provider-specific behavior behind adapter interfaces.
- Keep credit accounting ledger-first and transactional.
- Keep limit enforcement hot-path and isolated.
- Keep observability centralized, structured, and consistent.
- Keep public API responses stable even when upstream providers differ.
- Normalize all upstream responses into one OpenAI-compatible public contract before
  returning to clients.

## Recommended Module Boundaries

### `mizan-api`

Owns:

- HTTP server bootstrap.
- Route wiring.
- Auth middleware.
- Request validation.
- Response shaping.
- OpenAI-compatible surface.

Rule:

- Keep handlers thin. They should validate input, call a service, and return a
  response. Do not embed provider or credit logic in handlers.

### `mizan-core`

Owns:

- Shared domain types.
- Error types.
- Request context identifiers.
- Policy primitives.
- Pricing calculations.
- Auth primitives and hashed identifier helpers.

Rule:

- Put reusable business rules here when they are shared by more than one crate.

### `mizan-gateway`

Owns:

- Request orchestration.
- Model resolution.
- Provider selection.
- Stream/non-stream proxy flow.
- Request lifecycle coordination.

Rule:

- Treat this crate as the orchestrator, not the place where provider details
  live.

### `mizan-providers`

Owns:

- Provider adapter trait.
- OpenAI-compatible adapter.
- Local OpenAI-compatible adapter.
- Adapter-specific request/response normalization.

Rule:

- Each provider family should be isolated behind a dedicated module. Adding a
  new provider should not require rewriting the gateway.
- Non-OpenAI upstream formats should be translated here, then shaped as the
  shared `chat.completions` and `responses` contracts at the edge.

### `mizan-metering`

Owns:

- Usage normalization.
- Token estimation fallback.
- Usage event shaping.
- Charge calculation inputs.

Rule:

- Metering should receive normalized request completion data, not raw transport
  internals.

### `mizan-wallet`

Owns:

- Credit ledger writes.
- Balance calculation.
- Wallet policy checks.
- Atomic credit grant and charge operations.

Rule:

- Ledger rows are the source of truth. Never mutate balance without a ledger
  entry.

### `mizan-limits`

Owns:

- Redis counters.
- Redis concurrency leases.
- TTL management.
- Limit and block decisions.

Rule:

- Limit logic must be deterministic and easy to test in isolation.

### `mizan-cli`

Owns:

- Local admin helpers.
- Developer convenience commands.
- RTK-backed proxy entrypoints.

Rule:

- Keep CLI utilities thin and reuse shared services.

## Cross-Cutting Concerns

These concerns should be defined once and reused everywhere.

### Request Context

Every request should carry a small immutable context object with:

- `request_id`
- `trace_id`
- `user_id`
- `api_key_id`
- `provider_id`
- `model`
- `route_id`
- `streaming`

Use this context for logs, metrics, usage events, and error reporting.

### Logging And Tracing

- Use structured logs.
- Include request and trace identifiers in every important log line.
- Avoid raw prompt and response logging by default.
- Keep provider credentials and secrets out of logs.
- Add trace spans around auth, route resolution, provider calls, metering, and
  ledger writes.

### Errors

- Use typed application errors internally.
- Convert them to a stable OpenAI-compatible or API-friendly shape at the edge.
- Separate user-safe messages from internal diagnostic details.

### Configuration

- Parse environment variables once at startup.
- Validate required values early.
- Do not let each crate invent its own env parsing.

### Secrets

- Encrypt provider secrets at rest.
- Hash virtual API keys.
- Hash passwords.
- Never return raw secret material after creation.

## Provider Scaling Rules

When the number of providers grows, the codebase should still feel simple.

- Put provider registration in one place.
- Keep a common provider trait and shared normalized request/response types.
- Store provider-specific transforms inside provider modules.
- Keep model registry data separate from provider transport logic.
- Add a new provider by implementing an adapter, not by editing every handler.

## Credit And Limit Rules

These rules should remain true in every implementation milestone.

- Limit checks happen before expensive upstream work.
- Wallet updates happen in a transaction.
- Usage recording and credit ledger writes are immutable.
- Insufficient credit should fail fast with a clear error.
- Redis is for hot counters and leases, not source-of-truth balances.
- PostgreSQL is the source of truth for durable usage and ledger history.

## Testing Expectations

Add tests at the boundaries where bugs are most expensive:

- Unit tests for pricing, policy, and limit math.
- Integration tests for auth, ledger, and request lifecycle.
- Adapter tests for provider request normalization.
- Smoke tests against a local OpenAI-compatible upstream.

## Practical Rule Of Thumb

If a concern appears in more than one feature, extract it into a shared module
or service instead of duplicating it in handlers.
