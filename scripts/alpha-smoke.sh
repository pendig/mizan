#!/usr/bin/env bash
set -euo pipefail

API_PORT="${MIZAN_ALPHA_API_PORT:-18180}"
MOCK_PORT="${MIZAN_ALPHA_MOCK_PORT:-18182}"
WAIT_SECONDS="${MIZAN_ALPHA_WAIT_SECONDS:-600}"
BASE_URL="${MIZAN_BASE_URL:-http://127.0.0.1:${API_PORT}}"
MOCK_URL="${MIZAN_MOCK_BASE_URL:-http://127.0.0.1:${MOCK_PORT}}"
ADMIN_EMAIL="${MIZAN_ADMIN_EMAIL:-admin@mizan.local}"
ADMIN_PASSWORD="${MIZAN_ADMIN_PASSWORD:-change-me-alpha}"
PROVIDER_SECRET="${MIZAN_PROVIDER_SECRET_KEY:-alpha-provider-secret}"
WORK_DIR="${MIZAN_ALPHA_WORK_DIR:-$(mktemp -d)}"

cleanup() {
  if [[ -n "${API_PID:-}" ]]; then kill "${API_PID}" >/dev/null 2>&1 || true; fi
  if [[ -n "${MOCK_PID:-}" ]]; then kill "${MOCK_PID}" >/dev/null 2>&1 || true; fi
}
trap cleanup EXIT

json_field() {
  python3 -c 'import json,sys; data=json.load(sys.stdin); print(data["'$1'"])'
}

wait_for() {
  local url="$1"
  for _ in $(seq 1 "${WAIT_SECONDS}"); do
    if curl -fsS "$url" >/dev/null 2>&1; then return 0; fi
    sleep 1
  done
  echo "Timed out waiting for $url" >&2
  return 1
}

echo "Starting mock upstream on ${MOCK_URL}"
python3 scripts/mock-openai.py --port "${MOCK_PORT}" &
MOCK_PID="$!"
wait_for "${MOCK_URL}/v1/models"

if ! curl -fsS "${BASE_URL}/healthz" >/dev/null 2>&1; then
  echo "Starting mizan-api on ${BASE_URL}"
  mkdir -p "${WORK_DIR}"
  MIZAN_HTTP_ADDR="127.0.0.1:${API_PORT}" \
  DATABASE_URL="sqlite://${WORK_DIR}/mizan-alpha.sqlite3?mode=rwc" \
  REDIS_URL="${REDIS_URL:-redis://127.0.0.1:6379/}" \
  MIZAN_PROVIDER_SECRET_KEY="${PROVIDER_SECRET}" \
  MIZAN_ADMIN_EMAIL="${ADMIN_EMAIL}" \
  MIZAN_ADMIN_PASSWORD="${ADMIN_PASSWORD}" \
  MIZAN_LIMIT_RPM="${MIZAN_LIMIT_RPM:-30}" \
  MIZAN_LIMIT_TPM="${MIZAN_LIMIT_TPM:-1000}" \
  MIZAN_LIMIT_CONCURRENCY="${MIZAN_LIMIT_CONCURRENCY:-4}" \
  cargo run -p mizan-api &
  API_PID="$!"
  wait_for "${BASE_URL}/healthz"
fi

echo "Logging in seeded admin"
login_json="$(curl -fsS -X POST "${BASE_URL}/auth/login" \
  -H 'content-type: application/json' \
  -d "{\"email\":\"${ADMIN_EMAIL}\",\"password\":\"${ADMIN_PASSWORD}\"}")"
session_token="$(printf '%s' "${login_json}" | json_field access_token)"
admin_user_id="$(printf '%s' "${login_json}" | json_field user_id)"

echo "Creating admin virtual API key"
api_key_json="$(curl -fsS -X POST "${BASE_URL}/api-keys" \
  -H "authorization: Bearer ${session_token}" \
  -H 'content-type: application/json' \
  -d '{"label":"alpha-smoke"}')"
api_key="$(printf '%s' "${api_key_json}" | json_field key)"

echo "Creating provider connection and model route"
provider_json="$(curl -fsS -X POST "${BASE_URL}/admin/provider-connections" \
  -H "authorization: Bearer ${api_key}" \
  -H 'content-type: application/json' \
  -d "{\"name\":\"alpha-mock\",\"provider_type\":\"openai-compatible\",\"base_url\":\"${MOCK_URL}\",\"api_key_encrypted\":\"mock-secret\",\"enabled\":true}")"
provider_id="$(printf '%s' "${provider_json}" | json_field id)"

curl -fsS -X POST "${BASE_URL}/admin/model-routes" \
  -H "authorization: Bearer ${api_key}" \
  -H 'content-type: application/json' \
  -d "{\"provider_connection_id\":\"${provider_id}\",\"public_model\":\"alpha-mock\",\"upstream_model\":\"mock-gpt\",\"max_tokens\":128,\"pricing_input_per_1m_tokens\":1000,\"pricing_output_per_1m_tokens\":2000,\"enabled\":true}" >/dev/null

echo "Granting credits"
curl -fsS -X POST "${BASE_URL}/admin/users/${admin_user_id}/credits/grant" \
  -H "authorization: Bearer ${api_key}" \
  -H 'content-type: application/json' \
  -d '{"amount_microcredits":1000000,"reason":"alpha_smoke"}' >/dev/null

echo "Checking models, non-stream chat, stream chat, usage, credits, and metrics"
curl -fsS "${BASE_URL}/v1/models" -H "authorization: Bearer ${api_key}" >/dev/null
echo "Syncing upstream model list"
synced_models="$(
  MODEL_SYNC_BASE_URL="${MOCK_URL}/v1" \
  MODEL_SYNC_API_KEY="alpha-smoke" \
  bash scripts/model-sync.sh --format ids
)"
printf '%s\n' "${synced_models}" | grep -q '^mock-gpt$'
curl -fsS -X POST "${BASE_URL}/v1/chat/completions" \
  -H "authorization: Bearer ${api_key}" \
  -H 'content-type: application/json' \
  -d '{"model":"alpha-mock","messages":[{"role":"user","content":"hello"}],"max_tokens":32}' >/dev/null
curl -fsS -N -X POST "${BASE_URL}/v1/chat/completions" \
  -H "authorization: Bearer ${api_key}" \
  -H 'content-type: application/json' \
  -d '{"model":"alpha-mock","messages":[{"role":"user","content":"stream hello"}],"stream":true,"max_tokens":32}' >/dev/null
curl -fsS "${BASE_URL}/v1/usage" -H "authorization: Bearer ${api_key}" >/dev/null
curl -fsS "${BASE_URL}/v1/credits" -H "authorization: Bearer ${api_key}" >/dev/null
curl -fsS "${BASE_URL}/metrics" | grep -q 'mizan_gateway_requests_total'

echo "Alpha smoke passed"
