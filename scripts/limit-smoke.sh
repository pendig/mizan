#!/usr/bin/env bash
set -euo pipefail

export MIZAN_REDIS_URL="${MIZAN_REDIS_URL:-redis://127.0.0.1:6379}"

echo "Using Redis at ${MIZAN_REDIS_URL}"

if command -v redis-cli >/dev/null 2>&1; then
  redis-cli -u "${MIZAN_REDIS_URL}" PING >/dev/null
else
  echo "redis-cli not found; Redis connectivity will be checked by the Rust tests."
fi

cargo test -p mizan-limits -- --ignored --test-threads=1
cargo test -p mizan-api dropping_stream_events_releases_runtime_limit_lease -- --ignored --test-threads=1
