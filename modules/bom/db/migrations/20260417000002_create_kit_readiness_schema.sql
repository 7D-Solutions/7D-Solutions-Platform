-- Kit Readiness: snapshot audit trail + test-scaffold on-hand table.
--
-- kit_readiness_snapshots: one row per point-in-time availability check.
-- kit_readiness_lines: one row per BOM explosion line in that check.
--
-- item_on_hand: DIRECT-MODE TEST SCAFFOLD ONLY.
--   In production the BOM service calls the Inventory HTTP API; this table
--   is never written to by the BOM service itself.  Integration tests seed it
--   directly to exercise kit-readiness logic without a live Inventory service.

CREATE TABLE kit_readiness_snapshots (
    id                UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    tenant_id         TEXT NOT NULL,
    bom_id            UUID NOT NULL,
    required_quantity DOUBLE PRECISION NOT NULL,
    check_date        TIMESTAMPTZ NOT NULL,
    overall_status    TEXT NOT NULL CHECK (overall_status IN ('ready', 'partial', 'not_ready')),
    issue_summary     JSONB NOT NULL DEFAULT '[]',
    created_by        TEXT NOT NULL,
    created_at        TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE INDEX idx_kit_readiness_tenant_bom ON kit_readiness_snapshots(tenant_id, bom_id);

CREATE TABLE kit_readiness_lines (
    id                BIGSERIAL PRIMARY KEY,
    snapshot_id       UUID NOT NULL REFERENCES kit_readiness_snapshots(id) ON DELETE CASCADE,
    component_item_id UUID NOT NULL,
    required_qty      DOUBLE PRECISION NOT NULL,
    on_hand_qty       BIGINT NOT NULL DEFAULT 0,
    expired_qty       BIGINT NOT NULL DEFAULT 0,
    available_qty     BIGINT NOT NULL DEFAULT 0,
    status            TEXT NOT NULL CHECK (status IN ('ready', 'short', 'expired', 'quarantined'))
);

CREATE INDEX idx_kit_readiness_lines_snapshot ON kit_readiness_lines(snapshot_id);

-- Test scaffold: minimal on-hand projection for InventoryClient Direct mode queries.
-- Columns mirror Inventory's item_on_hand table at the fields kit-readiness needs.
-- In production this table exists but stays empty; the Platform/Http mode reads
-- live data from the Inventory service instead.
CREATE TABLE IF NOT EXISTS item_on_hand (
    id              UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    tenant_id       TEXT NOT NULL,
    item_id         UUID NOT NULL,
    on_hand_qty     BIGINT NOT NULL DEFAULT 0 CHECK (on_hand_qty >= 0),
    expired_qty     BIGINT NOT NULL DEFAULT 0 CHECK (expired_qty >= 0),
    quarantine_qty  BIGINT NOT NULL DEFAULT 0 CHECK (quarantine_qty >= 0),
    available_qty   BIGINT GENERATED ALWAYS AS (
        GREATEST(0, on_hand_qty - expired_qty - quarantine_qty)
    ) STORED,
    CONSTRAINT item_on_hand_bom_unique UNIQUE (tenant_id, item_id)
);
