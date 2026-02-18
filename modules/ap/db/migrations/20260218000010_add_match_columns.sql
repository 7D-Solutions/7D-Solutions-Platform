-- Add variance tracking columns and idempotency constraint to three_way_match
--
-- bd-sdh9: 3-way match engine (PO / receipt links / bill) with deterministic rules
--
-- Changes:
--   1. Add price_variance_minor BIGINT — (bill_price - po_price) × matched_qty
--   2. Add qty_variance DOUBLE PRECISION — bill_qty - (received_qty or po_qty)
--   3. Add match_status TEXT — matched | price_variance | qty_variance | price_and_qty_variance
--   4. Add UNIQUE INDEX on bill_line_id for idempotent re-runs (ON CONFLICT DO NOTHING)

ALTER TABLE three_way_match
    ADD COLUMN IF NOT EXISTS price_variance_minor BIGINT NOT NULL DEFAULT 0,
    ADD COLUMN IF NOT EXISTS qty_variance DOUBLE PRECISION NOT NULL DEFAULT 0.0,
    ADD COLUMN IF NOT EXISTS match_status TEXT NOT NULL DEFAULT 'matched'
        CHECK (match_status IN ('matched', 'price_variance', 'qty_variance', 'price_and_qty_variance'));

-- Idempotency: one match record per bill line — re-running the engine does not duplicate rows
CREATE UNIQUE INDEX IF NOT EXISTS uq_three_way_match_bill_line
    ON three_way_match (bill_line_id);
