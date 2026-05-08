# RTK Base Strategy

## Decision

Use RTK as the Rust implementation base for developer-tool proxying and
token-saving command-output filtering. Do not build the CLI proxy from scratch.
Build Mizan's AI gateway modules as new Rust crates around that RTK-backed
layer.

RTK is a strong fit because it is already a single Rust binary focused on CLI
proxy behavior, command-output filtering, hook-based AI tool integration, and
token-saving analytics. Mizan's core product needs a wider backend surface:
HTTP gateway, auth, provider registry, virtual keys, Redis limits, usage
metering, and credit ledger.

## What To Reuse From RTK

Reuse or adapt first:

- CLI proxy patterns.
- Command detection.
- Output filtering.
- Grouping, truncation, and deduplication strategies.
- Token-saving analytics ideas.
- Single-binary release discipline.
- AI tool integration patterns for Codex, Gemini CLI, Claude Code, and similar
  tools.

Do not create a separate Mizan CLI proxy implementation until there is a proven
gap that RTK cannot cover.

## What To Build Separately

Build new Mizan crates for:

- OpenAI-compatible HTTP gateway.
- Admin API.
- User API.
- Provider adapters.
- API key authentication.
- Wallet and credit ledger.
- PostgreSQL migrations.
- Redis rate limits and concurrency leases.
- Metrics and audit logs.

## Proposed Cargo Workspace

```text
crates/mizan-api/          HTTP API and route wiring
crates/mizan-core/         Shared domain types and errors
crates/mizan-gateway/      OpenAI-compatible proxy
crates/mizan-providers/    Provider adapters
crates/mizan-metering/     Usage events and token accounting
crates/mizan-wallet/       Credit ledger and wallet policy
crates/mizan-limits/       Redis rate limits and concurrency
crates/mizan-rtk/          RTK-derived compression/proxy layer
crates/mizan-cli/          CLI entrypoint and local admin helpers
```

## MVP Integration Order

1. Bring RTK into the workspace as `mizan-rtk`.
2. Keep RTK behavior working as the baseline CLI proxy.
3. Add `mizan-api`, `mizan-core`, and `mizan-gateway`.
4. Add Postgres, Redis, auth, provider routes, and usage metering.
5. Wire the RTK-backed CLI proxy to Mizan config and usage tracking.
6. Expose compression/filtering policy per model route later.

## Fork vs Inspiration

Recommended first path:

- Start a fresh Mizan Rust workspace.
- Import RTK as the first `mizan-rtk` base, either by fork, subtree, or carefully
  copied modules.
- Preserve upstream behavior before adding Mizan-specific integration.
- Copy/adapt only the modules that are license-compatible and genuinely needed.
- Keep RTK attribution in docs and source headers where required.

Avoid for MVP:

- Rewriting RTK's CLI proxy from scratch.
- Turning RTK itself into the full HTTP gateway without clear module boundaries.
- Making compression mandatory before usage/credit accounting is proven.

## License Check

Before copying RTK source files, verify the exact license files and headers in
the version being used. If there is any mismatch between README badges and the
actual license file, treat the license file as authoritative and document the
decision.
