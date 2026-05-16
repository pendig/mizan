# Runtime Limit Testing

Mizan has two layers of runtime limit validation:

- normal workspace tests that run in CI without Redis
- Redis-backed smoke tests for RPM, TPM cleanup, concurrency leases, and stream-drop lease release

## Local Redis Smoke Run

Start or point to a Redis instance, then run:

```bash
MIZAN_REDIS_URL=redis://127.0.0.1:6379 scripts/limit-smoke.sh
```

The script checks Redis connectivity and runs ignored tests serially so the Redis keys do not race each other.

## Covered Behaviors

- RPM blocks once the configured request window is exceeded.
- Concurrency leases block parallel requests and allow new requests after release.
- Earlier concurrency leases are released when a later scope fails admission.
- Releasing an expired or missing lease does not create a negative Redis counter.
- Dropping an SSE stream releases the runtime limit lease, matching client disconnect behavior.

## MVP Readiness Note

These tests are intentionally local smoke tests rather than default CI tests because they require Redis. The default CI path remains:

```bash
cargo fmt
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
```
