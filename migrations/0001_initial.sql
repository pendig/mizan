CREATE TABLE IF NOT EXISTS users (
    id TEXT PRIMARY KEY,
    email TEXT NOT NULL UNIQUE,
    password_hash TEXT NOT NULL,
    role TEXT NOT NULL DEFAULT 'member',
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL
);

CREATE TABLE IF NOT EXISTS sessions (
    id TEXT PRIMARY KEY,
    user_id TEXT NOT NULL,
    session_token_hash TEXT NOT NULL UNIQUE,
    expires_at TEXT,
    revoked INTEGER NOT NULL DEFAULT 0,
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL
);

CREATE TABLE IF NOT EXISTS api_keys (
    id TEXT PRIMARY KEY,
    user_id TEXT NOT NULL,
    key_hash TEXT NOT NULL UNIQUE,
    label TEXT,
    revoked INTEGER NOT NULL DEFAULT 0,
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL
);

CREATE TABLE IF NOT EXISTS provider_connections (
    id TEXT PRIMARY KEY,
    name TEXT NOT NULL UNIQUE,
    provider_type TEXT NOT NULL,
    base_url TEXT NOT NULL,
    api_key_encrypted TEXT NOT NULL,
    enabled INTEGER NOT NULL DEFAULT 1,
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL
);

CREATE TABLE IF NOT EXISTS model_routes (
    id TEXT PRIMARY KEY,
    provider_connection_id TEXT NOT NULL,
    public_model TEXT NOT NULL UNIQUE,
    upstream_model TEXT NOT NULL,
    max_tokens INTEGER,
    pricing_input_per_1m_tokens INTEGER NOT NULL DEFAULT 0,
    pricing_output_per_1m_tokens INTEGER NOT NULL DEFAULT 0,
    enabled INTEGER NOT NULL DEFAULT 1,
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL
);

CREATE TABLE IF NOT EXISTS wallets (
    id TEXT PRIMARY KEY,
    owner_user_id TEXT NOT NULL,
    balance_microcredits INTEGER NOT NULL DEFAULT 0,
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL
);

CREATE TABLE IF NOT EXISTS credit_ledger (
    id TEXT PRIMARY KEY,
    wallet_id TEXT NOT NULL,
    request_id TEXT,
    request_delta_microcredits INTEGER NOT NULL,
    balance_after_microcredits INTEGER NOT NULL,
    reason TEXT NOT NULL,
    created_at TEXT NOT NULL
);

CREATE TABLE IF NOT EXISTS usage_events (
    id TEXT PRIMARY KEY,
    request_id TEXT NOT NULL,
    user_id TEXT,
    api_key_id TEXT,
    provider_id TEXT,
    route_id TEXT,
    model TEXT NOT NULL,
    usage_prompt_tokens INTEGER NOT NULL DEFAULT 0,
    usage_completion_tokens INTEGER NOT NULL DEFAULT 0,
    usage_total_tokens INTEGER NOT NULL DEFAULT 0,
    usage_estimated INTEGER NOT NULL DEFAULT 0,
    status_code INTEGER NOT NULL,
    latency_ms INTEGER NOT NULL,
    created_at TEXT NOT NULL
);

CREATE TABLE IF NOT EXISTS request_logs (
    id TEXT PRIMARY KEY,
    request_id TEXT NOT NULL,
    user_id TEXT,
    api_key_id TEXT,
    provider_id TEXT,
    route_id TEXT,
    method TEXT NOT NULL,
    path TEXT NOT NULL,
    status_code INTEGER NOT NULL,
    created_at TEXT NOT NULL
);

CREATE TABLE IF NOT EXISTS admin_audit_logs (
    id TEXT PRIMARY KEY,
    actor_user_id TEXT,
    action TEXT NOT NULL,
    entity_type TEXT NOT NULL,
    entity_id TEXT,
    payload_json TEXT,
    created_at TEXT NOT NULL
);

CREATE INDEX IF NOT EXISTS idx_api_keys_user_id ON api_keys (user_id);
CREATE INDEX IF NOT EXISTS idx_usage_events_user_id_created_at ON usage_events (user_id, created_at);
CREATE INDEX IF NOT EXISTS idx_usage_events_api_key_id_created_at ON usage_events (api_key_id, created_at);
CREATE INDEX IF NOT EXISTS idx_usage_events_model_created_at ON usage_events (model, created_at);
CREATE INDEX IF NOT EXISTS idx_credit_ledger_wallet_created_at ON credit_ledger (wallet_id, created_at);
CREATE INDEX IF NOT EXISTS idx_provider_connections_enabled ON provider_connections (enabled);
CREATE INDEX IF NOT EXISTS idx_model_routes_public_model ON model_routes (public_model);
