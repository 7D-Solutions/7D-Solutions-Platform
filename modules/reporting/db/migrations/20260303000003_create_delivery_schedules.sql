-- Delivery schedules for scheduled report generation and delivery
-- bd-2nddc: Phase 64 reporting scheduled delivery

-- ============================================================
-- DELIVERY SCHEDULES
-- ============================================================
-- Tracks user-defined schedules that trigger report generation
-- and delivery via notifications. Grain: (tenant_id, id).
-- Idempotency via idempotency_key.

CREATE TABLE rpt_delivery_schedules (
    id              UUID        PRIMARY KEY DEFAULT gen_random_uuid(),
    tenant_id       TEXT        NOT NULL,
    report_id       TEXT        NOT NULL,
    schedule_name   TEXT        NOT NULL,
    cron_expr       TEXT,
    interval_secs   INT,
    delivery_channel TEXT       NOT NULL CHECK (delivery_channel IN ('email', 'webhook', 'sftp')),
    recipient       TEXT        NOT NULL,
    format          TEXT        NOT NULL CHECK (format IN ('csv', 'xlsx', 'pdf')),
    status          TEXT        NOT NULL DEFAULT 'active' CHECK (status IN ('active', 'paused', 'disabled')),
    idempotency_key TEXT,
    last_triggered_at TIMESTAMPTZ,
    next_trigger_at   TIMESTAMPTZ,
    created_at      TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at      TIMESTAMPTZ NOT NULL DEFAULT NOW(),

    -- Must have either cron_expr or interval_secs
    CONSTRAINT rpt_delivery_schedules_trigger_check
        CHECK (cron_expr IS NOT NULL OR interval_secs IS NOT NULL),

    CONSTRAINT rpt_delivery_schedules_idempotency_uq
        UNIQUE (tenant_id, idempotency_key)
);

CREATE INDEX idx_rpt_delivery_schedules_tenant ON rpt_delivery_schedules (tenant_id);
CREATE INDEX idx_rpt_delivery_schedules_tenant_report ON rpt_delivery_schedules (tenant_id, report_id);
CREATE INDEX idx_rpt_delivery_schedules_active_next ON rpt_delivery_schedules (next_trigger_at)
    WHERE status = 'active';

COMMENT ON TABLE rpt_delivery_schedules IS
    'User-defined delivery schedules for recurring report generation. Grain: (tenant_id, id).';

-- ============================================================
-- SCHEDULE EXECUTION LOG
-- ============================================================
-- Audit trail for every schedule trigger execution.

CREATE TABLE rpt_schedule_executions (
    id              UUID        PRIMARY KEY DEFAULT gen_random_uuid(),
    schedule_id     UUID        NOT NULL REFERENCES rpt_delivery_schedules(id),
    tenant_id       TEXT        NOT NULL,
    export_run_id   UUID,
    status          TEXT        NOT NULL DEFAULT 'pending' CHECK (status IN ('pending', 'running', 'completed', 'failed')),
    error_message   TEXT,
    triggered_at    TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    completed_at    TIMESTAMPTZ
);

CREATE INDEX idx_rpt_schedule_executions_schedule ON rpt_schedule_executions (schedule_id);
CREATE INDEX idx_rpt_schedule_executions_tenant ON rpt_schedule_executions (tenant_id);

COMMENT ON TABLE rpt_schedule_executions IS
    'Audit trail for schedule trigger executions. Links schedule to export run.';
