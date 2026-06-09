#!/usr/bin/env bash
set -euo pipefail

BASE_URL="${MIZAN_BASE_URL:-http://127.0.0.1:18180}"
EMAIL="${MIZAN_SMOKE_EMAIL:-mizan-user-$(date +%s)@example.test}"
PASSWORD="${MIZAN_SMOKE_PASSWORD:-change-me-distributed}"
EXPECTED_MODEL="${MIZAN_DISTRIBUTED_MODEL:-}"
RUN_CHAT="${MIZAN_DISTRIBUTED_RUN_CHAT:-0}"

json_field() {
  python3 -c 'import json,sys; data=json.load(sys.stdin); print(data["'$1'"])'
}

echo "Registering ${EMAIL}"
curl -fsS -X POST "${BASE_URL}/auth/register" \
  -H 'content-type: application/json' \
  -d "{\"email\":\"${EMAIL}\",\"password\":\"${PASSWORD}\"}" >/dev/null

echo "Logging in"
login_json="$(curl -fsS -X POST "${BASE_URL}/auth/login" \
  -H 'content-type: application/json' \
  -d "{\"email\":\"${EMAIL}\",\"password\":\"${PASSWORD}\"}")"
session_token="$(printf '%s' "${login_json}" | json_field access_token)"

echo "Creating virtual API key"
api_key_json="$(curl -fsS -X POST "${BASE_URL}/api-keys" \
  -H "authorization: Bearer ${session_token}" \
  -H 'content-type: application/json' \
  -d '{"label":"distributed-onboarding-smoke"}')"
api_key="$(printf '%s' "${api_key_json}" | json_field key)"

echo "Listing public models"
models_json="$(curl -fsS "${BASE_URL}/v1/models" \
  -H "authorization: Bearer ${api_key}")"
printf '%s\n' "${models_json}" | python3 -m json.tool >/dev/null

if [[ -n "${EXPECTED_MODEL}" ]]; then
  printf '%s\n' "${models_json}" | python3 -c '
import json, sys
payload = json.load(sys.stdin)
expected = sys.argv[1]
ids = [item.get("id") for item in payload.get("data", [])]
if expected not in ids:
    raise SystemExit(f"expected model {expected!r} in /v1/models, got {ids!r}")
' "${EXPECTED_MODEL}"
fi

if [[ "${RUN_CHAT}" == "1" ]]; then
  if [[ -z "${EXPECTED_MODEL}" ]]; then
    echo "Set MIZAN_DISTRIBUTED_MODEL before MIZAN_DISTRIBUTED_RUN_CHAT=1" >&2
    exit 1
  fi

  echo "Calling ${EXPECTED_MODEL}"
  curl -fsS -X POST "${BASE_URL}/v1/chat/completions" \
    -H "authorization: Bearer ${api_key}" \
    -H 'content-type: application/json' \
    -d "{\"model\":\"${EXPECTED_MODEL}\",\"messages\":[{\"role\":\"user\",\"content\":\"hello from Mizan distributed onboarding smoke\"}],\"max_tokens\":32}" >/dev/null
fi

echo "Distributed onboarding smoke passed"
