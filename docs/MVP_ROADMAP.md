# MVP Roadmap

## Phase 0 - Product and Repo Foundation

Deliverables:

- PRD.
- Architecture doc.
- Research notes.
- README.
- Apache-2.0 license.
- Initial issue backlog.

Exit criteria:

- The backend scope is clear enough for implementation.
- Internal credit and usage scope is explicit.
- Git repository is initialized.

## Phase 0.5 - Modular Platform Contracts

Deliverables:

- Shared crate and module boundaries.
- Request context shape with request id, trace id, user id, api key id, route,
  provider, and model.
- Structured logging and tracing conventions.
- Stable error envelope rules.
- Provider adapter trait and normalization strategy.
- Credit and limit service ownership rules.

Exit criteria:

- A new provider can be added without editing every handler.
- Logging, tracing, and request context are defined once.
- Credit and limit concerns have clear single owners.

## Phase 1 - Backend Skeleton

Deliverables:

- Rust Cargo workspace.
- RTK-backed CLI proxy base module.
- HTTP server.
- Config loader.
- Shared request context and error types.
- Structured logging and tracing initialization.
- SQLite-first migrations (with PostgreSQL-compatible schema baseline).
- Redis connection.
- Health endpoint.
- Standard log redaction and field conventions.
- Docker Compose for local development.

Exit criteria:

- `docker compose up` starts API, SQLite-backed storage, and Redis.
- `GET /healthz` returns healthy state.
- RTK-backed CLI proxy code is available through `mizan-rtk`.
- Core modules remain separated by concern rather than collapsed into one large
  app crate.

## Phase 2 - Auth, Users, and API Keys

Deliverables:

- User registration/login.
- Admin seed account.
- Password hashing.
- API key creation/revocation.
- API key hashing.
- Basic role middleware.
- Reusable auth primitives in the shared core crate.

Exit criteria:

- User can create a virtual key.
- Revoked key can no longer call protected endpoints.

## Phase 3 - Provider Connections and Model Routes

Status: ✅ Implemented in PR #33

Deliverables:

- Admin provider CRUD (`/admin/provider-connections`).
- OpenAI-compatible provider adapter.
- Model route CRUD (`/admin/model-routes`).
- `/v1/models`.
- Provider modules isolated from gateway handlers.

Exit criteria:

- Admin can expose a public model alias backed by an upstream model.
- User can list allowed models.

## Phase 4 - Gateway Proxy

Status: ✅ Implemented in milestone 4 (PR #33)

Deliverables:

- `POST /v1/chat/completions` with non-streaming response mapping implemented.
- Streaming response path in HTTP/SSE implemented and forwards parsed OpenAI-compatible upstream chunks.
- Upstream provider errors normalized into stable API envelope.
- Request/Trace IDs propagated from request headers and included in response headers.
- Basic provider health status.
- Provider transforms, routing, and gateway orchestration stay separate.

Exit criteria:

- OpenAI SDK can call the gateway by changing only base URL and API key.
- Streaming response format aligns with SSE `chat.completion.chunk` envelope with provider-native chunk forwarding for OpenAI-compatible providers.

## Phase 5 - Usage Metering and Credits

Deliverables:

- Usage event table.
- Wallet table.
- Credit ledger table.
- Admin credit grant endpoint.
- Charge calculation from prompt/output tokens.
- Token estimation fallback.
- User usage and credit endpoints.
- Ledger writes stay immutable and atomic.

Exit criteria:

- Every completed request creates usage and credit ledger records.
- Credit balance cannot go negative during concurrent requests.

## Phase 6 - Limits

Deliverables:

- Redis RPM and TPM counters.
- Redis concurrency leases.
- Per-key and per-provider policies.
- Insufficient-credit block.
- Limit error responses.
- Redis logic stays inside a dedicated limit layer.

Exit criteria:

- Limits behave consistently under concurrent load.
- Leases expire safely if a request crashes.

## Phase 7 - Admin/User Thin UI or CLI

Choose one lean path:

- Thin web UI for admin and user basics.
- CLI plus API docs.

Recommended for fastest MVP:

- Backend API first.
- Minimal admin/user web UI after the gateway is proven.
- Do not build UI until the modular backend shape is stable.

Exit criteria:

- A non-core contributor can run and test the project locally.

## First Public Alpha Scope

Include:

- OpenAI-compatible provider.
- Local model endpoint.
- Virtual keys.
- Usage and credits.
- Redis limits.
- RTK-backed CLI proxy module.
- Admin/user APIs.
- Docker Compose.
- Basic docs.

Exclude:

- Subscription-backed connectors by default.
- Advanced dashboard.
- Prompt compression.
- Enterprise RBAC.
