# Research Notes

These notes summarize patterns from adjacent open-source AI gateway projects and
what Mizan should borrow for a backend-first MVP.

## References Checked

### LiteLLM

Source: https://docs.litellm.ai/

Relevant patterns:

- Unified OpenAI input/output format across 100+ providers.
- Proxy server positioned as a central LLM gateway.
- Auth, logging, cost tracking, and rate limiting as proxy features.
- Virtual keys and spend management.
- Streaming support.

Takeaway:

Mizan should copy the shape of the gateway surface, not the entire feature
set. Start with OpenAI-compatible chat completions, virtual keys, spend/credit
tracking, and rate limits.

### OmniRoute

Source: https://github.com/diegosouzapw/OmniRoute

Relevant patterns:

- One endpoint for coding agents and AI tools.
- Multi-account and multi-provider routing.
- Subscription, API key, cheap, and free provider tiers.
- Fallback routing and health-aware behavior.
- RTK/Caveman-style prompt compression as a later optimization.

Takeaway:

Mizan's differentiator can be controlled access and credit
accounting, but the MVP should avoid becoming a giant router immediately.
Subscription connectors should be isolated and policy-aware because they may
touch provider terms of service.

### RTK

Source: https://github.com/rtk-ai/rtk

Relevant patterns:

- Single Rust binary.
- CLI proxy behavior for AI coding tools.
- Command-output filtering and compression.
- 100+ supported command patterns.
- Token-saving analytics.

Takeaway:

RTK is the right starting base for Mizan's CLI proxy path. Mizan should not
rebuild RTK's command rewriting, command-output filtering, and token-saving
analytics from scratch. RTK is not a full AI gateway, so Mizan still needs new
gateway, auth, provider, metering, and credit-ledger modules around it.

### GoModel

Source: https://gomodel.enterpilot.io/

Relevant patterns:

- Built in Go.
- OpenAI-compatible API.
- Aliases and workflows.
- Per-user usage tracking.
- Audit logs and admin dashboard.
- Simple deployment path that can grow to PostgreSQL, MongoDB, and Redis.

Takeaway:

Even though Mizan should now be Rust-first, GoModel is still useful as product
reference: model aliases, usage tracking, and audit logs should be first-class.

### Inference Gateway

Source: https://github.com/inference-gateway/inference-gateway

Relevant patterns:

- Cloud-native, multi-provider gateway.
- OpenAI-compatible chat completions.
- Streaming support.
- Prometheus/OpenTelemetry metrics.
- Token usage counters for prompt, completion, and total tokens.

Takeaway:

Add metrics early. Usage metering should feed both the credit ledger and
operations visibility.

### Portkey

Source: https://portkey.ai/docs/product/open-source

Relevant patterns:

- Open-source gateway with unified interface.
- Fallbacks, load balancing, retries.
- Separate open-source model pricing database for cost tracking.

Takeaway:

Pricing data should be configurable and eventually importable, but MVP can start
with admin-defined credit prices per model route.

## MVP Design Implications

1. Start OpenAI-compatible.
2. Treat virtual keys as the core user-facing primitive.
3. Track usage in a durable ledger, not only in metrics.
4. Use Redis for hot counters, not as source-of-truth wallet storage.
5. Expose model aliases so users are insulated from upstream provider changes.
6. Keep provider adapters modular.
7. Add subscription/CLI providers later as isolated adapters with policy checks.
8. Use RTK as the Rust CLI proxy base, not as the whole gateway.

## RTK-Inspired Compression

RTK-style compression can be useful later for coding-agent traffic because tool
outputs and command logs can be token-heavy.

Do not include it in MVP. Add it later as a router middleware:

- `compression=off|safe|aggressive`
- Per-model route setting.
- Raw prompt preservation mode.
- Audit field showing original and compressed token estimate.

The first MVP should prove routing, metering, credits, and limits before prompt
mutation enters the system.
