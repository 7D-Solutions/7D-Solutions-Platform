-- AP PO Receipt Links
--
-- Records the link between a PO line and a goods-receipt/GRN from the
-- inventory or warehouse module. This table is the 3-way match anchor:
--   PO line → receipt → vendor bill.
--
-- Indexed for:
--   - 3-way match lookup: (po_id, line_id, inventory_ref / receipt_id)
--   - PO-scoped receipt history
--
-- inventory_ref is the external receipt/GRN identifier from the inventory
-- module (maps to the `receipt_id` field in ap.po_line_received_linked events).

CREATE TABLE po_receipt_links (
    id                  BIGSERIAL PRIMARY KEY,
    -- The purchase order being received against
    po_id               UUID NOT NULL REFERENCES purchase_orders (po_id),
    -- The specific line on the PO
    po_line_id          UUID NOT NULL REFERENCES po_lines (line_id),
    vendor_id           UUID NOT NULL REFERENCES vendors (vendor_id),
    -- External receipt/GRN identifier (from inventory module)
    receipt_id          UUID NOT NULL,
    -- Quantity received on this link
    quantity_received   NUMERIC(18, 6) NOT NULL CHECK (quantity_received > 0),
    unit_of_measure     TEXT NOT NULL,
    -- Unit price at PO creation time (minor currency units, i64)
    unit_price_minor    BIGINT NOT NULL CHECK (unit_price_minor >= 0),
    -- ISO 4217
    currency            CHAR(3) NOT NULL,
    gl_account_code     TEXT NOT NULL,
    received_at         TIMESTAMP WITH TIME ZONE NOT NULL,
    received_by         TEXT NOT NULL,

    -- Prevent duplicate receipt links for the same (po_line, receipt)
    CONSTRAINT uq_po_receipt_link UNIQUE (po_line_id, receipt_id)
);

-- 3-way match lookup: given (po_id, line_id, inventory_ref) find the receipt link
CREATE INDEX idx_po_receipt_match ON po_receipt_links (po_id, po_line_id, receipt_id);

-- All receipts for a PO
CREATE INDEX idx_po_receipt_po_id ON po_receipt_links (po_id);

-- All links for a specific receipt (to find which PO lines a receipt covers)
CREATE INDEX idx_po_receipt_receipt_id ON po_receipt_links (receipt_id);
