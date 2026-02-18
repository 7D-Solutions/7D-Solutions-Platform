-- Fixed Assets: Depreciation Schedules
-- bd-2s2s: Period-by-period depreciation plan for each asset.
--
-- Generated when an asset enters service. Each row represents one period's
-- planned depreciation. Actual posting is tracked via depreciation_runs.
-- All monetary fields are BIGINT minor units + TEXT currency.

CREATE TABLE fa_depreciation_schedules (
    id                    UUID        PRIMARY KEY DEFAULT gen_random_uuid(),
    tenant_id             TEXT        NOT NULL,
    asset_id              UUID        NOT NULL REFERENCES fa_assets(id),
    -- Period identification
    period_number         INT         NOT NULL CHECK (period_number >= 1),
    period_start          DATE        NOT NULL,
    period_end            DATE        NOT NULL,
    -- Planned amounts (computed from method + cost + salvage + life)
    depreciation_amount_minor BIGINT  NOT NULL CHECK (depreciation_amount_minor >= 0),
    currency              TEXT        NOT NULL DEFAULT 'usd',
    -- Cumulative planned values at end of this period
    cumulative_depreciation_minor BIGINT NOT NULL CHECK (cumulative_depreciation_minor >= 0),
    remaining_book_value_minor    BIGINT NOT NULL,
    -- Posting status
    is_posted             BOOLEAN     NOT NULL DEFAULT FALSE,
    posted_at             TIMESTAMPTZ,
    posted_by_run_id      UUID,       -- References fa_depreciation_runs(id)
    created_at            TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at            TIMESTAMPTZ NOT NULL DEFAULT NOW(),

    CONSTRAINT fa_depr_schedule_asset_period_unique
        UNIQUE (asset_id, period_number),
    CONSTRAINT fa_depr_schedule_period_order
        CHECK (period_end > period_start)
);

CREATE INDEX idx_fa_depr_schedules_tenant
    ON fa_depreciation_schedules (tenant_id);

CREATE INDEX idx_fa_depr_schedules_asset
    ON fa_depreciation_schedules (asset_id);

-- For depreciation runs: find unposted periods up to a given date
CREATE INDEX idx_fa_depr_schedules_unposted
    ON fa_depreciation_schedules (tenant_id, period_end)
    WHERE is_posted = FALSE;

CREATE INDEX idx_fa_depr_schedules_run
    ON fa_depreciation_schedules (posted_by_run_id)
    WHERE posted_by_run_id IS NOT NULL;
