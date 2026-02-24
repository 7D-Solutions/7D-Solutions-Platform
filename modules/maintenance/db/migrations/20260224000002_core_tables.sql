-- Maintenance Core Tables
-- maintainable_assets, meter_types, maintenance_plans,
-- maintenance_plan_assignments, work_orders, wo_counters

-- ============================================================
-- MAINTAINABLE ASSETS
-- Lightweight asset register: tag, name, type, location, status.
-- Optional link to Fixed-Assets via fixed_asset_ref.
-- ============================================================

CREATE TABLE maintainable_assets (
    id                 UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    tenant_id          TEXT NOT NULL,
    asset_tag          TEXT NOT NULL,
    name               TEXT NOT NULL,
    description        TEXT,
    asset_type         TEXT NOT NULL
                       CHECK (asset_type IN ('vehicle', 'machinery', 'equipment', 'facility', 'other')),
    location           TEXT,
    department         TEXT,
    responsible_person TEXT,
    serial_number      TEXT,
    fixed_asset_ref    UUID,
    status             TEXT NOT NULL DEFAULT 'active'
                       CHECK (status IN ('active', 'inactive', 'retired')),
    metadata           JSONB,
    created_at         TIMESTAMP WITH TIME ZONE NOT NULL DEFAULT NOW(),
    updated_at         TIMESTAMP WITH TIME ZONE NOT NULL DEFAULT NOW(),

    CONSTRAINT maintainable_assets_tenant_tag_unique UNIQUE (tenant_id, asset_tag)
);

CREATE INDEX idx_maintainable_assets_tenant ON maintainable_assets(tenant_id);
CREATE INDEX idx_maintainable_assets_tenant_type ON maintainable_assets(tenant_id, asset_type);
CREATE INDEX idx_maintainable_assets_tenant_status ON maintainable_assets(tenant_id, status);

-- ============================================================
-- METER TYPES
-- Per-tenant meter definitions (odometer, engine hours, etc.)
-- rollover_value nullable: when set, enables rollover detection.
-- ============================================================

CREATE TABLE meter_types (
    id             UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    tenant_id      TEXT NOT NULL,
    name           TEXT NOT NULL,
    unit_label     TEXT NOT NULL,
    rollover_value BIGINT,
    created_at     TIMESTAMP WITH TIME ZONE NOT NULL DEFAULT NOW(),
    updated_at     TIMESTAMP WITH TIME ZONE NOT NULL DEFAULT NOW(),

    CONSTRAINT meter_types_tenant_name_unique UNIQUE (tenant_id, name)
);

CREATE INDEX idx_meter_types_tenant ON meter_types(tenant_id);

-- ============================================================
-- MAINTENANCE PLANS
-- Recurring schedule templates: calendar, meter, or both.
-- meter_type_id references meter_types for meter-based plans.
-- ============================================================

CREATE TABLE maintenance_plans (
    id                         UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    tenant_id                  TEXT NOT NULL,
    name                       TEXT NOT NULL,
    description                TEXT,
    asset_type_filter          TEXT
                               CHECK (asset_type_filter IS NULL
                                      OR asset_type_filter IN ('vehicle', 'machinery', 'equipment', 'facility', 'other')),
    schedule_type              TEXT NOT NULL
                               CHECK (schedule_type IN ('calendar', 'meter', 'both')),
    calendar_interval_days     INTEGER,
    meter_type_id              UUID REFERENCES meter_types(id),
    meter_interval             BIGINT,
    priority                   TEXT NOT NULL DEFAULT 'medium'
                               CHECK (priority IN ('low', 'medium', 'high', 'critical')),
    estimated_duration_minutes INTEGER,
    estimated_cost_minor       BIGINT,
    task_checklist             JSONB,
    is_active                  BOOLEAN NOT NULL DEFAULT TRUE,
    created_at                 TIMESTAMP WITH TIME ZONE NOT NULL DEFAULT NOW(),
    updated_at                 TIMESTAMP WITH TIME ZONE NOT NULL DEFAULT NOW()
);

