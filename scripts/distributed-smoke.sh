#!/usr/bin/env bash
set -euo pipefail

API_PORT="${MIZAN_DISTRIBUTED_API_PORT:-18184}"
MOCK_PORT="${MIZAN_DISTRIBUTED_MOCK_PORT:-18186}"
if [[ -n "${MIZAN_DISTRIBUTED_WORK_DIR:-}" ]]; then
  WORK_DIR="${MIZAN_DISTRIBUTED_WORK_DIR}"
  CLEANUP_WORK_DIR=0
else
  WORK_DIR="$(mktemp -d)"
  CLEANUP_WORK_DIR=1
fi
BASE_URL="${MIZAN_BASE_URL:-http://127.0.0.1:${API_PORT}}"
MOCK_URL="${MIZAN_MOCK_BASE_URL:-http://127.0.0.1:${MOCK_PORT}}"
WAIT_SECONDS="${MIZAN_DISTRIBUTED_WAIT_SECONDS:-600}"
ADMIN_EMAIL="${MIZAN_ADMIN_EMAIL:-admin+mizan-distributed@mizan.local}"
ADMIN_PASSWORD="${MIZAN_ADMIN_PASSWORD:-change-me-distributed}"
USER_EMAIL="${MIZAN_SMOKE_EMAIL:-mizan-user-$(date +%s)@example.test}"
USER_PASSWORD="${MIZAN_SMOKE_PASSWORD:-change-me-distributed}"
MODEL_NAME="${MIZAN_DISTRIBUTED_MODEL:-mock-gpt}"

cleanup() {
  if [[ -n "${DAEMON_PID:-}" ]]; then kill "${DAEMON_PID}" >/dev/null 2>&1 || true; fi
  if [[ -n "${API_PID:-}" ]]; then kill "${API_PID}" >/dev/null 2>&1 || true; fi
  if [[ -n "${MOCK_PID:-}" ]]; then kill "${MOCK_PID}" >/dev/null 2>&1 || true; fi
  if [[ "${CLEANUP_WORK_DIR}" == "1" && -n "${WORK_DIR:-}" ]]; then rm -rf "${WORK_DIR}"; fi
}
trap cleanup EXIT

json_field() {
  python3 -c 'import json,sys; data=json.load(sys.stdin); print(data["'$1'"])'
}

fetch_json() {
  local url="$1"
  shift

  local response
  if ! response="$(curl -fsS -w '\n%{http_code}' "$@" "$url")"; then
    echo "request failed for ${url}" >&2
    exit 1
  fi

  local status="${response##*$'\n'}"
  local body="${response%$'\n'*}"

  if [[ "${status}" -lt 200 || "${status}" -gt 299 ]]; then
    echo "request to ${url} returned HTTP ${status}" >&2
    printf '%s\n' "${body}" >&2
    exit 1
  fi

  if ! printf '%s' "${body}" | python3 -c 'import json,sys; json.load(sys.stdin)' >/dev/null 2>&1; then
    echo "invalid JSON response from ${url}" >&2
    printf '%s\n' "${body}" >&2
    exit 1
  fi

  printf '%s' "${body}"
}

fetch_text() {
  local url="$1"
  shift

  local response
  if ! response="$(curl -fsS -w '\n%{http_code}' "$@" "$url")"; then
    echo "request failed for ${url}" >&2
    exit 1
  fi

  local status="${response##*$'\n'}"
  local body="${response%$'\n'*}"

  if [[ "${status}" -lt 200 || "${status}" -gt 299 ]]; then
    echo "request to ${url} returned HTTP ${status}" >&2
    printf '%s\n' "${body}" >&2
    exit 1
  fi

  printf '%s' "${body}"
}

json_has_model() {
  local payload="$1"
  local model="$2"
  MIZAN_SMOKE_JSON_PAYLOAD="${payload}" python3 - "$model" <<'PY'
import json
import os
import sys

model = sys.argv[1]
payload = json.loads(os.environ["MIZAN_SMOKE_JSON_PAYLOAD"])
ids = [item.get("id") for item in payload.get("data", [])]
raise SystemExit(0 if model in ids else 1)
PY
}

wait_for() {
  local url="$1"
  for _ in $(seq 1 "${WAIT_SECONDS}"); do
    if curl -fsS "$url" >/dev/null 2>&1; then
      return 0
    fi
    sleep 1
  done
  echo "timed out waiting for ${url}" >&2
  return 1
}

wait_for_model() {
  local api_key="$1"
  local url="$2"
  local model="$3"
  for _ in $(seq 1 "${WAIT_SECONDS}"); do
    local models
    models="$(fetch_json "${url}" -H "authorization: Bearer ${api_key}")"
    if json_has_model "${models}" "${model}"; then
      printf '%s\n' "${models}"
      return 0
    fi
    sleep 1
  done
  echo "timed out waiting for model ${model} on ${url}" >&2
  return 1
}

assert_usage_has_rows() {
  local payload="$1"
  local require_daemon_node_id="$2"
  MIZAN_SMOKE_JSON_PAYLOAD="${payload}" python3 - "$require_daemon_node_id" <<'PY'
import json
import os
import sys

payload = json.loads(os.environ["MIZAN_SMOKE_JSON_PAYLOAD"])
rows = payload.get("data") or []
if not rows:
    raise SystemExit(1)
if sys.argv[1] == "1":
    first = rows[0]
    if first.get("daemon_node_id") is None:
        raise SystemExit(1)
print(len(rows))
PY
}

assert_metrics_has_daemon_node() {
  local metrics_payload="$1"
  printf '%s' "${metrics_payload}" | grep -Eq 'daemon_node="[0-9a-f]{8}-[0-9a-f-]{27}"' || return 1
}

