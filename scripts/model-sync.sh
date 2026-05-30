#!/usr/bin/env bash
set -euo pipefail

format="ids"

usage() {
  cat <<'EOF'
Usage:
  MODEL_SYNC_BASE_URL=... MODEL_SYNC_API_KEY=... scripts/model-sync.sh [--format ids|json]

Environment:
  MODEL_SYNC_BASE_URL   OpenAI-compatible base URL, for example https://api.example.com/v1
  MODEL_SYNC_API_KEY    Bearer token for the endpoint
  OPENAI_BASE_URL       Fallback base URL if MODEL_SYNC_BASE_URL is not set
  OPENAI_API_KEY        Fallback API key if MODEL_SYNC_API_KEY is not set

Output:
  ids   Newline-separated model ids, sorted and deduplicated
  json  Raw /v1/models JSON payload
EOF
}

while [[ $# -gt 0 ]]; do
  case "$1" in
    --format)
      format="${2:-}"
      shift 2
      ;;
    --json)
      format="json"
      shift
      ;;
    -h|--help)
      usage
      exit 0
      ;;
    *)
      echo "error: unknown argument: $1" >&2
      usage >&2
      exit 2
      ;;
  esac
done

base_url="${MODEL_SYNC_BASE_URL:-${OPENAI_BASE_URL:-}}"
api_key="${MODEL_SYNC_API_KEY:-${OPENAI_API_KEY:-}}"

if [[ -z "${base_url}" || -z "${api_key}" ]]; then
  echo "error: set MODEL_SYNC_BASE_URL and MODEL_SYNC_API_KEY (or OPENAI_* fallbacks)" >&2
  usage >&2
  exit 1
fi

if [[ "${format}" != "ids" && "${format}" != "json" ]]; then
  echo "error: unsupported format: ${format}" >&2
  exit 2
fi

url="${base_url%/}/models"
response="$(
  curl -fsS --retry 2 --retry-delay 1 \
    -H "Authorization: Bearer ${api_key}" \
    "${url}"
)"

jq -e '.object == "list" and (.data | type == "array")' >/dev/null <<<"${response}"
count="$(jq '.data | length' <<<"${response}")"
echo "synced ${count} models from ${url}" >&2

if [[ "${format}" == "json" ]]; then
  printf '%s\n' "${response}" | jq .
else
  printf '%s\n' "${response}" | jq -r '.data[].id' | sort -u
fi
