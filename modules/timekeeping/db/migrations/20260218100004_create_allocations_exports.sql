-- Timekeeping: Project Allocations and Export Runs
--
-- Allocations track planned hours per employee per project (resource planning).
-- Export runs record payroll / billing data extractions for downstream systems.

-- ============================================================
-- ALLOCATIONS (planned hours per employee per project)
-- ============================================================

CREATE TABLE tk_allocations (
    id                       UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    app_id                   VARCHAR(50) NOT NULL,
    employee_id              UUID NOT NULL REFERENCES tk_employees(id) ON DELETE RESTRICT,
    project_id               UUID NOT NULL REFERENCES tk_projects(id) ON DELETE RESTRICT,
    task_id                  UUID REFERENCES tk_tasks(id) ON DELETE RESTRICT,
    -- Planned minutes per week (integer, matches entry convention)
    allocated_minutes_per_week INT NOT NULL CHECK (allocated_minutes_per_week > 0),
    effective_from           DATE NOT NULL,
    effective_to             DATE,           -- NULL = ongoing
    active                   BOOLEAN NOT NULL DEFAULT TRUE,
    created_at               TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at               TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE INDEX tk_allocations_app_id
    ON tk_allocations(app_id);
CREATE INDEX tk_allocations_employee
    ON tk_allocations(app_id, employee_id);
CREATE INDEX tk_allocations_project
    ON tk_allocations(app_id, project_id);
CREATE INDEX tk_allocations_effective
    ON tk_allocations(app_id, effective_from, effective_to)
    WHERE active = TRUE;

-- ============================================================
-- EXPORT RUNS (payroll / billing data exports)
-- ============================================================

CREATE TYPE tk_export_status AS ENUM (
    'pending',
    'in_progress',
    'completed',
    'failed'
);

CREATE TABLE tk_export_runs (
    id            UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    app_id        VARCHAR(50) NOT NULL,
    export_type   VARCHAR(50) NOT NULL,     -- 'payroll', 'billing', 'report'
    period_start  DATE NOT NULL,
    period_end    DATE NOT NULL,
    status        tk_export_status NOT NULL DEFAULT 'pending',
    record_count  INT,
    -- Export metadata: filters applied, target system, etc.
    metadata      JSONB,
    error_message TEXT,
    started_at    TIMESTAMPTZ,
    completed_at  TIMESTAMPTZ,
    created_at    TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE INDEX tk_export_runs_app_id
    ON tk_export_runs(app_id);
CREATE INDEX tk_export_runs_app_type
    ON tk_export_runs(app_id, export_type);
CREATE INDEX tk_export_runs_app_period
    ON tk_export_runs(app_id, period_start, period_end);
CREATE INDEX tk_export_runs_status
    ON tk_export_runs(status);
