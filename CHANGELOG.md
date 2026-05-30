# Changelog

All notable changes to Mizan will be documented in this file.

The format is intended to stay simple while the project is in bootstrap.

## Unreleased

- No unreleased changes yet.

## v0.1.0 - 2026-05-31

- Add backend/API gateway release surface for `/v1/models`,
  `/v1/chat/completions`, and `/v1/responses`.
- Add SQLite-first storage with PostgreSQL-ready SQL preparation.
- Add user auth, admin seed account, virtual API keys, provider connections,
  model routes, usage reads, credit reads, and admin credit grants.
- Add usage metering, credit ledger writes, wallet balance handling, and Redis
  RPM/concurrency limit enforcement.
- Add request log and admin audit log foundations with centralized gateway
  completion logging.
- Add RTK baseline crate and CLI proxy/filter tooling.
- Add provider auth-mode metadata for `api_key`, `subscription_cli`, and
  `browser_session` registration.
- Add model-sync and alpha smoke helpers for OpenAI-compatible providers.
- Add OSS project docs, license, contribution guide, security policy, and
  release readiness docs.
