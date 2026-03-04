-- Phase 67: Calibration events, out-of-service tracking, downtime extensions (bd-20rpu)
--
-- 1. calibration_events: immutable record of calibration performed on an asset
-- 2. out_of_service flag on maintainable_assets
-- 3. downtime_events extensions: workcenter_id, reason_code, wo_ref

-- ============================================================================
-- 1. Calibration events table
-- ============================================================================

CREATE TABLE calibration_events (
    id              UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    tenant_id       TEXT NOT NULL,
    asset_id        UUID NOT NULL REFERENCES maintainable_assets(id),
    performed_at    TIMESTAMP WITH TIME ZONE NOT NULL,
    due_at          TIMESTAMP WITH TIME ZONE NOT NULL,
    result          TEXT NOT NULL CHECK (result IN ('pass', 'fail', 'conditional')),
    doc_revision_id UUID,
    idempotency_key TEXT,
    created_at      TIMESTAMP WITH TIME ZONE NOT NULL DEFAULT NOW(),

    CONSTRAINT calibration_events_tenant_idemp_unique
        UNIQUE (tenant_id, idempotency_key)
);

CREATE INDEX idx_calibration_events_tenant ON calibration_events(tenant_id);
CREATE INDEX idx_calibration_events_tenant_asset ON calibration_events(tenant_id, asset_id);
CREATE INDEX idx_calibration_events_latest ON calibration_events(tenant_id, asset_id, performed_at DESC);

-- ============================================================================
-- 2. Out-of-service tracking on assets
-- ============================================================================

ALTER TABLE maintainable_assets
    ADD COLUMN out_of_service BOOLEAN NOT NULL DEFAULT FALSE,
    ADD COLUMN out_of_service_reason TEXT;

-- ============================================================================
-- 3. Downtime event extensions
-- ============================================================================

ALTER TABLE downtime_events
    ALTER COLUMN asset_id DROP NOT NULL,
    ADD COLUMN workcenter_id UUID,
    ADD COLUMN reason_code TEXT,
    ADD COLUMN wo_ref TEXT;

-- At least one of asset_id or workcenter_id must be set
ALTER TABLE downtime_events
    ADD CONSTRAINT downtime_events_asset_or_workcenter
        CHECK (asset_id IS NOT NULL OR workcenter_id IS NOT NULL);
