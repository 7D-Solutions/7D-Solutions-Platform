-- Quality Inspection module: inspection plans, inspections, dispositions, outbox
--
-- inspection_plans: define what to inspect and acceptance criteria per part/operation.
-- inspections: individual inspection records with results.
-- dispositions: accept/reject/MRB decisions on inspected lots.

-- Inspection plans: define inspection criteria for a part or operation
CREATE TABLE inspection_plans (
    id          UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    tenant_id   TEXT NOT NULL,
    part_id     UUID NOT NULL,
    plan_name   TEXT NOT NULL,
    revision    TEXT NOT NULL DEFAULT 'A',
    status      TEXT NOT NULL DEFAULT 'draft'
                CHECK (status IN ('draft', 'active', 'superseded')),
    created_at  TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at  TIMESTAMPTZ NOT NULL DEFAULT NOW(),

    CONSTRAINT inspection_plans_tenant_part_rev_unique UNIQUE (tenant_id, part_id, revision)
);

CREATE INDEX idx_inspection_plans_tenant ON inspection_plans(tenant_id);
CREATE INDEX idx_inspection_plans_part ON inspection_plans(part_id);

-- Inspections: individual inspection records
CREATE TABLE inspections (
    id              UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    tenant_id       TEXT NOT NULL,
    plan_id         UUID REFERENCES inspection_plans(id),
    lot_id          UUID,
    inspector_id    UUID,
    inspection_type TEXT NOT NULL DEFAULT 'receiving'
                    CHECK (inspection_type IN ('receiving', 'in_process', 'final')),
    result          TEXT NOT NULL DEFAULT 'pending'
                    CHECK (result IN ('pending', 'pass', 'fail', 'conditional')),
    notes           TEXT,
    inspected_at    TIMESTAMPTZ,
    created_at      TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at      TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE INDEX idx_inspections_tenant ON inspections(tenant_id);
CREATE INDEX idx_inspections_plan ON inspections(plan_id);
CREATE INDEX idx_inspections_lot ON inspections(lot_id);
CREATE INDEX idx_inspections_result ON inspections(result);

-- Dispositions: accept/reject/MRB decisions on inspected lots
CREATE TABLE dispositions (
    id              UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    tenant_id       TEXT NOT NULL,
    inspection_id   UUID NOT NULL REFERENCES inspections(id),
    decision        TEXT NOT NULL CHECK (decision IN ('accept', 'reject', 'mrb', 'rework', 'return_to_vendor')),
    decided_by      UUID,
    reason          TEXT,
    decided_at      TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    created_at      TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE INDEX idx_dispositions_tenant ON dispositions(tenant_id);
CREATE INDEX idx_dispositions_inspection ON dispositions(inspection_id);

-- Outbox for quality inspection events
CREATE TABLE quality_inspection_outbox (
    id              BIGSERIAL PRIMARY KEY,
    event_id        UUID NOT NULL UNIQUE,
    event_type      TEXT NOT NULL,
    aggregate_type  TEXT NOT NULL,
    aggregate_id    TEXT NOT NULL,
    tenant_id       TEXT NOT NULL,
    payload         JSONB NOT NULL,
    correlation_id  TEXT,
    causation_id    TEXT,
    schema_version  TEXT NOT NULL DEFAULT '1.0.0',
    created_at      TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    published_at    TIMESTAMPTZ
);

CREATE INDEX idx_qi_outbox_unpublished ON quality_inspection_outbox(created_at) WHERE published_at IS NULL;
CREATE INDEX idx_qi_outbox_tenant ON quality_inspection_outbox(tenant_id);

-- Processed events for idempotent consumers
CREATE TABLE quality_inspection_processed_events (
    id           BIGSERIAL PRIMARY KEY,
    event_id     UUID NOT NULL UNIQUE,
    event_type   TEXT NOT NULL,
    processor    TEXT NOT NULL,
    processed_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE INDEX idx_qi_processed_event_id ON quality_inspection_processed_events(event_id);
