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
- `0002_request_log_foundations.sql`
- `0003_provider_auth_modes.sql`
- `0004_daemon_nodes.sql`
- `0005_daemon_capabilities.sql`
  - daemon heartbeat capability fields and selection indexes
- `0006_dispatch_jobs.sql`
  - daemon dispatch lifecycle records and lookup indexes
