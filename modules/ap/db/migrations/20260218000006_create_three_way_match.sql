-- AP Three-Way Match Table
--
-- Records the matching decision linking a vendor bill line to a PO line
-- (and optionally a goods receipt for 3-way match).
--
-- Match types:
--   two_way   — PO ↔ Bill (no receipt required)
--   three_way — PO ↔ Receipt ↔ Bill (full verification)
--   non_po    — Bill only (no PO backing, spot purchase)
--
-- Indexed for:
--   - Match lookup by (po_id, line_id, inventory_ref/receipt_id)
--   - Bill-scoped match history

CREATE TABLE three_way_match (
    id                      BIGSERIAL PRIMARY KEY,
    bill_id                 UUID NOT NULL REFERENCES vendor_bills (bill_id),
    bill_line_id            UUID NOT NULL REFERENCES bill_lines (line_id),
    po_id                   UUID REFERENCES purchase_orders (po_id),
    po_line_id              UUID REFERENCES po_lines (line_id),
    -- The receipt/GRN link (required for three_way, NULL for two_way/non_po)
    receipt_id              UUID,
    -- "two_way", "three_way", or "non_po"
    match_type              TEXT NOT NULL
                                CHECK (match_type IN ('two_way', 'three_way', 'non_po')),
    matched_quantity        NUMERIC(18, 6) NOT NULL CHECK (matched_quantity > 0),
    -- Matched amount in minor currency units (i64)
    matched_amount_minor    BIGINT NOT NULL CHECK (matched_amount_minor >= 0),
    -- True if quantity and price tolerance checks passed
    within_tolerance        BOOLEAN NOT NULL,
    matched_by              TEXT NOT NULL,
    matched_at              TIMESTAMP WITH TIME ZONE NOT NULL DEFAULT NOW()
);

-- 3-way match lookup: given (po_id, line_id, inventory_ref) find matches
-- This is the primary index cited in acceptance criteria
CREATE INDEX idx_three_way_match_po_line_receipt
    ON three_way_match (po_id, po_line_id, receipt_id);

-- Bill-scoped match history
CREATE INDEX idx_three_way_match_bill
    ON three_way_match (bill_id);

-- Open tolerance failures (within_tolerance = false needs review)
CREATE INDEX idx_three_way_match_tolerance
    ON three_way_match (bill_id, within_tolerance)
    WHERE within_tolerance = FALSE;
