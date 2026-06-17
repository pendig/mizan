-- Track daemon-backed usage and request logs without storing prompt/response content.
ALTER TABLE usage_events
    ADD COLUMN daemon_node_id TEXT;

ALTER TABLE request_logs
    ADD COLUMN daemon_node_id TEXT;

CREATE INDEX IF NOT EXISTS idx_usage_events_daemon_node_created_at
    ON usage_events (daemon_node_id, created_at);

CREATE INDEX IF NOT EXISTS idx_request_logs_daemon_node_created_at
    ON request_logs (daemon_node_id, created_at);
