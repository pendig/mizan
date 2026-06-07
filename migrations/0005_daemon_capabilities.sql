ALTER TABLE daemon_nodes ADD COLUMN provider_family TEXT;
ALTER TABLE daemon_nodes ADD COLUMN model_ids_json TEXT NOT NULL DEFAULT '[]';
ALTER TABLE daemon_nodes ADD COLUMN max_concurrency INTEGER;
ALTER TABLE daemon_nodes ADD COLUMN pricing_metadata_json TEXT;
ALTER TABLE daemon_nodes ADD COLUMN region TEXT;
ALTER TABLE daemon_nodes ADD COLUMN labels_json TEXT NOT NULL DEFAULT '[]';
ALTER TABLE daemon_nodes ADD COLUMN health_status TEXT;
ALTER TABLE daemon_nodes ADD COLUMN capability_metadata_json TEXT;
ALTER TABLE daemon_nodes ADD COLUMN disabled INTEGER NOT NULL DEFAULT 0;

CREATE INDEX IF NOT EXISTS idx_daemon_nodes_disabled ON daemon_nodes (disabled);
CREATE INDEX IF NOT EXISTS idx_daemon_nodes_health_status ON daemon_nodes (health_status);
