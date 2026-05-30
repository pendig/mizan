# Product Requirements Document

## Working Title

Mizan

## Name Decision

`Mizan` is the chosen project name. It means balance, scale, or measure, which
matches the project's focus on credits, limits, usage, and accountable routing.

## One-Liner

An open-source backend gateway that lets an admin expose controlled AI access
through virtual API keys, token-based credit accounting, and usage tracking.

## Problem

Developers increasingly use many AI tools and model providers: Codex, Gemini
CLI, Claude Code, OpenAI-compatible APIs, local models, and hosted inference
providers. Managing credentials, limits, usage, and credits across those
connections becomes messy when multiple users or projects need access.

The product solves the admin-side problem of pooling AI connections behind one
controlled router, and the user-side problem of getting a simple API key with
clear usage and credit visibility.

## Goals

- Give admins one backend to register upstream AI connections.
- Give users virtual API keys instead of raw provider credentials.
- Normalize all outbound provider output to a single OpenAI-compatible gateway
  contract for chat and responses (including model routing, errors, and usage metadata).
- Track usage by user, API key, provider, route, and model.
- Enforce basic limits: concurrency, requests per minute, tokens per minute,
  and credit balance.
- Price model access using credits per 1M input tokens and per 1M output tokens.
- Ship a lean MVP that prioritizes backend correctness over a large dashboard.

## Non-Goals for MVP

- Full OpenRouter clone.
- Complex prompt compression.
- Enterprise RBAC.
- SSO/OIDC.
- Multi-region deployment.
- A polished marketplace.
- Fine-tuning, RAG, agents, or MCP tools.
- Guaranteeing that subscription-backed providers are legally resellable.

## Important Legal/Product Boundary

Subscription-backed connections can be risky if a provider's terms prohibit
resale, pooling, automation, or shared access. The open-source core should frame
these as admin-owned connections and policy-controlled adapters, not as a tool
for bypassing provider rules.

For MVP, prioritize API-key providers and local OpenAI-compatible models first.
Treat subscription/CLI connectors as experimental adapters with clear warnings,
admin-only credential storage, and per-provider policy flags.

Regardless of upstream transport:

- Internal adapters must normalize to an OpenAI-compatible surface before results
  are returned to clients.
- Both `/v1/chat/completions` and `/v1/responses` should share the same public
  shape and usage/credit semantics.
- Authentication modes can differ (API key, CLI session, browser session), but the
  exposed response contract should stay consistent.

## Personas

### Admin

An operator who owns provider keys, local model endpoints, or subscription
connections. The admin wants to expose safe access to users without sharing raw
credentials.

Admin needs:

- Login to admin API.
- Add provider connections.
- Configure model aliases.
- Configure prices in credits per 1M input/output tokens.
- Configure per-provider and per-user limits.
- See usage, errors, and credit consumption.
- Disable keys or providers quickly.

### User

A developer or app owner who wants one API key and a clear view of usage.

User needs:

- Register/login.
- Create/revoke virtual API keys.
- Call the router through an OpenAI-compatible endpoint.
- See available models.
- Set default model preference.
- See request history, token usage, and credit balance.

## MVP User Stories

- As an admin, I can create an upstream API provider connection.
- As an admin, I can create a local OpenAI-compatible provider connection.
- As an admin, I can map public model names to upstream provider models.
- As an admin, I can set credit prices per 1M input and output tokens.
- As an admin, I can set min/max credit balances or credit caps for a user.
- As an admin, I can grant or adjust user credits manually.
- As an admin, I can set concurrency and rate limits for a key, user, provider,
  or model route.
- As a user, I can register and create a virtual API key.
- As a user, I can call `/v1/chat/completions` using that key.
- As a user, I can call `/v1/responses` using that key.
- As a user, I can stream responses.
- As a user, I can view token usage, credits spent, and remaining credits.
- As the system, any provider path (API-key, CLI session, browser session) emits
  OpenAI-compatible responses and normalized errors.
- As the system, every completed request creates an immutable usage event and
  credit ledger entry.

## MVP Functional Requirements

### Authentication

- Password login for admin and users.
- Hashed virtual API keys.
- API key scopes: `chat:write`, `models:read`, `usage:read`.
- Admin role and user role.

### Provider Connections

MVP provider types:

