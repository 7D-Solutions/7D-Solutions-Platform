-- Consolidation Module: Configuration Tables
-- bd-2ye7: entity groups, COA mappings, elimination rules, FX translation policies
--
-- Design principles:
--   - All config is scoped to (tenant_id, group_id)
--   - Audit fields (created_at, updated_at) on every config table
--   - Monetary amounts stored as BIGINT minor units + TEXT currency (in caches)
--   - Tables prefixed csl_ to avoid clashes with source-module schemas
--   - Ownership percentage stored as basis points (10000 = 100%)

-- ============================================================
-- CONSOLIDATION GROUPS
-- ============================================================
-- A consolidation group defines a set of entities whose financials
-- are combined into a single consolidated view. Each group has one
-- reporting currency; all entity balances are translated into it.

CREATE TABLE csl_groups (
    id                    UUID        PRIMARY KEY DEFAULT gen_random_uuid(),
    tenant_id             TEXT        NOT NULL,
    name                  TEXT        NOT NULL,
    description           TEXT,
    reporting_currency    TEXT        NOT NULL,   -- ISO 4217 (group-level target currency)
    fiscal_year_end_month SMALLINT    NOT NULL DEFAULT 12
        CHECK (fiscal_year_end_month BETWEEN 1 AND 12),
    is_active             BOOLEAN     NOT NULL DEFAULT TRUE,
    created_at            TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at            TIMESTAMPTZ NOT NULL DEFAULT NOW(),

    CONSTRAINT csl_groups_unique_name
        UNIQUE (tenant_id, name)
);

CREATE INDEX idx_csl_groups_tenant
    ON csl_groups (tenant_id);

CREATE INDEX idx_csl_groups_tenant_active
    ON csl_groups (tenant_id) WHERE is_active = TRUE;

COMMENT ON TABLE csl_groups IS
    'Consolidation groups: defines a set of entities consolidated into one reporting view.';

-- ============================================================
-- GROUP ENTITIES
-- ============================================================
-- Each entity maps to a tenant_id in the platform. The entity's
-- functional currency may differ from the group reporting currency,
-- requiring FX translation during consolidation.

