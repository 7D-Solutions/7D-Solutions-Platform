-- Projection cursor tracking for payments module (bd-v7g8u).
--
-- payments.get_payment uses ProjectionCursor::load() to detect stale projections
-- and activate the HTTP fallback path. The projection_cursors table must exist in
-- the payments database for this to work.
--
-- DDL matches platform/projections/db/migrations/20260216000001_create_projection_cursors.sql
-- exactly. The canonical schema lives there; this migration keeps the payments DB in sync.

CREATE TABLE IF NOT EXISTS projection_cursors (
    projection_name        VARCHAR(100) NOT NULL,
    tenant_id              VARCHAR(100) NOT NULL,
    last_event_id          UUID         NOT NULL,
    last_event_occurred_at TIMESTAMPTZ  NOT NULL,
    updated_at             TIMESTAMPTZ  NOT NULL DEFAULT CURRENT_TIMESTAMP,
    events_processed       BIGINT       NOT NULL DEFAULT 1,
    PRIMARY KEY (projection_name, tenant_id)
);

CREATE INDEX IF NOT EXISTS projection_cursors_updated_at ON projection_cursors(updated_at DESC);
CREATE INDEX IF NOT EXISTS projection_cursors_tenant_id  ON projection_cursors(tenant_id);

COMMENT ON TABLE projection_cursors IS
    'Tracks event stream position for each projection to ensure idempotent processing and enable deterministic rebuilds';
