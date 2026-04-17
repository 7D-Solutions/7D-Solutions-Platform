-- MRP (Net Requirements) tables
--
-- mrp_snapshots: immutable audit record of one explode() call.
-- mrp_requirement_lines: per-component net requirements for that snapshot.

CREATE TABLE mrp_snapshots (
    id               UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    tenant_id        TEXT NOT NULL,
    bom_id           UUID NOT NULL REFERENCES bom_headers(id),
    demand_quantity  FLOAT8 NOT NULL CHECK (demand_quantity > 0),
    effectivity_date TIMESTAMPTZ NOT NULL,
    on_hand_snapshot JSONB NOT NULL,
    created_at       TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    created_by       TEXT NOT NULL
);

CREATE INDEX idx_mrp_snapshots_tenant  ON mrp_snapshots(tenant_id);
CREATE INDEX idx_mrp_snapshots_bom     ON mrp_snapshots(bom_id);
CREATE INDEX idx_mrp_snapshots_created ON mrp_snapshots(created_at);

CREATE TABLE mrp_requirement_lines (
    id                      BIGSERIAL PRIMARY KEY,
    snapshot_id             UUID NOT NULL REFERENCES mrp_snapshots(id) ON DELETE CASCADE,
    level                   INT NOT NULL,
    parent_part_id          UUID NOT NULL,
    component_item_id       UUID NOT NULL,
    gross_quantity          FLOAT8 NOT NULL,
    scrap_factor            FLOAT8 NOT NULL,
    scrap_adjusted_quantity FLOAT8 NOT NULL,
    on_hand_quantity        FLOAT8 NOT NULL,
    net_quantity            FLOAT8 NOT NULL,
    uom                     TEXT,
    revision_id             UUID NOT NULL,
    revision_label          TEXT NOT NULL
);

CREATE INDEX idx_mrp_req_lines_snapshot  ON mrp_requirement_lines(snapshot_id);
CREATE INDEX idx_mrp_req_lines_component ON mrp_requirement_lines(component_item_id);
