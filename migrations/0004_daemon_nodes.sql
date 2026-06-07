CREATE TABLE IF NOT EXISTS daemon_nodes (
    id TEXT PRIMARY KEY,
    host_user_id TEXT,
    label TEXT,
    hostname TEXT,
    public_key TEXT,
    token_hash TEXT NOT NULL UNIQUE,
    status TEXT NOT NULL DEFAULT 'pending',
    revoked INTEGER NOT NULL DEFAULT 0,
    last_seen_at TEXT,
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL,
    FOREIGN KEY (host_user_id) REFERENCES users (id) ON DELETE SET NULL
);

CREATE INDEX IF NOT EXISTS idx_daemon_nodes_host_user_id ON daemon_nodes (host_user_id);
CREATE INDEX IF NOT EXISTS idx_daemon_nodes_status ON daemon_nodes (status);
CREATE INDEX IF NOT EXISTS idx_daemon_nodes_last_seen_at ON daemon_nodes (last_seen_at);
CREATE INDEX IF NOT EXISTS idx_daemon_nodes_token_hash ON daemon_nodes (token_hash);
