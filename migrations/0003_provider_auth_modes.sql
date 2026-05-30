ALTER TABLE provider_connections
ADD COLUMN auth_mode TEXT NOT NULL DEFAULT 'api_key';

ALTER TABLE provider_connections
ADD COLUMN auth_config_json TEXT;

CREATE INDEX IF NOT EXISTS idx_provider_connections_auth_mode
ON provider_connections (auth_mode);