- `openai_compatible`: API base URL plus API key.
- `local_openai_compatible`: local or private endpoint such as Ollama, vLLM, or
  LM Studio.

Later provider types:

- `native_provider`: provider-specific request/response transforms.
- `subscription_cli`: adapter for subscription-backed CLI/web accounts (including
  Codex, Gemini CLI, or Claude-like flows).
- `browser_session`: controlled session/cookie adapter, only if legally safe.

### Canonical Provider Contract

For every provider family, request handling must normalize upstream output into a
single public shape before calling the route handler:

- `POST /v1/chat/completions` and `POST /v1/responses` use the same normalized
  OpenAI-compatible envelope.
- Error responses keep the same public schema and contract fields (`error`, `type`,
  `code`, request metadata).
- Usage/credit accounting uses the same token semantics across provider families.
- Unsupported or provider-specific payload fields are omitted from public responses.
- Streaming SSE for chat remains OpenAI-compatible (`chat.completion.chunk`).

Non-API adapters are intentionally phased and should not change the contract
surface above.

### Model Routes

Each route contains:

- Public model name, for example `mizan/smart`.
- Upstream provider connection.
- Upstream model name.
- Enabled/disabled flag.
- Input credit price per 1M tokens.
- Output credit price per 1M tokens.
- Optional fallback route.
- Optional max context limit.

### Gateway API

MVP endpoints:

- `GET /healthz`
- `GET /v1/models`
- `POST /v1/chat/completions`
- `POST /v1/responses` (next phase: OpenAI-compatible canonical response surface)

Admin/user API:

- `POST /auth/register`
- `POST /auth/login`
- `GET /me`
- `POST /api-keys`
- `GET /api-keys`
- `DELETE /api-keys/{id}`
- `GET /usage`
- `GET /credits`
- `POST /admin/provider-connections`
- `GET /admin/provider-connections`
- `DELETE /admin/provider-connections/{id}`
- `POST /admin/model-routes`
- `GET /admin/model-routes`
- `DELETE /admin/model-routes/{id}`
- `POST /admin/users/{id}/credits/grant`
- `PATCH /admin/users/{id}/credit-policy`
- `GET /admin/usage`

> Note: `PATCH` endpoints for providers/model routes are currently planned for a
> later refinement pass; CRUD currently provides `GET`, `POST`, and `DELETE`.

### Usage Metering

For every gateway request, capture:

- Request id.
- User id.
- API key id.
- Public model.
- Upstream provider.
- Upstream model.
- Prompt tokens.
- Completion tokens.
- Total tokens.
- Input credits charged.
- Output credits charged.
- Total credits charged.
- Cache hit flag, later.
- Stream flag.
- Status code.
- Error code.
- Latency.

If the upstream returns usage metadata, trust upstream usage. If it does not,
estimate with a tokenizer and mark the event as `estimated=true`.

### Credit Ledger

Credits should be ledger-first:

- `credit_grant`: admin adds credits to a user.
- `usage_charge`: gateway spends credits.
- `refund`: admin or system reverses an event.
- `adjustment`: manual correction.

Never update balances without a ledger row. Balance can be cached, but the
ledger is the truth.

The open-source core only understands credits as internal accounting units.
Admins can set user min/max credit policy and grant credits manually.

### Limits

MVP limits:

- Per-key concurrency.
- Per-user concurrency.
- Per-provider concurrency.
- RPM per key.
- TPM per key.
- Daily credit cap per key.
- Per-user minimum credit threshold, used for warnings or admin reports.
- Per-user maximum credit cap, used to prevent over-allocation in the OSS core.
- Hard block when user balance is insufficient.

Redis should enforce runtime counters and leases. PostgreSQL should store policy
configuration and durable usage records.

## Success Metrics

- A user can create a key and successfully call the gateway.
- Usage records are correct for non-streaming and streaming calls.
- Credit balance cannot go negative under concurrent traffic.
- Admin can disable a provider or API key immediately.
- Basic load test shows stable behavior under concurrent requests.
- Local setup runs with `docker compose up`.

## Open Questions

- Should registration be public by default, invite-only, or admin-created?
- Which tokenizer should be the default for unknown providers?
- Is the first UI a tiny admin/user web dashboard or API-only plus CLI?
- Which providers are safe to include in OSS without encouraging ToS violations?
