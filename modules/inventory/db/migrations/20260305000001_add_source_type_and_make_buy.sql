-- Phase A: Add source_type to receipt ledger records and make_buy to items
--
-- source_type: Discriminates the origin of a receipt (purchase, production, return).
--   Existing ledger rows default to 'purchase' (backward-compatible).
--   Production receipts carry caller-provided unit_cost (no backflush).
--
-- make_buy: Classifies items as MAKE (manufactured) or BUY (purchased).
--   Nullable to preserve backward compatibility with existing items.
--   Tenant-scoped via items.tenant_id.

-- ─── inventory_ledger: source_type ──────────────────────────────────────────

ALTER TABLE inventory_ledger
    ADD COLUMN source_type TEXT NOT NULL DEFAULT 'purchase';

COMMENT ON COLUMN inventory_ledger.source_type IS
    'Origin of the stock movement: purchase | production | return';

CREATE INDEX idx_ledger_source_type
    ON inventory_ledger(tenant_id, source_type)
    WHERE entry_type = 'received';

-- ─── items: make_buy ────────────────────────────────────────────────────────

ALTER TABLE items
    ADD COLUMN make_buy TEXT;

COMMENT ON COLUMN items.make_buy IS
    'Manufacturing classification: make | buy (NULL = unset)';

CREATE INDEX idx_items_make_buy
    ON items(tenant_id, make_buy)
    WHERE make_buy IS NOT NULL;
