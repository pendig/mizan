CREATE TABLE IF NOT EXISTS dispatch_jobs (
    id TEXT PRIMARY KEY,
    request_id TEXT NOT NULL,
    node_id TEXT NOT NULL,
    model TEXT NOT NULL,
    status TEXT NOT NULL,
    request_json TEXT NOT NULL,
    response_json TEXT,
    error_code TEXT,
    error_message TEXT,
    leased_at TEXT,
    started_at TEXT,
    completed_at TEXT,
    timeout_at TEXT NOT NULL,
    latency_ms INTEGER,
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL,
    FOREIGN KEY (node_id) REFERENCES daemon_nodes (id) ON DELETE CASCADE
);

CREATE INDEX IF NOT EXISTS idx_dispatch_jobs_node_status ON dispatch_jobs (node_id, status, created_at);
CREATE INDEX IF NOT EXISTS idx_dispatch_jobs_request_id ON dispatch_jobs (request_id);
CREATE INDEX IF NOT EXISTS idx_dispatch_jobs_status_timeout ON dispatch_jobs (status, timeout_at);
