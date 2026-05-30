# Alpha Runbook

This is the lean alpha workflow for Mizan. The project intentionally uses API
docs and shell helpers first, not a dashboard, until gateway correctness,
metering, limits, and observability are stable.

## Chosen Alpha Surface

Use API endpoints plus scripts:

- Admin setup: `POST /admin/provider-connections`, `POST /admin/model-routes`,
  `POST /admin/users/{id}/credits/grant`
- User setup: `POST /auth/register`, `POST /auth/login`, `POST /api-keys`
- Runtime checks: `GET /v1/models`, `POST /v1/chat/completions`,
  `GET /v1/usage`, `GET /v1/credits`, `GET /metrics`
- Future extension (next milestone): `POST /v1/responses` with the same
  OpenAI-compatible shape.
- Model sync helper: `MODEL_SYNC_BASE_URL=... MODEL_SYNC_API_KEY=... scripts/model-sync.sh`
  for syncing OpenAI-compatible model ids from an upstream provider.

Tradeoff: this is less friendly than a web UI, but it keeps alpha scope small
and makes correctness easy to validate in CI-like scripts.

## Local Smoke Test

Prerequisites:

- Rust toolchain
- `curl`
- `python3`
- Redis available at `REDIS_URL` or `redis://127.0.0.1:6379/`

If Redis is not already running locally, start it with Docker Compose:

```bash
docker compose up -d redis
```

Run:

```bash
REDIS_URL=redis://127.0.0.1:6379/ scripts/alpha-smoke.sh
```

The script starts:

- a local OpenAI-compatible mock upstream on port `18182`
- `mizan-api` on port `18180` if it is not already running
- a fresh SQLite database under a temporary directory

Set `DATABASE_URL=postgres://...` before running if you want to exercise the
same flow against PostgreSQL. The API migration path is shared by SQLite and
PostgreSQL through `sqlx::Any`.

## Expected Output

The run should end with:

```text
Alpha smoke passed
```

The smoke covers:

- seeded admin login
- virtual API key creation
- provider connection creation
- model route creation
- credit grant
- model listing
- model sync helper against the mock upstream
- non-streaming chat
- streaming chat
- /v1/responses (when available; tracked in roadmap issue)
- usage and credit reads
- Prometheus metrics scrape

## Alpha 1 Release Gate

Before tagging `v0.1.0-alpha.1`, run:

```bash
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
MIZAN_REDIS_URL=redis://127.0.0.1:6379 scripts/limit-smoke.sh
REDIS_URL=redis://127.0.0.1:6379/ scripts/alpha-smoke.sh
```

See [Alpha 1 Readiness](ALPHA_1_READINESS.md) for the current release
boundary and latest local proof.

## Troubleshooting

- `Connection refused` for Redis means start Redis or set `REDIS_URL`.
- `provider secret key` errors mean set `MIZAN_PROVIDER_SECRET_KEY`.
- Port conflicts can be avoided with `MIZAN_ALPHA_API_PORT` and
  `MIZAN_ALPHA_MOCK_PORT`.
- If the first `cargo run -p mizan-api` build is slow, raise
  `MIZAN_ALPHA_WAIT_SECONDS` for the smoke run.
- If an existing API is already running at `MIZAN_BASE_URL`, the script reuses
  it and only starts the mock upstream.
