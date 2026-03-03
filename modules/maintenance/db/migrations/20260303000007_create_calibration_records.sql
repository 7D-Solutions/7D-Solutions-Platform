-- Calibration Records
-- Tracks calibration schedules, completions, and compliance for maintainable assets.
-- Calibration records are immutable once completed (aerospace audit requirement).

CREATE TABLE calibration_records (
    id               UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    tenant_id        TEXT NOT NULL,
    asset_id         UUID NOT NULL REFERENCES maintainable_assets(id),
    calibration_type TEXT NOT NULL,
    due_date         DATE NOT NULL,
    completed_date   TIMESTAMP WITH TIME ZONE,
    certificate_ref  TEXT,
    status           TEXT NOT NULL DEFAULT 'scheduled'
                     CHECK (status IN ('scheduled', 'completed')),
    idempotency_key  TEXT NOT NULL,
    created_at       TIMESTAMP WITH TIME ZONE NOT NULL DEFAULT NOW(),
    updated_at       TIMESTAMP WITH TIME ZONE NOT NULL DEFAULT NOW(),

    CONSTRAINT calibration_records_tenant_idemp_unique UNIQUE (tenant_id, idempotency_key)
);

CREATE INDEX idx_calibration_records_tenant ON calibration_records(tenant_id);
CREATE INDEX idx_calibration_records_tenant_asset ON calibration_records(tenant_id, asset_id);
CREATE INDEX idx_calibration_records_tenant_status ON calibration_records(tenant_id, status);
CREATE INDEX idx_calibration_records_overdue ON calibration_records(tenant_id, due_date)
    WHERE status = 'scheduled';
