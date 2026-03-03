-- Export runs and events outbox for reporting data exports
-- bd-3r96e: CSV/Excel/PDF export support

-- ============================================================
-- EXPORT RUNS
-- ============================================================
-- Tracks each export request and its completion status.
-- Grain: (tenant_id, id). Idempotency via idempotency_key.

CREATE TABLE rpt_export_runs (
    id              UUID        PRIMARY KEY DEFAULT gen_random_uuid(),
    tenant_id       TEXT        NOT NULL,
    report_id       TEXT        NOT NULL,
    format          TEXT        NOT NULL CHECK (format IN ('csv', 'xlsx', 'pdf')),
    status          TEXT        NOT NULL DEFAULT 'pending' CHECK (status IN ('pending', 'running', 'completed', 'failed')),
    output_ref      TEXT,
    row_count       INT,
    idempotency_key TEXT,
    error_message   TEXT,
    created_at      TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    completed_at    TIMESTAMPTZ,

    CONSTRAINT rpt_export_runs_idempotency_uq
        UNIQUE (tenant_id, idempotency_key)
);

CREATE INDEX idx_rpt_export_runs_tenant ON rpt_export_runs (tenant_id);
CREATE INDEX idx_rpt_export_runs_tenant_report ON rpt_export_runs (tenant_id, report_id);

COMMENT ON TABLE rpt_export_runs IS
    'Export run tracking for CSV/Excel/PDF report exports. Grain: (tenant_id, id).';

-- ============================================================
-- EVENTS OUTBOX
-- ============================================================
-- Transactional outbox for reliable event publishing from reporting module.

CREATE TABLE IF NOT EXISTS events_outbox (
    id              BIGSERIAL   PRIMARY KEY,
    event_id        UUID        NOT NULL UNIQUE,
    event_type      TEXT        NOT NULL,
    aggregate_type  TEXT        NOT NULL,
    aggregate_id    TEXT        NOT NULL,
    payload         JSONB       NOT NULL,
    tenant_id       TEXT        NOT NULL,
    created_at      TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    published_at    TIMESTAMPTZ,

    -- Envelope metadata
    source_module   TEXT,
    source_version  TEXT,
    schema_version  TEXT,
    occurred_at     TIMESTAMPTZ,
    replay_safe     BOOLEAN,
    trace_id        TEXT,
    correlation_id  TEXT,
    causation_id    TEXT,
    reverses_event_id UUID,
    supersedes_event_id UUID,
    side_effect_id  TEXT,
    mutation_class  TEXT
);

CREATE INDEX IF NOT EXISTS idx_events_outbox_unpublished
    ON events_outbox (created_at) WHERE published_at IS NULL;

CREATE INDEX IF NOT EXISTS idx_events_outbox_tenant
    ON events_outbox (tenant_id);
