-- Per-invoice platform vs QBO tax divergence log (bd-jsbai)
--
-- Written by the reconciliation worker when a tenant is dual-running:
-- platform-computed tax + QBO AST in parallel. Rows are immutable once inserted;
-- review fields (reviewed_by, reviewed_at, resolution) are the only updates allowed.

CREATE TABLE IF NOT EXISTS ar_tax_reconciliation_log (
    id UUID PRIMARY KEY,
    tenant_id UUID NOT NULL,
    invoice_id UUID NOT NULL,
    platform_tax_cents BIGINT NOT NULL,
    qbo_tax_cents BIGINT NOT NULL,
    divergence_cents BIGINT GENERATED ALWAYS AS (platform_tax_cents - qbo_tax_cents) STORED,
    divergence_pct NUMERIC(6,4) GENERATED ALWAYS AS (
        CASE WHEN qbo_tax_cents = 0 THEN NULL
        ELSE (platform_tax_cents - qbo_tax_cents)::numeric / qbo_tax_cents::numeric
        END
    ) STORED,
    flagged BOOLEAN NOT NULL,
    detected_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    reviewed_by UUID,
    reviewed_at TIMESTAMPTZ,
    resolution TEXT CHECK (resolution IN (
        'accepted_platform', 'accepted_qbo', 'bug_filed', 'config_wrong'
    ))
);

CREATE INDEX IF NOT EXISTS ar_tax_reconciliation_log_tenant_flagged_idx
    ON ar_tax_reconciliation_log (tenant_id, flagged, detected_at DESC);

COMMENT ON TABLE ar_tax_reconciliation_log IS
    'Platform vs QBO tax divergence log; immutable rows with additive review fields';
COMMENT ON COLUMN ar_tax_reconciliation_log.divergence_pct IS
    'NULL when qbo_tax_cents=0 — divergence_cents alone determines flagging in zero-QBO case';
