-- Fixed Assets: Depreciation Runs
-- bd-2s2s: Batch execution records for depreciation posting.
--
-- Each run processes all unposted schedule periods up to a given as_of_date.
-- Runs are idempotent — re-running for the same as_of_date skips already-posted periods.
-- Status lifecycle: pending -> running -> completed | failed

CREATE TYPE fa_run_status AS ENUM (
    'pending',
    'running',
    'completed',
    'failed'
);

CREATE TABLE fa_depreciation_runs (
    id                    UUID          PRIMARY KEY DEFAULT gen_random_uuid(),
    tenant_id             TEXT          NOT NULL,
    -- The effective date: post all schedule periods ending on or before this date
    as_of_date            DATE          NOT NULL,
    status                fa_run_status NOT NULL DEFAULT 'pending',
    -- Execution stats
    assets_processed      INT           NOT NULL DEFAULT 0,
    periods_posted        INT           NOT NULL DEFAULT 0,
    total_depreciation_minor BIGINT     NOT NULL DEFAULT 0,
    currency              TEXT          NOT NULL DEFAULT 'usd',
    -- Error tracking
    error_message         TEXT,
    -- Idempotency: one completed run per tenant per as_of_date
    idempotency_key       UUID          NOT NULL DEFAULT gen_random_uuid(),
    -- Timestamps
    started_at            TIMESTAMPTZ,
    completed_at          TIMESTAMPTZ,
    created_at            TIMESTAMPTZ   NOT NULL DEFAULT NOW(),
    updated_at            TIMESTAMPTZ   NOT NULL DEFAULT NOW(),
    created_by            TEXT,

    CONSTRAINT fa_depr_runs_idempotency_unique
        UNIQUE (idempotency_key)
);

CREATE INDEX idx_fa_depr_runs_tenant
    ON fa_depreciation_runs (tenant_id);

CREATE INDEX idx_fa_depr_runs_tenant_date
    ON fa_depreciation_runs (tenant_id, as_of_date);

CREATE INDEX idx_fa_depr_runs_tenant_status
    ON fa_depreciation_runs (tenant_id, status);
