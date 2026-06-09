# User API Key Onboarding

This flow lets a fresh user create a virtual Mizan API key and use the
OpenAI-compatible gateway without knowing which upstream or daemon host serves
the request.

Set the base URL for your Mizan API:

```sh
export MIZAN_BASE_URL="http://127.0.0.1:18180"
```

Register a user:

```sh
curl -fsS -X POST "${MIZAN_BASE_URL}/auth/register" \
  -H 'content-type: application/json' \
  -d '{"email":"user@example.test","password":"change-me-user"}'
```

Log in and keep the session token:

```sh
export MIZAN_SESSION_TOKEN="$(
  curl -fsS -X POST "${MIZAN_BASE_URL}/auth/login" \
    -H 'content-type: application/json' \
    -d '{"email":"user@example.test","password":"change-me-user"}' \
  | python3 -c 'import json,sys; print(json.load(sys.stdin)["access_token"])'
)"
```

Create a virtual API key. The raw key is returned only once.

```sh
export MIZAN_API_KEY="$(
  curl -fsS -X POST "${MIZAN_BASE_URL}/api-keys" \
    -H "authorization: Bearer ${MIZAN_SESSION_TOKEN}" \
    -H 'content-type: application/json' \
    -d '{"label":"local-dev"}' \
  | python3 -c 'import json,sys; print(json.load(sys.stdin)["key"])'
)"
```

List visible models:

```sh
curl -fsS "${MIZAN_BASE_URL}/v1/models" \
  -H "authorization: Bearer ${MIZAN_API_KEY}" \
  | python3 -m json.tool
```

`/v1/models` returns enabled admin model routes and active healthy daemon
advertisements. Daemon-backed entries expose the model id and provider family
only; daemon node IDs, hostnames, labels, regions, local provider URLs, tokens,
and capability metadata are not returned to users.

Call a model through the OpenAI-compatible gateway:

```sh
curl -fsS -X POST "${MIZAN_BASE_URL}/v1/chat/completions" \
  -H "authorization: Bearer ${MIZAN_API_KEY}" \
  -H 'content-type: application/json' \
  -d '{
    "model": "alpha-mock",
    "messages": [{"role": "user", "content": "hello"}],
    "max_tokens": 32
  }' | python3 -m json.tool
```

For a one-command check against a running Mizan API, use:

```sh
scripts/user-onboarding-smoke.sh
```

Useful overrides:

```sh
MIZAN_BASE_URL="http://127.0.0.1:18180" \
MIZAN_SMOKE_EMAIL="user-$(date +%s)@example.test" \
MIZAN_SMOKE_PASSWORD="change-me-user" \
MIZAN_SMOKE_MODEL="alpha-mock" \
scripts/user-onboarding-smoke.sh
```
