-- Phase E: Workcenter downtime signals (bd-1kw8s)
-- Tracks active/completed downtime on production workcenters.
-- Production emits started/ended events; Maintenance consumes them.

CREATE TABLE IF NOT EXISTS workcenter_downtime (
    downtime_id     UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    tenant_id       TEXT NOT NULL,
    workcenter_id   UUID NOT NULL REFERENCES workcenters(workcenter_id),
    reason          TEXT NOT NULL,
    reason_code     TEXT,
    started_at      TIMESTAMPTZ NOT NULL DEFAULT now(),
    ended_at        TIMESTAMPTZ,
    started_by      TEXT,
    ended_by        TEXT,
    notes           TEXT,
    created_at      TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at      TIMESTAMPTZ NOT NULL DEFAULT now(),

    CONSTRAINT downtime_end_after_start
        CHECK (ended_at IS NULL OR ended_at > started_at)
);

CREATE INDEX idx_workcenter_downtime_tenant
    ON workcenter_downtime (tenant_id);

CREATE INDEX idx_workcenter_downtime_wc_active
    ON workcenter_downtime (tenant_id, workcenter_id)
    WHERE ended_at IS NULL;
