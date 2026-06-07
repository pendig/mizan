# Migrations

This directory stores SQL migrations for the first schema phase.

Run migrations through the application startup, which runs from `mizan-api` when
`run_from_env` boots.

Current versions:

- `0001_initial.sql`
  - users
  - sessions
  - api_keys
  - provider_connections
  - model_routes
  - wallets
  - credit_ledger
  - usage_events
  - request_logs
  - admin_audit_logs
  - daemon_nodes