CREATE TABLE csl_group_entities (
    id                    UUID        PRIMARY KEY DEFAULT gen_random_uuid(),
    group_id              UUID        NOT NULL REFERENCES csl_groups(id) ON DELETE CASCADE,
    entity_tenant_id      TEXT        NOT NULL,   -- tenant_id of the subsidiary
    entity_name           TEXT        NOT NULL,
    functional_currency   TEXT        NOT NULL,   -- ISO 4217 (entity's local currency)
    -- Ownership percentage in basis points (10000 = 100%)
    ownership_pct_bp      INT         NOT NULL DEFAULT 10000
        CHECK (ownership_pct_bp > 0 AND ownership_pct_bp <= 10000),
    consolidation_method  TEXT        NOT NULL DEFAULT 'full'
        CHECK (consolidation_method IN ('full', 'proportional', 'equity')),
    is_active             BOOLEAN     NOT NULL DEFAULT TRUE,
    created_at            TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at            TIMESTAMPTZ NOT NULL DEFAULT NOW(),

    CONSTRAINT csl_group_entities_unique
        UNIQUE (group_id, entity_tenant_id)
);

CREATE INDEX idx_csl_group_entities_group
    ON csl_group_entities (group_id);

CREATE INDEX idx_csl_group_entities_entity
    ON csl_group_entities (entity_tenant_id);

COMMENT ON TABLE csl_group_entities IS
    'Group member entities: each maps a tenant_id to a consolidation group with ownership and method.';

-- ============================================================
-- COA MAPPINGS (Chart of Accounts)
-- ============================================================
-- Maps each entity's local account codes to group-level consolidated
-- account codes. The consolidated TB builder uses these to reclassify
-- entity-level balances into a uniform group chart of accounts.

CREATE TABLE csl_coa_mappings (
    id                    UUID        PRIMARY KEY DEFAULT gen_random_uuid(),
    group_id              UUID        NOT NULL REFERENCES csl_groups(id) ON DELETE CASCADE,
    entity_tenant_id      TEXT        NOT NULL,
    source_account_code   TEXT        NOT NULL,   -- entity-level account code
    target_account_code   TEXT        NOT NULL,   -- group-level consolidated account code
    target_account_name   TEXT,                    -- optional label for display
    created_at            TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at            TIMESTAMPTZ NOT NULL DEFAULT NOW(),

    CONSTRAINT csl_coa_mappings_unique
        UNIQUE (group_id, entity_tenant_id, source_account_code)
);

CREATE INDEX idx_csl_coa_mappings_group
    ON csl_coa_mappings (group_id);

CREATE INDEX idx_csl_coa_mappings_group_entity
    ON csl_coa_mappings (group_id, entity_tenant_id);

COMMENT ON TABLE csl_coa_mappings IS
    'COA mappings: maps entity-level account codes to group-level consolidated accounts.';

-- ============================================================
-- ELIMINATION RULES
-- ============================================================
-- Defines rules for generating elimination journals during consolidation.
-- Each rule specifies which account codes to debit/credit when eliminating
-- intercompany balances (e.g. intercompany receivable vs payable).

CREATE TABLE csl_elimination_rules (
    id                    UUID        PRIMARY KEY DEFAULT gen_random_uuid(),
    group_id              UUID        NOT NULL REFERENCES csl_groups(id) ON DELETE CASCADE,
    rule_name             TEXT        NOT NULL,
    rule_type             TEXT        NOT NULL
        CHECK (rule_type IN (
            'intercompany_revenue_cost',
            'intercompany_receivable_payable',
            'intercompany_investment_equity',
            'custom'
        )),
    -- Group-level consolidated account codes for the elimination journal
    debit_account_code    TEXT        NOT NULL,
    credit_account_code   TEXT        NOT NULL,
    description           TEXT,
    is_active             BOOLEAN     NOT NULL DEFAULT TRUE,
    created_at            TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at            TIMESTAMPTZ NOT NULL DEFAULT NOW(),

    CONSTRAINT csl_elimination_rules_unique_name
        UNIQUE (group_id, rule_name)
);

CREATE INDEX idx_csl_elimination_rules_group
    ON csl_elimination_rules (group_id);

CREATE INDEX idx_csl_elimination_rules_group_active
    ON csl_elimination_rules (group_id) WHERE is_active = TRUE;

COMMENT ON TABLE csl_elimination_rules IS
    'Elimination rules: define debit/credit pairs for intercompany elimination journals.';

-- ============================================================
-- FX TRANSLATION POLICIES
-- ============================================================
-- Per-entity policy for how to translate functional-currency balances
-- into the group reporting currency. Different financial statement
-- sections use different rate types (e.g. BS at closing rate,
-- P&L at average rate, equity at historical rate).

CREATE TABLE csl_fx_policies (
    id                    UUID        PRIMARY KEY DEFAULT gen_random_uuid(),
    group_id              UUID        NOT NULL REFERENCES csl_groups(id) ON DELETE CASCADE,
    entity_tenant_id      TEXT        NOT NULL,
    -- Rate type for balance sheet items: closing (spot at period end)
    bs_rate_type          TEXT        NOT NULL DEFAULT 'closing'
        CHECK (bs_rate_type IN ('closing', 'average', 'historical')),
    -- Rate type for income statement items: average over the period
    pl_rate_type          TEXT        NOT NULL DEFAULT 'average'
        CHECK (pl_rate_type IN ('closing', 'average', 'historical')),
    -- Rate type for equity items: historical (rate at time of investment)
    equity_rate_type      TEXT        NOT NULL DEFAULT 'historical'
        CHECK (equity_rate_type IN ('closing', 'average', 'historical')),
    -- Where to source FX rates (references GL fx_rates table)
    fx_rate_source        TEXT        NOT NULL DEFAULT 'gl',
    created_at            TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at            TIMESTAMPTZ NOT NULL DEFAULT NOW(),

    CONSTRAINT csl_fx_policies_unique
        UNIQUE (group_id, entity_tenant_id)
);

CREATE INDEX idx_csl_fx_policies_group
    ON csl_fx_policies (group_id);

COMMENT ON TABLE csl_fx_policies IS
    'FX translation policies: defines rate types (closing/average/historical) per entity per statement section.';
