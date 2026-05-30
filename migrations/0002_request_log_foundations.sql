-- Add request log fields required for issue #7 observability foundations.
ALTER TABLE request_logs
    ADD COLUMN latency_ms INTEGER NOT NULL DEFAULT 0;

ALTER TABLE request_logs
    ADD COLUMN route TEXT;

ALTER TABLE request_logs
    ADD COLUMN provider TEXT;

ALTER TABLE request_logs
    ADD COLUMN error_code TEXT;
