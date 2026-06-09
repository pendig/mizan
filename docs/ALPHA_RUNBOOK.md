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
  `POST /v1/responses`, `GET /v1/usage`, `GET /v1/credits`, `GET /metrics`
- Model sync helper: `MODEL_SYNC_BASE_URL=... MODEL_SYNC_API_KEY=... scripts/model-sync.sh`
  for syncing OpenAI-compatible model ids from an upstream provider. The
  helper uses `python3` for JSON parsing, so `jq` is not required.

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

## Self-Serve Distributed Onboarding

Once the API and at least one healthy `mizan-daemon` node are running, a normal
user should be able to register, create a virtual API key, and discover
daemon-backed models without host details leaking through `/v1/models`.

```bash
BASE_URL="${MIZAN_BASE_URL:-http://127.0.0.1:18180}"
EMAIL="user-$(date +%s)@example.test"
PASSWORD="change-me-distributed"

curl -fsS -X POST "${BASE_URL}/auth/register" \
  -H 'content-type: application/json' \
  -d "{\"email\":\"${EMAIL}\",\"password\":\"${PASSWORD}\"}"

SESSION_TOKEN="$(
  curl -fsS -X POST "${BASE_URL}/auth/login" \
    -H 'content-type: application/json' \
    -d "{\"email\":\"${EMAIL}\",\"password\":\"${PASSWORD}\"}" \
    | python3 -c 'import json,sys; print(json.load(sys.stdin)["access_token"])'
)"

API_KEY="$(
  curl -fsS -X POST "${BASE_URL}/api-keys" \
    -H "authorization: Bearer ${SESSION_TOKEN}" \
    -H 'content-type: application/json' \
    -d '{"label":"distributed-onboarding"}' \
    | python3 -c 'import json,sys; print(json.load(sys.stdin)["key"])'
)"

curl -fsS "${BASE_URL}/v1/models" \
  -H "authorization: Bearer ${API_KEY}" \
  | python3 -m json.tool
```

Run the same flow as a repeatable smoke check:

```bash
MIZAN_BASE_URL="${BASE_URL}" \
MIZAN_DISTRIBUTED_MODEL="llama3.1" \
scripts/distributed-onboarding-smoke.sh
```

Set `MIZAN_DISTRIBUTED_RUN_CHAT=1` to include a daemon-backed
`/v1/chat/completions` call after the distributed dispatch lifecycle is enabled.
Until then, the smoke script validates self-serve onboarding and model
visibility only.

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
- /v1/responses
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
