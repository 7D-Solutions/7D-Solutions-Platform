-- Scheduled Reconciliation Runs (bd-1kl)
--
-- Tracks reconciliation run windows for scheduled execution.
-- Workers claim pending runs via FOR UPDATE SKIP LOCKED.
-- Duplicate windows per tenant are deduped via UNIQUE constraint.

CREATE TABLE IF NOT EXISTS ar_recon_scheduled_runs (
    id SERIAL PRIMARY KEY,
    scheduled_run_id UUID NOT NULL UNIQUE,
    app_id VARCHAR(50) NOT NULL,
    window_start TIMESTAMP NOT NULL,
    window_end TIMESTAMP NOT NULL,
    status VARCHAR(20) NOT NULL DEFAULT 'pending',
    recon_run_id UUID,
    worker_id VARCHAR(100),
    claimed_at TIMESTAMP,
    completed_at TIMESTAMP,
    error_message TEXT,
    match_count INTEGER,
    exception_count INTEGER,
    correlation_id VARCHAR(255),
    created_at TIMESTAMP NOT NULL DEFAULT CURRENT_TIMESTAMP,

    -- Dedup: one run per window per tenant
    CONSTRAINT uq_recon_scheduled_window UNIQUE(app_id, window_start, window_end)
);

CREATE INDEX IF NOT EXISTS ar_recon_scheduled_runs_app_id ON ar_recon_scheduled_runs(app_id);
CREATE INDEX IF NOT EXISTS ar_recon_scheduled_runs_status ON ar_recon_scheduled_runs(status);
CREATE INDEX IF NOT EXISTS ar_recon_scheduled_runs_window ON ar_recon_scheduled_runs(app_id, window_start, window_end);
