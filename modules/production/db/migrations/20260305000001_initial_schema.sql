-- Production module: initial schema
-- Base tables for work orders, workcenters, routing templates, operations, and outbox.
-- No business logic endpoints yet — this is scaffold only.

CREATE TABLE IF NOT EXISTS workcenters (
    workcenter_id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    tenant_id     TEXT NOT NULL,
    code          TEXT NOT NULL,
    name          TEXT NOT NULL,
    description   TEXT,
    capacity      INTEGER,
    cost_rate_minor BIGINT DEFAULT 0,
    is_active     BOOLEAN NOT NULL DEFAULT TRUE,
    created_at    TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at    TIMESTAMPTZ NOT NULL DEFAULT now(),
    UNIQUE (tenant_id, code)
);

CREATE TABLE IF NOT EXISTS routing_templates (
    routing_template_id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    tenant_id           TEXT NOT NULL,
    name                TEXT NOT NULL,
    description         TEXT,
    item_id             UUID,
    bom_revision_id     UUID,
    is_active           BOOLEAN NOT NULL DEFAULT TRUE,
    created_at          TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at          TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE TABLE IF NOT EXISTS routing_steps (
    routing_step_id     UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    routing_template_id UUID NOT NULL REFERENCES routing_templates(routing_template_id),
    sequence_number     INTEGER NOT NULL,
    workcenter_id       UUID NOT NULL REFERENCES workcenters(workcenter_id),
    operation_name      TEXT NOT NULL,
    description         TEXT,
    setup_time_minutes  INTEGER DEFAULT 0,
    run_time_minutes    INTEGER DEFAULT 0,
    created_at          TIMESTAMPTZ NOT NULL DEFAULT now(),
    UNIQUE (routing_template_id, sequence_number)
);

CREATE TABLE IF NOT EXISTS work_orders (
    work_order_id   UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    tenant_id       TEXT NOT NULL,
    order_number    TEXT NOT NULL,
    status          TEXT NOT NULL DEFAULT 'draft',
    item_id         UUID NOT NULL,
    bom_revision_id UUID NOT NULL,
    routing_template_id UUID,
    planned_quantity INTEGER NOT NULL,
    completed_quantity INTEGER NOT NULL DEFAULT 0,
    planned_start   TIMESTAMPTZ,
    planned_end     TIMESTAMPTZ,
    actual_start    TIMESTAMPTZ,
    actual_end      TIMESTAMPTZ,
    material_cost_minor  BIGINT NOT NULL DEFAULT 0,
    labor_cost_minor     BIGINT NOT NULL DEFAULT 0,
    overhead_cost_minor  BIGINT NOT NULL DEFAULT 0,
    created_at      TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at      TIMESTAMPTZ NOT NULL DEFAULT now(),
    UNIQUE (tenant_id, order_number)
);

CREATE TABLE IF NOT EXISTS operations (
    operation_id    UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    work_order_id   UUID NOT NULL REFERENCES work_orders(work_order_id),
    sequence_number INTEGER NOT NULL,
    workcenter_id   UUID NOT NULL REFERENCES workcenters(workcenter_id),
    operation_name  TEXT NOT NULL,
    status          TEXT NOT NULL DEFAULT 'pending',
    started_at      TIMESTAMPTZ,
    completed_at    TIMESTAMPTZ,
    labor_minutes   INTEGER DEFAULT 0,
    created_at      TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at      TIMESTAMPTZ NOT NULL DEFAULT now(),
    UNIQUE (work_order_id, sequence_number)
);

CREATE TABLE IF NOT EXISTS production_outbox (
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
    published       BOOLEAN NOT NULL DEFAULT FALSE,
    created_at      TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE INDEX IF NOT EXISTS idx_production_outbox_unpublished
    ON production_outbox (created_at) WHERE published = FALSE;

CREATE INDEX IF NOT EXISTS idx_work_orders_tenant_status
    ON work_orders (tenant_id, status);

CREATE INDEX IF NOT EXISTS idx_operations_work_order
    ON operations (work_order_id);

CREATE INDEX IF NOT EXISTS idx_workcenters_tenant
    ON workcenters (tenant_id);
