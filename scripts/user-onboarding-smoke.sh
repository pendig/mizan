#!/usr/bin/env bash
set -euo pipefail

BASE_URL="${MIZAN_BASE_URL:-http://127.0.0.1:18180}"
EMAIL="${MIZAN_SMOKE_EMAIL:-user-$(date +%s)@example.test}"
PASSWORD="${MIZAN_SMOKE_PASSWORD:-change-me-user}"
MODEL="${MIZAN_SMOKE_MODEL:-}"

json_field() {
  python3 -c 'import json,sys; data=json.load(sys.stdin); print(data["'$1'"])'
}

first_model_id() {
  python3 -c 'import json,sys; data=json.load(sys.stdin); models=data.get("data", []); print(models[0]["id"] if models else "")'
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
  -d '{"label":"user-onboarding-smoke"}')"
api_key="$(printf '%s' "${api_key_json}" | json_field key)"

echo "Listing models"
models_json="$(curl -fsS "${BASE_URL}/v1/models" \
  -H "authorization: Bearer ${api_key}")"

if [[ -z "${MODEL}" ]]; then
  MODEL="$(printf '%s' "${models_json}" | first_model_id)"
fi

if [[ -z "${MODEL}" ]]; then
  echo "No models returned by /v1/models. Configure a model route or healthy daemon model before running the smoke." >&2
  exit 1
fi

echo "Calling model ${MODEL}"
curl -fsS -X POST "${BASE_URL}/v1/chat/completions" \
  -H "authorization: Bearer ${api_key}" \
  -H 'content-type: application/json' \
  -d "{\"model\":\"${MODEL}\",\"messages\":[{\"role\":\"user\",\"content\":\"hello\"}],\"max_tokens\":32}" >/dev/null

echo "User onboarding smoke passed for ${EMAIL} with model ${MODEL}"
