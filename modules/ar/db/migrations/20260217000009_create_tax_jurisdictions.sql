-- Tax Jurisdictions & Rules (Phase 23b, bd-360)
--
-- Jurisdiction resolution: given a (country, state, postal_code) tuple and
-- a product tax_code, resolve the applicable tax rules (rate, flags, effective dates).
--
-- Tables:
--   ar_tax_jurisdictions — canonical jurisdiction records keyed by region
--   ar_tax_rules         — rate rules per jurisdiction, with effective date ranges
--   ar_invoice_tax_snapshots — persisted resolved jurisdiction snapshot per invoice
--
-- Design invariants:
--   1. Resolution is deterministic: same (address, product_code, as_of_date) → same rules
--   2. Every invoice calculation persists its resolved snapshot for replayability
--   3. Effective-date windows are non-overlapping per (jurisdiction_id, tax_code)

-- ============================================================================
-- ar_tax_jurisdictions
-- ============================================================================

CREATE TABLE IF NOT EXISTS ar_tax_jurisdictions (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    app_id VARCHAR(50) NOT NULL,
    -- ISO 3166-1 alpha-2 country code
    country_code VARCHAR(2) NOT NULL,
    -- ISO 3166-2 subdivision code (state/province), nullable for country-level
    state_code VARCHAR(10),
    -- Postal/ZIP code pattern (nullable = applies to all postal codes in region)
    postal_pattern VARCHAR(20),
    -- Human-readable jurisdiction name (e.g. "California State Tax")
    jurisdiction_name VARCHAR(255) NOT NULL,
    -- Tax type: sales_tax, vat, gst, excise, etc.
    tax_type VARCHAR(50) NOT NULL DEFAULT 'sales_tax',
    -- Whether this jurisdiction is currently active
    is_active BOOLEAN NOT NULL DEFAULT TRUE,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    -- Uniqueness: one jurisdiction per (app_id, country, state, postal, tax_type)
    CONSTRAINT uq_tax_jurisdiction_region UNIQUE (app_id, country_code, state_code, postal_pattern, tax_type)
);

CREATE INDEX IF NOT EXISTS idx_tax_jurisdictions_lookup
    ON ar_tax_jurisdictions(app_id, country_code, state_code, is_active);

-- ============================================================================
-- ar_tax_rules
-- ============================================================================

CREATE TABLE IF NOT EXISTS ar_tax_rules (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    jurisdiction_id UUID NOT NULL REFERENCES ar_tax_jurisdictions(id),
    app_id VARCHAR(50) NOT NULL,
    -- Product tax code (e.g. "SW050000" for SaaS). NULL = default rule for jurisdiction.
    tax_code VARCHAR(100),
    -- Tax rate as decimal (0.0–1.0), e.g. 0.085 for 8.5%
    rate NUMERIC(10, 6) NOT NULL,
    -- Flat amount in minor currency units (additional to rate, usually 0)
    flat_amount_minor BIGINT NOT NULL DEFAULT 0,
    -- Whether this product is exempt in this jurisdiction
    is_exempt BOOLEAN NOT NULL DEFAULT FALSE,
    -- Effective date range (inclusive start, exclusive end)
    effective_from DATE NOT NULL,
    effective_to DATE, -- NULL = no end date (currently effective)
    -- Priority for rule resolution (higher = more specific, takes precedence)
    priority INT NOT NULL DEFAULT 0,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    -- No overlapping effective ranges for the same (jurisdiction, tax_code)
    CONSTRAINT uq_tax_rule_effective UNIQUE (jurisdiction_id, tax_code, effective_from)
);

CREATE INDEX IF NOT EXISTS idx_tax_rules_resolution
    ON ar_tax_rules(jurisdiction_id, effective_from, effective_to);

CREATE INDEX IF NOT EXISTS idx_tax_rules_app
    ON ar_tax_rules(app_id);

-- ============================================================================
-- ar_invoice_tax_snapshots
-- ============================================================================

CREATE TABLE IF NOT EXISTS ar_invoice_tax_snapshots (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    app_id VARCHAR(50) NOT NULL,
    invoice_id VARCHAR(255) NOT NULL,
    -- The resolved jurisdiction at the time of invoice calculation
    jurisdiction_id UUID NOT NULL REFERENCES ar_tax_jurisdictions(id),
    jurisdiction_name VARCHAR(255) NOT NULL,
    country_code VARCHAR(2) NOT NULL,
    state_code VARCHAR(10),
    -- Input address used for resolution (serialized for audit trail)
    ship_to_address JSONB NOT NULL,
    -- Per-line resolved rules snapshot
    resolved_rules JSONB NOT NULL,
    -- Total tax calculated from these rules
    total_tax_minor BIGINT NOT NULL,
    -- The tax_code used for resolution (if any)
    tax_code VARCHAR(100),
    -- Rate that was applied
    applied_rate NUMERIC(10, 6) NOT NULL,
    -- Deterministic hash of (address + tax_code + as_of_date) for cache validation
    resolution_hash VARCHAR(64) NOT NULL,
    -- Date the rules were resolved as-of (for effective date lookups)
    resolved_as_of DATE NOT NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    -- One snapshot per (app_id, invoice_id) — recalculation replaces
    CONSTRAINT uq_invoice_tax_snapshot UNIQUE (app_id, invoice_id)
);

CREATE INDEX IF NOT EXISTS idx_invoice_tax_snapshots_lookup
    ON ar_invoice_tax_snapshots(app_id, invoice_id);
