-- Fixed Assets: Asset Register
-- bd-2s2s: Core asset records with acquisition, depreciation config, and status tracking.
--
-- Each row is a single fixed asset per tenant. Carries its own depreciation
-- parameters (defaulted from category but overridable). All monetary fields
-- are BIGINT minor units + TEXT currency.
--
-- Status lifecycle: draft -> active -> fully_depreciated | disposed | impaired

CREATE TYPE fa_asset_status AS ENUM (
    'draft',
    'active',
    'fully_depreciated',
    'disposed',
    'impaired'
);

CREATE TABLE fa_assets (
    id                    UUID           PRIMARY KEY DEFAULT gen_random_uuid(),
    tenant_id             TEXT           NOT NULL,
    category_id           UUID           NOT NULL REFERENCES fa_categories(id),
    asset_tag             TEXT           NOT NULL,       -- Human-readable asset identifier
    name                  TEXT           NOT NULL,
    description           TEXT,
    status                fa_asset_status NOT NULL DEFAULT 'draft',
    -- Acquisition details
    acquisition_date      DATE           NOT NULL,
    in_service_date       DATE,                          -- When depreciation begins
    acquisition_cost_minor BIGINT        NOT NULL CHECK (acquisition_cost_minor >= 0),
    currency              TEXT           NOT NULL DEFAULT 'usd',
    -- Depreciation parameters (override category defaults)
    depreciation_method   TEXT           NOT NULL DEFAULT 'straight_line'
        CHECK (depreciation_method IN ('straight_line', 'declining_balance', 'units_of_production', 'none')),
    useful_life_months    INT            NOT NULL CHECK (useful_life_months > 0),
    salvage_value_minor   BIGINT         NOT NULL DEFAULT 0 CHECK (salvage_value_minor >= 0),
    -- Running totals (updated by depreciation runs and disposals)
    accum_depreciation_minor BIGINT      NOT NULL DEFAULT 0 CHECK (accum_depreciation_minor >= 0),
    net_book_value_minor  BIGINT         NOT NULL DEFAULT 0,
    -- GL account overrides (NULL = use category defaults)
    asset_account_ref     TEXT,
    depreciation_expense_ref TEXT,
    accum_depreciation_ref TEXT,
    -- Location / assignment
    location              TEXT,
    department            TEXT,
    responsible_person    TEXT,
    -- Metadata
    serial_number         TEXT,
    vendor                TEXT,
    purchase_order_ref    TEXT,
    notes                 TEXT,
    created_at            TIMESTAMPTZ    NOT NULL DEFAULT NOW(),
    updated_at            TIMESTAMPTZ    NOT NULL DEFAULT NOW(),

    CONSTRAINT fa_assets_tenant_tag_unique
        UNIQUE (tenant_id, asset_tag)
);

CREATE INDEX idx_fa_assets_tenant
    ON fa_assets (tenant_id);

CREATE INDEX idx_fa_assets_tenant_status
    ON fa_assets (tenant_id, status);

CREATE INDEX idx_fa_assets_category
    ON fa_assets (category_id);

CREATE INDEX idx_fa_assets_tenant_acquisition_date
    ON fa_assets (tenant_id, acquisition_date);

CREATE INDEX idx_fa_assets_tenant_in_service
    ON fa_assets (tenant_id, in_service_date)
    WHERE in_service_date IS NOT NULL;
