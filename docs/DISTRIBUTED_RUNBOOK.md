# Distributed Proxy Runbook

This document is the practical smoke and verification runbook for the v0.2.0
`daemon` data-plane preview.

## Prerequisites

- Rust toolchain
- `curl`
- `python3`
- Redis available at `REDIS_URL` or `redis://127.0.0.1:6379/`

If Redis is not already available:

```bash

docker compose up -d redis
```

## Prereq Ports

The distributed smoke script uses separate ports from alpha so it can run beside
other local demos:

- Control plane: `MIZAN_DISTRIBUTED_API_PORT` (default `18184`)
- Mock upstream: `MIZAN_DISTRIBUTED_MOCK_PORT` (default `18186`)

Override with environment variables if these are in use.

## End-to-End Smoke

From repository root:

```bash
MIZAN_DISTRIBUTED_API_PORT=18184 \
MIZAN_DISTRIBUTED_MOCK_PORT=18186 \
MIZAN_DISTRIBUTED_MODEL=mock-gpt \
MIZAN_PROVIDER_SECRET_KEY=change-me-smoke \
MIZAN_ADMIN_EMAIL=admin@mizan.local \
MIZAN_ADMIN_PASSWORD=change-me-admin \
scripts/distributed-smoke.sh
```

The script now performs:

1. Start mock OpenAI-compatible upstream (`scripts/mock-openai.py`).
2. Start `mizan-api` with seeded admin credentials.
3. Create an admin API key.
4. Create one daemon node with a one-time token.
5. Register daemon and run `mizan-daemon` against that control plane.
6. Wait for daemon-backed model to appear in `/v1/models`.
7. Register a regular user and create user API key.
8. Call `/v1/chat/completions` against daemon-backed model.
9. Verify:
   - user usage endpoint returns a row (`/v1/usage`)
   - host/admin usage endpoint includes daemon metadata (`/admin/usage`)
   - metrics has non-`none` `daemon_node` label for the request (`/metrics`)

If the script exits without error it prints:

```text
Distributed smoke passed
```

## What to review when smoke fails

- If `MIZAN_DISTRIBUTED_WORK_DIR` is not set, `scripts/distributed-smoke.sh`
  creates a temporary `WORK_DIR` and removes it on exit.
- If `MIZAN_DISTRIBUTED_WORK_DIR` is set, verify whether you need to clean it
  manually after a run.
- Daemon logs are written to `/tmp/mizan-daemon-distributed.log` while running.
- Confirm control-plane auth and request signature headers exist on every daemon call.
- Verify Redis is reachable and `/healthz` is healthy before the smoke starts.

## Security foundation checks covered

This runbook validates v0.2 transport hardening in motion:

- `mizan-daemon` signs each control-plane call with:
  - `x-mizan-daemon-signature`
  - `x-mizan-daemon-timestamp`
  - `x-mizan-daemon-nonce`
- `mizan-api` verifies signature freshness + nonce and rejects stale/replayed or
  malformed calls.
- `daemon` payloads are not sent in clear over the request logs; dispatch and
  usage surfaces are checked through explicit metadata fields instead.

`payload encryption` is still documented as a future follow-up item and should be
tracked separately before enabling a managed hosted variant.
