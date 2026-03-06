-- Phase E: Maintenance ↔ Production integration (bd-1kw8s)
-- 1. Workcenter projection (read model from production workcenter events)
-- 2. Processed events dedup table for consumer idempotency

-- ============================================================================
-- 1. Workcenter projection (read model)
-- ============================================================================

CREATE TABLE IF NOT EXISTS workcenter_projections (
    workcenter_id   UUID PRIMARY KEY,
    tenant_id       TEXT NOT NULL,
    code            TEXT NOT NULL,
    name            TEXT NOT NULL,
    is_active       BOOLEAN NOT NULL DEFAULT TRUE,
    last_event_id   UUID NOT NULL,
    projected_at    TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE INDEX idx_workcenter_proj_tenant
    ON workcenter_projections (tenant_id);

CREATE UNIQUE INDEX idx_workcenter_proj_tenant_code
    ON workcenter_projections (tenant_id, code);

-- ============================================================================
-- 2. Processed events dedup table
-- ============================================================================

CREATE TABLE IF NOT EXISTS maintenance_processed_events (
    event_id    UUID PRIMARY KEY,
    event_type  TEXT NOT NULL,
    processor   TEXT NOT NULL,
    created_at  TIMESTAMPTZ NOT NULL DEFAULT NOW()
);