CREATE INDEX idx_maintenance_plans_tenant ON maintenance_plans(tenant_id);
CREATE INDEX idx_maintenance_plans_tenant_active ON maintenance_plans(tenant_id, is_active);

-- ============================================================
-- MAINTENANCE PLAN ASSIGNMENTS
-- Links plans to specific assets with due tracking.
-- Scheduler queries by next_due_date and next_due_meter.
-- ============================================================

CREATE TABLE maintenance_plan_assignments (
    id                 UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    tenant_id          TEXT NOT NULL,
    plan_id            UUID NOT NULL REFERENCES maintenance_plans(id),
    asset_id           UUID NOT NULL REFERENCES maintainable_assets(id),
    last_completed_at  TIMESTAMP WITH TIME ZONE,
    last_meter_reading BIGINT,
    next_due_date      DATE,
    next_due_meter     BIGINT,
    state              TEXT NOT NULL DEFAULT 'active'
                       CHECK (state IN ('active', 'paused', 'completed')),
    created_at         TIMESTAMP WITH TIME ZONE NOT NULL DEFAULT NOW(),
    updated_at         TIMESTAMP WITH TIME ZONE NOT NULL DEFAULT NOW(),

    CONSTRAINT plan_assignments_plan_asset_unique UNIQUE (plan_id, asset_id)
);

CREATE INDEX idx_plan_assignments_tenant ON maintenance_plan_assignments(tenant_id);
CREATE INDEX idx_plan_assignments_tenant_due_date ON maintenance_plan_assignments(tenant_id, next_due_date);
CREATE INDEX idx_plan_assignments_tenant_due_meter ON maintenance_plan_assignments(tenant_id, next_due_meter);

-- ============================================================
-- WORK ORDERS
-- Individual maintenance tasks with full lifecycle.
-- Status transitions enforced by domain state machine (not DB).
-- ============================================================

CREATE TABLE work_orders (
    id                  UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    tenant_id           TEXT NOT NULL,
    asset_id            UUID NOT NULL REFERENCES maintainable_assets(id),
    plan_assignment_id  UUID REFERENCES maintenance_plan_assignments(id),
    wo_number           TEXT NOT NULL,
    title               TEXT NOT NULL,
    description         TEXT,
    wo_type             TEXT NOT NULL
                        CHECK (wo_type IN ('preventive', 'corrective', 'inspection')),
    priority            TEXT NOT NULL DEFAULT 'medium'
                        CHECK (priority IN ('low', 'medium', 'high', 'critical')),
    status              TEXT NOT NULL DEFAULT 'draft'
                        CHECK (status IN (
                            'draft', 'awaiting_approval', 'scheduled',
                            'in_progress', 'on_hold', 'completed',
                            'closed', 'cancelled'
                        )),
    assigned_to         TEXT,
    scheduled_date      DATE,
    started_at          TIMESTAMP WITH TIME ZONE,
    completed_at        TIMESTAMP WITH TIME ZONE,
    closed_at           TIMESTAMP WITH TIME ZONE,
    checklist           JSONB,
    downtime_minutes    INTEGER,
    notes               TEXT,
    created_at          TIMESTAMP WITH TIME ZONE NOT NULL DEFAULT NOW(),
    updated_at          TIMESTAMP WITH TIME ZONE NOT NULL DEFAULT NOW(),

    CONSTRAINT work_orders_tenant_wo_number_unique UNIQUE (tenant_id, wo_number)
);

CREATE INDEX idx_work_orders_tenant_status ON work_orders(tenant_id, status);
CREATE INDEX idx_work_orders_tenant_asset ON work_orders(tenant_id, asset_id);

-- ============================================================
-- WO COUNTERS
-- Per-tenant sequential WO number generator.
-- SELECT FOR UPDATE on this row to safely allocate wo_number.
-- ============================================================

CREATE TABLE wo_counters (
    tenant_id   TEXT PRIMARY KEY,
    next_number BIGINT NOT NULL DEFAULT 1
);
