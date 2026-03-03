-- Dashboard layout framework for configurable report dashboards
-- bd-aarxn: Phase 64 — Reporting dashboard layout framework

-- ============================================================
-- DASHBOARD LAYOUTS
-- ============================================================
-- Stores dashboard configurations composed of report widgets.
-- Grain: (tenant_id, id). Idempotency via idempotency_key.

CREATE TABLE rpt_dashboard_layouts (
    id              UUID        PRIMARY KEY DEFAULT gen_random_uuid(),
    tenant_id       TEXT        NOT NULL,
    name            TEXT        NOT NULL,
    description     TEXT,
    version         INT         NOT NULL DEFAULT 1,
    idempotency_key TEXT,
    created_at      TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at      TIMESTAMPTZ NOT NULL DEFAULT NOW(),

    CONSTRAINT rpt_dashboard_layouts_idempotency_uq
        UNIQUE (tenant_id, idempotency_key)
);

CREATE INDEX idx_rpt_dashboard_layouts_tenant ON rpt_dashboard_layouts (tenant_id);

COMMENT ON TABLE rpt_dashboard_layouts IS
    'Dashboard layout configurations for report widgets. Grain: (tenant_id, id).';

-- ============================================================
-- DASHBOARD WIDGETS
-- ============================================================
-- Individual widget slots within a dashboard layout.
-- Each widget references a report query and has positioning/display config.

CREATE TABLE rpt_dashboard_widgets (
    id              UUID        PRIMARY KEY DEFAULT gen_random_uuid(),
    layout_id       UUID        NOT NULL REFERENCES rpt_dashboard_layouts(id) ON DELETE CASCADE,
    tenant_id       TEXT        NOT NULL,
    widget_type     TEXT        NOT NULL,
    title           TEXT        NOT NULL,
    report_query    TEXT        NOT NULL,
    position_x      INT         NOT NULL DEFAULT 0,
    position_y      INT         NOT NULL DEFAULT 0,
    width           INT         NOT NULL DEFAULT 1,
    height          INT         NOT NULL DEFAULT 1,
    display_config  JSONB       NOT NULL DEFAULT '{}',
    created_at      TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at      TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE INDEX idx_rpt_dashboard_widgets_layout ON rpt_dashboard_widgets (layout_id);
CREATE INDEX idx_rpt_dashboard_widgets_tenant ON rpt_dashboard_widgets (tenant_id);

COMMENT ON TABLE rpt_dashboard_widgets IS
    'Widget slots within a dashboard layout. Each references a report query and has position/display config.';
