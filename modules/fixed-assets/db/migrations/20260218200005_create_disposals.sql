-- Fixed Assets: Disposals & Impairments
-- bd-2s2s: Records asset disposal events (sale, scrap, impairment write-down).
--
-- Each disposal transitions the asset to 'disposed' or 'impaired' status
-- and records the financial impact (proceeds, gain/loss).
-- All monetary fields are BIGINT minor units + TEXT currency.

CREATE TYPE fa_disposal_type AS ENUM (
    'sale',
    'scrap',
    'impairment',
    'write_off',
    'transfer'
);

CREATE TABLE fa_disposals (
    id                    UUID              PRIMARY KEY DEFAULT gen_random_uuid(),
    tenant_id             TEXT              NOT NULL,
    asset_id              UUID              NOT NULL REFERENCES fa_assets(id),
    disposal_type         fa_disposal_type  NOT NULL,
    disposal_date         DATE              NOT NULL,
    -- Financial details at time of disposal
    net_book_value_at_disposal_minor BIGINT NOT NULL,
    proceeds_minor        BIGINT            NOT NULL DEFAULT 0,
    gain_loss_minor       BIGINT            NOT NULL DEFAULT 0,
    currency              TEXT              NOT NULL DEFAULT 'usd',
    -- Context
    reason                TEXT,
    buyer                 TEXT,             -- For sales
    reference             TEXT,             -- External doc reference
    -- GL posting reference
    journal_entry_ref     TEXT,             -- GL journal entry ID once posted
    is_posted             BOOLEAN           NOT NULL DEFAULT FALSE,
    posted_at             TIMESTAMPTZ,
    -- Metadata
    created_by            TEXT,
    approved_by           TEXT,
    created_at            TIMESTAMPTZ       NOT NULL DEFAULT NOW(),
    updated_at            TIMESTAMPTZ       NOT NULL DEFAULT NOW()
);

CREATE INDEX idx_fa_disposals_tenant
    ON fa_disposals (tenant_id);

CREATE INDEX idx_fa_disposals_asset
    ON fa_disposals (asset_id);

CREATE INDEX idx_fa_disposals_tenant_date
    ON fa_disposals (tenant_id, disposal_date);

CREATE INDEX idx_fa_disposals_tenant_type
    ON fa_disposals (tenant_id, disposal_type);
