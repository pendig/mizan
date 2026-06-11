CREATE TABLE IF NOT EXISTS dispatch_jobs (
    id TEXT PRIMARY KEY,
    request_id TEXT NOT NULL,
    node_id TEXT NOT NULL,
    user_id TEXT,
    api_key_id TEXT,
    model TEXT NOT NULL,
    status TEXT NOT NULL,
    request_json TEXT NOT NULL,
    response_json TEXT,
    error_code TEXT,
    error_message TEXT,
    leased_at TEXT,
    completed_at TEXT,
    deadline_at TEXT NOT NULL,
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL,
    FOREIGN KEY (node_id) REFERENCES daemon_nodes (id) ON DELETE CASCADE,
    FOREIGN KEY (user_id) REFERENCES users (id) ON DELETE SET NULL,
    FOREIGN KEY (api_key_id) REFERENCES api_keys (id) ON DELETE SET NULL
);

CREATE INDEX IF NOT EXISTS idx_dispatch_jobs_node_status ON dispatch_jobs (node_id, status, created_at);
CREATE INDEX IF NOT EXISTS idx_dispatch_jobs_request_id ON dispatch_jobs (request_id);
CREATE INDEX IF NOT EXISTS idx_dispatch_jobs_status_deadline ON dispatch_jobs (status, deadline_at);
