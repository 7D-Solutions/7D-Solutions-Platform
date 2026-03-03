-- Inventory: Lot Genealogy (parent-child transformation edges)
--
-- Records immutable directed edges between lots representing material
-- transformations: split (one parent → many children), merge (many parents →
-- one child), and extensible types (repackage, consume, etc.).
--
-- Invariants:
-- - Edges are immutable once created (no UPDATE/DELETE in application layer).
-- - No cross-tenant edges (parent and child must share tenant_id).
-- - No self-referencing edges (parent_lot_id != child_lot_id).
-- - Idempotent at the HTTP level via inv_idempotency_keys table.
-- - Per-edge dedup: (tenant_id, operation_id, parent_lot_id, child_lot_id).
-- - operation_id groups edges that belong to the same logical transformation.

CREATE TABLE inv_lot_genealogy (
    id               UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    tenant_id        TEXT NOT NULL,
    operation_id     UUID NOT NULL,
    parent_lot_id    UUID NOT NULL REFERENCES inventory_lots(id),
    child_lot_id     UUID NOT NULL REFERENCES inventory_lots(id),
    transformation   TEXT NOT NULL CHECK (transformation IN ('split', 'merge', 'repackage', 'consume')),
    -- Quantity moved from parent to child in this edge
    quantity         BIGINT NOT NULL CHECK (quantity > 0),
    -- Unit of measure label (informational, matches item's base_uom)
    unit             TEXT NOT NULL DEFAULT 'ea',
    occurred_at      TIMESTAMP WITH TIME ZONE NOT NULL DEFAULT NOW(),
    actor_id         UUID,
    notes            TEXT,
    created_at       TIMESTAMP WITH TIME ZONE NOT NULL DEFAULT NOW(),

    -- No self-referencing edges
    CONSTRAINT inv_genealogy_no_self_ref CHECK (parent_lot_id != child_lot_id),
    -- No duplicate edges within the same operation
    CONSTRAINT inv_genealogy_edge_unique UNIQUE (tenant_id, operation_id, parent_lot_id, child_lot_id)
);

-- Query patterns:
-- 1. "Show me all children of lot X" (forward trace)
CREATE INDEX idx_genealogy_parent ON inv_lot_genealogy(tenant_id, parent_lot_id);
-- 2. "Show me all parents of lot Y" (reverse trace)
CREATE INDEX idx_genealogy_child  ON inv_lot_genealogy(tenant_id, child_lot_id);
-- 3. "Show me all edges for operation Z" (group lookup)
CREATE INDEX idx_genealogy_operation ON inv_lot_genealogy(tenant_id, operation_id);
