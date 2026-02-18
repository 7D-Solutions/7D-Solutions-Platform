-- Fixed Assets: Asset Categories
-- bd-2s2s: Categories define default depreciation parameters for grouped assets.
--
-- Design principles:
--   - All tables scoped to tenant_id for multi-tenancy
--   - Monetary amounts stored as BIGINT minor units + TEXT currency
--   - Depreciation defaults: method, useful_life_months, salvage_pct_bp (basis points)
--   - Tables prefixed fa_ to avoid clashes with other module schemas

CREATE TABLE fa_categories (
    id                    UUID        PRIMARY KEY DEFAULT gen_random_uuid(),
    tenant_id             TEXT        NOT NULL,
    code                  TEXT        NOT NULL,
    name                  TEXT        NOT NULL,
    description           TEXT,
    -- Default depreciation parameters (can be overridden per asset)
    default_method        TEXT        NOT NULL DEFAULT 'straight_line'
        CHECK (default_method IN ('straight_line', 'declining_balance', 'units_of_production', 'none')),
    default_useful_life_months INT   NOT NULL DEFAULT 60
        CHECK (default_useful_life_months > 0),
    -- Salvage value as basis points of original cost (10000 = 100%)
    default_salvage_pct_bp     INT   NOT NULL DEFAULT 0
        CHECK (default_salvage_pct_bp >= 0 AND default_salvage_pct_bp <= 10000),
    -- GL account references for journal postings
    asset_account_ref          TEXT  NOT NULL,   -- Fixed asset (BS)
    depreciation_expense_ref   TEXT  NOT NULL,   -- Depreciation expense (P&L)
    accum_depreciation_ref     TEXT  NOT NULL,   -- Accumulated depreciation (contra-asset BS)
    gain_loss_account_ref      TEXT,             -- Gain/loss on disposal (P&L)
    is_active             BOOLEAN     NOT NULL DEFAULT TRUE,
    created_at            TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at            TIMESTAMPTZ NOT NULL DEFAULT NOW(),

    CONSTRAINT fa_categories_tenant_code_unique
        UNIQUE (tenant_id, code)
);

CREATE INDEX idx_fa_categories_tenant
    ON fa_categories (tenant_id);

CREATE INDEX idx_fa_categories_tenant_active
    ON fa_categories (tenant_id) WHERE is_active = TRUE;