echo "Starting mock upstream on ${MOCK_URL}"
python3 scripts/mock-openai.py --port "${MOCK_PORT}" &
MOCK_PID="$!"
wait_for "${MOCK_URL}/v1/models"

echo "Starting mizan-api on ${BASE_URL}"
mkdir -p "${WORK_DIR}"
MIZAN_HTTP_ADDR="127.0.0.1:${API_PORT}" \
DATABASE_URL="sqlite://${WORK_DIR}/mizan-distributed.sqlite3?mode=rwc" \
REDIS_URL="${REDIS_URL:-redis://127.0.0.1:6379/}" \
MIZAN_PROVIDER_SECRET_KEY="${MIZAN_PROVIDER_SECRET_KEY:-dist-provider-secret}" \
MIZAN_ADMIN_EMAIL="${ADMIN_EMAIL}" \
MIZAN_ADMIN_PASSWORD="${ADMIN_PASSWORD}" \
MIZAN_DAEMON_STALE_SECONDS="${MIZAN_DAEMON_STALE_SECONDS:-60}" \
cargo run -p mizan-api >/dev/null 2>&1 &
API_PID="$!"
wait_for "${BASE_URL}/healthz"

echo "Logging in seeded admin"
admin_login_json="$(fetch_json "${BASE_URL}/auth/login" \
  -H 'content-type: application/json' \
  -d "{\"email\":\"${ADMIN_EMAIL}\",\"password\":\"${ADMIN_PASSWORD}\"}" -X POST)"
admin_session_token="$(printf '%s' "${admin_login_json}" | json_field access_token)"

admin_api_key_response="$(fetch_json "${BASE_URL}/api-keys" \
  -H "authorization: Bearer ${admin_session_token}" \
  -H 'content-type: application/json' \
  -d '{"label":"distributed-smoke-admin"}' -X POST)"
admin_api_key="$(printf '%s' "${admin_api_key_response}" | json_field key)"

echo "Creating daemon node"
daemon_node_json="$(fetch_json "${BASE_URL}/admin/daemon-nodes" \
  -H "authorization: Bearer ${admin_api_key}" \
  -H 'content-type: application/json' \
  -d '{"label":"distributed-smoke-node","hostname":"localhost"}' -X POST)"
daemon_token="$(printf '%s' "${daemon_node_json}" | json_field token)"
daemon_token_file="${WORK_DIR}/daemon-token"
printf '%s\n' "${daemon_token}" > "${daemon_token_file}"

daemon_config="${WORK_DIR}/mizan-daemon-distributed.toml"
cat > "${daemon_config}" <<EOF_CFG
control_plane_url = "${BASE_URL}"
daemon_token_path = "${daemon_token_file}"
local_provider_url = "${MOCK_URL}/v1"
provider_family = "openai-compatible"
advertised_models = ["${MODEL_NAME}"]
max_concurrency = 2
health_addr = "127.0.0.1:19180"
heartbeat_interval_seconds = 2
EOF_CFG

echo "Registering daemon with control plane"
cargo run -p mizan-daemon -- register --config "${daemon_config}"

echo "Starting daemon"
cargo run -p mizan-daemon -- run --config "${daemon_config}" >/tmp/mizan-daemon-distributed.log 2>&1 &
DAEMON_PID="$!"

echo "Waiting for daemon-backed model ${MODEL_NAME} in public list"
if ! wait_for_model "${admin_api_key}" "${BASE_URL}/v1/models" "${MODEL_NAME}" >/dev/null; then
  echo "daemon-backed model ${MODEL_NAME} never appeared" >&2
  cat /tmp/mizan-daemon-distributed.log >&2 || true
  exit 1
fi

echo "Registering distributed user"
fetch_json "${BASE_URL}/auth/register" \
  -H 'content-type: application/json' \
  -d "{\"email\":\"${USER_EMAIL}\",\"password\":\"${USER_PASSWORD}\"}" -X POST >/dev/null
user_login_json="$(fetch_json "${BASE_URL}/auth/login" \
  -H 'content-type: application/json' \
  -d "{\"email\":\"${USER_EMAIL}\",\"password\":\"${USER_PASSWORD}\"}" -X POST)"
user_session_token="$(printf '%s' "${user_login_json}" | json_field access_token)"

user_api_key_json="$(fetch_json "${BASE_URL}/api-keys" \
  -H "authorization: Bearer ${user_session_token}" \
  -H 'content-type: application/json' \
  -d '{"label":"distributed-smoke-user"}' -X POST)"
user_api_key="$(printf '%s' "${user_api_key_json}" | json_field key)"

echo "Creating daemon-backed chat request"
fetch_json "${BASE_URL}/v1/chat/completions" \
  -H "authorization: Bearer ${user_api_key}" \
  -H 'content-type: application/json' \
  -d "{\"model\":\"${MODEL_NAME}\",\"messages\":[{\"role\":\"user\",\"content\":\"hello from mizan distributed smoke\"}],\"max_tokens\":32}" -X POST >/dev/null

echo "Verifying usage and metrics"
user_usage_json="$(fetch_json "${BASE_URL}/v1/usage" -H "authorization: Bearer ${user_api_key}")"
assert_usage_has_rows "${user_usage_json}" 0
admin_usage_json="$(fetch_json "${BASE_URL}/admin/usage" -H "authorization: Bearer ${admin_api_key}")"
assert_usage_has_rows "${admin_usage_json}" 1
metrics_payload="$(fetch_text "${BASE_URL}/metrics" -H 'accept: text/plain;version=0.0.4')"
assert_metrics_has_daemon_node "${metrics_payload}"

echo "Distributed smoke passed"
