-- Projection Cursor Tracking Schema
-- Tracks event stream position for each projection to enable:
-- 1. Idempotent event processing (no duplicate applies)
-- 2. Deterministic rebuild capability
-- 3. Monotonic cursor advancement
-- PostgreSQL with SQLx for Rust backend

-- ============================================================
-- PROJECTION CURSORS TABLE
-- ============================================================

CREATE TABLE projection_cursors (
    -- Projection identification
    projection_name VARCHAR(100) NOT NULL,
    tenant_id VARCHAR(100) NOT NULL,

    -- Event stream position
    last_event_id UUID NOT NULL,
    last_event_occurred_at TIMESTAMPTZ NOT NULL,

    -- Cursor metadata
    updated_at TIMESTAMPTZ NOT NULL DEFAULT CURRENT_TIMESTAMP,
    events_processed BIGINT NOT NULL DEFAULT 1,

    -- Composite primary key ensures one cursor per (projection, tenant)
    PRIMARY KEY (projection_name, tenant_id)
);

-- ============================================================
-- INDEXES FOR QUERY PERFORMANCE
-- ============================================================

-- Index for finding projections that are behind (for monitoring)
CREATE INDEX projection_cursors_updated_at ON projection_cursors(updated_at DESC);

-- Index for finding all projections for a specific tenant
CREATE INDEX projection_cursors_tenant_id ON projection_cursors(tenant_id);

-- ============================================================
-- COMMENTS
-- ============================================================

COMMENT ON TABLE projection_cursors IS 'Tracks event stream position for each projection to ensure idempotent processing and enable deterministic rebuilds';
COMMENT ON COLUMN projection_cursors.projection_name IS 'Name of the projection (e.g., "invoice_summary", "customer_balance")';
COMMENT ON COLUMN projection_cursors.tenant_id IS 'Tenant identifier for multi-tenant isolation';
COMMENT ON COLUMN projection_cursors.last_event_id IS 'UUID of the last successfully processed event';
COMMENT ON COLUMN projection_cursors.last_event_occurred_at IS 'Timestamp when the last processed event occurred';
COMMENT ON COLUMN projection_cursors.updated_at IS 'Timestamp when this cursor was last updated';
COMMENT ON COLUMN projection_cursors.events_processed IS 'Total number of events processed by this projection for this tenant';
