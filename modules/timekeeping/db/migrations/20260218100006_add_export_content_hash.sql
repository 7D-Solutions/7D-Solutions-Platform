-- Add content_hash for deterministic idempotent export runs.
-- SHA-256 hex of the canonical CSV + JSON output.
ALTER TABLE tk_export_runs ADD COLUMN content_hash VARCHAR(64);

-- Unique constraint: same app + type + period + hash = idempotent replay
CREATE UNIQUE INDEX tk_export_runs_idempotent
    ON tk_export_runs(app_id, export_type, period_start, period_end, content_hash)
    WHERE content_hash IS NOT NULL;
