-- Downtime event records for asset tracking (bd-127te)
-- Immutable once created — no UPDATE path by design.

CREATE TABLE downtime_events (
    id                    UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    tenant_id             TEXT NOT NULL,
    asset_id              UUID NOT NULL REFERENCES maintainable_assets(id),
    start_time            TIMESTAMP WITH TIME ZONE NOT NULL,
    end_time              TIMESTAMP WITH TIME ZONE,
    reason                TEXT NOT NULL,
    impact_classification TEXT NOT NULL
                          CHECK (impact_classification IN ('none', 'minor', 'major', 'critical')),
    idempotency_key       TEXT,
    notes                 TEXT,
    created_at            TIMESTAMP WITH TIME ZONE NOT NULL DEFAULT NOW(),

    CONSTRAINT downtime_events_tenant_idempotency_unique
        UNIQUE (tenant_id, idempotency_key)
);

CREATE INDEX idx_downtime_events_tenant ON downtime_events(tenant_id);
CREATE INDEX idx_downtime_events_tenant_asset ON downtime_events(tenant_id, asset_id);
CREATE INDEX idx_downtime_events_tenant_start ON downtime_events(tenant_id, start_time DESC);
