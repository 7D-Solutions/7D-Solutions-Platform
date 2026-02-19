-- cp_retention_policies: per-tenant data retention configuration
-- Phase 34: Hardening / Launch Readiness
--
-- Stores retention knobs per tenant. Tombstone path uses data_tombstoned_at
-- as the authoritative marker; export_ready_at is set after a successful export.
--
-- Default retention is 7 years (2555 days) with 30-day tombstone window.

CREATE TABLE cp_retention_policies (
    tenant_id UUID PRIMARY KEY REFERENCES tenants(tenant_id) ON DELETE CASCADE,

    -- How many days after deletion tenant data must be retained.
    -- Default: 2555 days (~7 years) to cover common regulatory requirements.
    data_retention_days INT NOT NULL DEFAULT 2555
        CHECK (data_retention_days > 0),

    -- Output format for tenant data exports. Only 'jsonl' is supported today.
    export_format TEXT NOT NULL DEFAULT 'jsonl'
        CHECK (export_format IN ('jsonl')),

    -- Days after export_ready_at before physical data may be tombstoned.
    -- Provides a grace window for the data subject to review their export.
    auto_tombstone_days INT NOT NULL DEFAULT 30
        CHECK (auto_tombstone_days >= 0),

    -- Set when the first successful export artifact has been produced.
    -- NULL = no export produced yet.
    export_ready_at TIMESTAMPTZ,

    -- Set when the tenant's data has been tombstoned (soft-purged).
    -- NULL = data still retained. Non-null = tombstoned; physical purge
    -- may proceed once data_retention_days has elapsed from deleted_at.
    data_tombstoned_at TIMESTAMPTZ,

    created_at TIMESTAMPTZ NOT NULL DEFAULT CURRENT_TIMESTAMP,
    updated_at TIMESTAMPTZ NOT NULL DEFAULT CURRENT_TIMESTAMP
);

COMMENT ON TABLE cp_retention_policies IS 'Per-tenant data retention configuration; governs export and tombstone lifecycle';
COMMENT ON COLUMN cp_retention_policies.data_retention_days IS 'Days data must be retained after tenant deletion (default 2555 = ~7 years)';
COMMENT ON COLUMN cp_retention_policies.export_format IS 'Format for tenant data export artifacts; currently only jsonl';
COMMENT ON COLUMN cp_retention_policies.auto_tombstone_days IS 'Grace window in days between export_ready_at and permitted tombstone';
COMMENT ON COLUMN cp_retention_policies.export_ready_at IS 'When the first deterministic export artifact was produced; NULL if never exported';
COMMENT ON COLUMN cp_retention_policies.data_tombstoned_at IS 'When tenant data was tombstoned; NULL if not yet tombstoned';
